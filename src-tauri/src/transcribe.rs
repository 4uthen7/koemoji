use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use whisper_rs::{
    FullParams, SamplingStrategy, SegmentCallbackData, WhisperContext,
    WhisperContextParameters, WhisperState,
};

use crate::gpu;
use crate::model;
use crate::AppState;

/// Whisper が要求するサンプルレート
const SAMPLE_RATE: usize = 16_000;
/// 混在モードで言語を判定する窓の長さ(秒)。
/// 短いほど言語の切り替わりに追従できるが、判定回数が増える。
/// 判定は tiny モデルで行うため 10 秒でも十分高速。
const DETECT_WINDOW_SEC: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    #[serde(default = "default_segment_source")]
    pub source: String,
}

fn default_segment_source() -> String {
    "speech".into()
}

#[derive(Serialize, Clone)]
struct StatusPayload {
    stage: String,
}

/// 音声・動画ファイルを文字起こしする。
/// language に "mixed" を渡すと、10秒ごとに tiny モデルで言語を判定し、
/// 同一言語の連続区間ごとに選択モデルで文字起こしする(日英混在ファイル向け)。
/// セグメントは確定するたびに "transcribe-segment" イベントでリアルタイム送信する。
#[tauri::command]
pub async fn transcribe(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    model_id: String,
    language: String,
    translate: bool,
    ocr_enabled: bool,
    ocr_interval_secs: u32,
) -> Result<Vec<Segment>, String> {
    let model_path = model::model_path(&app, &model_id)?;
    if !model_path.exists() {
        return Err(format!(
            "モデル「{model_id}」が未ダウンロードです。「モデルの管理」からダウンロードしてください。"
        ));
    }

    // 混在モードの言語判定には高速な tiny モデルを使う。未取得なら先にダウンロード(約75MB)
    if language == "mixed" {
        let tiny_path = model::model_path(&app, "tiny")?;
        if !tiny_path.exists() {
            let _ = app.emit(
                "transcribe-status",
                StatusPayload { stage: "preparing".into() },
            );
            model::download_model(app.clone(), "tiny".to_string()).await?;
        }
    }

    // 長い文字起こしの後に依存不足で失敗しないよう、OCR環境は先に確認する。
    if ocr_enabled {
        crate::ocr::ensure_ocr_support()?;
    }

    // 前回のキャンセル状態をリセット
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel = state.cancel_flag.clone();

    let app_handle = app.clone();
    let job_path = path.clone();
    let job_language = language.clone();

    // whisper.cpp の実行は重いのでブロッキングスレッドで行う
    // GPU (CUDA) が利用可能ならそちらを使う
    let use_gpu = gpu::is_cuda_available(&app);

    let segments = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Segment>, String> {
        // GPU パス: CUDA whisper-cli に任せる
        if use_gpu {
            let _ = app_handle.emit(
                "transcribe-status",
                StatusPayload { stage: "running".into() },
            );
            return gpu::transcribe_with_cuda(
                &app_handle,
                &job_path,
                &model_path.to_string_lossy(),
                &job_language,
                translate,
                &cancel,
            );
        }

        // 1. デコード
        let _ = app_handle.emit(
            "transcribe-status",
            StatusPayload { stage: "decoding".into() },
        );
        let audio = crate::audio::decode_to_mono_16k(&job_path)?;
        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }

        // 2. モデル読み込み
        let _ = app_handle.emit(
            "transcribe-status",
            StatusPayload { stage: "loading_model".into() },
        );
        let ctx = WhisperContext::new_with_params(
            &model_path,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("モデルの読み込みに失敗しました: {e}"))?;
        let mut wstate = ctx
            .create_state()
            .map_err(|e| format!("初期化に失敗しました: {e}"))?;

        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .min(8);

        // 3. 実行(混在モード or 通常モード)
        // 2秒未満の極端に短い音声は言語判定が安定しないため通常の自動判定へ
        let mut segments = if job_language == "mixed" && audio.len() >= 2 * SAMPLE_RATE {
            // 判定専用に tiny を読み込む(判定は分類タスクなので tiny で十分)
            let tiny_path = model::model_path(&app_handle, "tiny")?;
            let det_ctx = WhisperContext::new_with_params(
                &tiny_path,
                WhisperContextParameters::default(),
            )
            .map_err(|e| format!("判定用モデルの読み込みに失敗しました: {e}"))?;
            let mut det_state = det_ctx
                .create_state()
                .map_err(|e| format!("判定用モデルの初期化に失敗しました: {e}"))?;

            transcribe_mixed(
                &app_handle,
                &mut det_state,
                &mut wstate,
                &audio,
                translate,
                threads,
                &cancel,
            )?
        } else {
            let lang = if job_language == "mixed" { "auto" } else { job_language.as_str() };
            let _ = app_handle.emit(
                "transcribe-status",
                StatusPayload { stage: "running".into() },
            );
            run_whisper(
                &app_handle,
                &mut wstate,
                &audio,
                0..audio.len(),
                lang,
                translate,
                threads,
                &cancel,
            )?
        };

        if ocr_enabled {
            let (mut ocr_segments, ocr_snapshots) = crate::ocr::extract_text_segments(
                &app_handle,
                &job_path,
                ocr_interval_secs,
                &cancel,
            )?;
            // 累積 OCR テキストを AppState に保存（フロントから取得できるように）
            app_handle
                .state::<crate::AppState>()
                .ocr_snapshots
                .lock()
                .map_err(|e| format!("内部エラー: {e}"))?
                .replace(Some((ocr_snapshots, job_path.clone())));
            let _ = app_handle.emit(
                "transcribe-status",
                StatusPayload { stage: "merging".into() },
            );
            segments.append(&mut ocr_segments);
            segments.sort_by(|a, b| {
                a.start_ms
                    .cmp(&b.start_ms)
                    // 同時刻なら「音声 → 画面」の順でノートに並べる。
                    .then_with(|| (a.source == "ocr").cmp(&(b.source == "ocr")))
            });
        }

        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }
        Ok(segments)
    })
    .await
    .map_err(|e| format!("内部エラー: {e}"))??;

    // 履歴に保存(保存に失敗しても文字起こし結果は返す)
    let file_name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    if let Err(e) =
        crate::history::save_entry(&app, &file_name, &model_id, &language, &segments)
    {
        eprintln!("履歴の保存に失敗しました: {e}");
    }

    Ok(segments)
}

/// audio[range] を言語 lang で 1 回文字起こしし、
/// range.start に応じたオフセットを加えたセグメントを返す。
/// 各セグメントは確定した時点で "transcribe-segment" としてリアルタイム送信する。
/// 進捗は「音声全体に対する割合」として emit する。
#[allow(clippy::too_many_arguments)]
fn run_whisper(
    app: &AppHandle,
    wstate: &mut WhisperState,
    audio: &[f32],
    range: std::ops::Range<usize>,
    lang: &str,
    translate: bool,
    threads: i32,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let chunk = &audio[range.clone()];
    // タイムスタンプは centi 秒(10ms)単位。区間の開始位置ぶんオフセットする
    let offset_cs = (range.start as i64) * 100 / SAMPLE_RATE as i64;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // whisper.cpp は "auto" を渡すと言語自動判定になる
    params.set_language(Some(lang));
    params.set_translate(translate);
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    // この区間の進捗(0-100)を音声全体に対する進捗へ換算して通知
    let base = range.start as f64;
    let len = range.len() as f64;
    let total = (audio.len() as f64).max(1.0);
    let progress_app = app.clone();
    params.set_progress_callback_safe(move |p: i32| {
        let overall = ((base + len * p as f64 / 100.0) / total * 100.0) as i32;
        let _ = progress_app.emit("transcribe-progress", overall.min(100));
    });

    // セグメント確定ごとにリアルタイム送信
    // (set_segment_callback_safe_lossy の内部実装は型が正しいため素のクロージャで安全)
    let seg_app = app.clone();
    params.set_segment_callback_safe_lossy(move |data: SegmentCallbackData| {
        let text = data.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let _ = seg_app.emit(
            "transcribe-segment",
            Segment {
                start_ms: (data.start_timestamp + offset_cs) * 10,
                end_ms: (data.end_timestamp + offset_cs) * 10,
                text,
                source: "speech".into(),
            },
        );
    });

    // キャンセルフラグが立ったら中断する。
    //
    // 【重要】whisper-rs 0.16.0 の set_abort_callback_safe には型不一致のバグがある。
    // 内部の FFI トランポリンが誤った型でインスタンス化されているため、素のクロージャを
    // 渡すとコールバックが不正なメモリを読んで不定値を返し、whisper が開始直後に中断
    // されて "failed to encode" になる。
    // あらかじめ Box<dyn FnMut() -> bool> に変換して渡すと内部の型表現が一致し、
    // 正しく動作する。素のクロージャに戻さないこと。
    let abort_flag = cancel.clone();
    let abort_cb: Box<dyn FnMut() -> bool> =
        Box::new(move || abort_flag.load(Ordering::SeqCst));
    params.set_abort_callback_safe(abort_cb);

    // 注: キャンセルで中断された場合も full() はエラーを返すため、
    // キャンセル起因かどうかを見てメッセージを分ける
    if let Err(e) = wstate.full(params, chunk) {
        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }
        return Err(format!("文字起こしに失敗しました: {e}"));
    }

    let n = wstate.full_n_segments();
    let mut segments = Vec::with_capacity(n as usize);
    for i in 0..n {
        if let Some(seg) = wstate.get_segment(i) {
            let text = seg
                .to_str_lossy()
                .map(|c| c.into_owned())
                .unwrap_or_default()
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            segments.push(Segment {
                start_ms: (seg.start_timestamp() + offset_cs) * 10,
                end_ms: (seg.end_timestamp() + offset_cs) * 10,
                text,
                source: "speech".into(),
            });
        }
    }
    Ok(segments)
}

/// 日英などの混在ファイル向け:
/// tiny モデルで 10 秒窓ごとに言語を判定し、
/// 同一言語が連続する区間にまとめてから、選択モデルで区間ごとに文字起こしする。
fn transcribe_mixed(
    app: &AppHandle,
    det_state: &mut WhisperState,
    main_state: &mut WhisperState,
    audio: &[f32],
    translate: bool,
    threads: i32,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let window = DETECT_WINDOW_SEC * SAMPLE_RATE;
    let n_windows = audio.len().div_ceil(window);

    let _ = app.emit(
        "transcribe-status",
        StatusPayload { stage: "detecting".into() },
    );

    // 1. 各窓の言語IDを tiny で判定(detect_language=true は判定のみで即座に返る)
    let mut langs: Vec<i32> = Vec::with_capacity(n_windows);
    for w in 0..n_windows {
        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }
        let start = w * window;
        let end = (start + window).min(audio.len());

        // 1秒未満の末尾窓は判定が不安定なので直前の言語を引き継ぐ
        if end - start < SAMPLE_RATE {
            if let Some(&prev) = langs.last() {
                langs.push(prev);
                continue;
            }
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("auto"));
        params.set_detect_language(true);
        params.set_n_threads(threads);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // run_whisper と同じ理由で Box<dyn> に包んで渡す(バグ回避)
        let abort_flag = cancel.clone();
        let abort_cb: Box<dyn FnMut() -> bool> =
            Box::new(move || abort_flag.load(Ordering::SeqCst));
        params.set_abort_callback_safe(abort_cb);

        if let Err(e) = det_state.full(params, &audio[start..end]) {
            if cancel.load(Ordering::SeqCst) {
                return Err("キャンセルされました".into());
            }
            return Err(format!("言語判定に失敗しました: {e}"));
        }
        langs.push(det_state.full_lang_id_from_state());

        // 判定フェーズの進捗(0-100)
        let p = ((w + 1) * 100 / n_windows) as i32;
        let _ = app.emit("transcribe-progress", p);
    }

    // 2. 同一言語の連続窓を1つの区間にまとめる
    let mut groups: Vec<(usize, usize, i32)> = Vec::new(); // (start, end, lang_id)
    for (w, &lang) in langs.iter().enumerate() {
        let start = w * window;
        let end = ((w + 1) * window).min(audio.len());
        match groups.last_mut() {
            Some(last) if last.2 == lang => last.1 = end,
            _ => groups.push((start, end, lang)),
        }
    }

    // 3. 区間ごとに、その言語を明示して選択モデルで文字起こし
    let _ = app.emit(
        "transcribe-status",
        StatusPayload { stage: "running".into() },
    );
    let mut all = Vec::new();
    for (start, end, lang_id) in groups {
        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }
        let lang = whisper_rs::get_lang_str(lang_id).unwrap_or("auto");
        let mut segs = run_whisper(
            app, main_state, audio, start..end, lang, translate, threads, cancel,
        )?;
        all.append(&mut segs);
    }
    Ok(all)
}

/// 実行中の文字起こしをキャンセルする
#[tauri::command]
pub fn cancel_transcribe(state: State<'_, AppState>) {
    state.cancel_flag.store(true, Ordering::SeqCst);
}

/// テキストをファイルに保存する(保存先はフロント側の保存ダイアログで選択済み)
#[tauri::command]
pub fn save_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("保存に失敗しました: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_history_segments_default_to_speech() {
        let segment: Segment = serde_json::from_str(
            r#"{"start_ms":0,"end_ms":1000,"text":"過去の文字起こし"}"#,
        )
        .unwrap();
        assert_eq!(segment.source, "speech");
    }
}
