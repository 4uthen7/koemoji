// @4uthent / tkmt_wonderkid
// gpu.rs — GPU 検出 + whisper-cli-gpu の実行（build.rs が自動生成）

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::AppHandle;

use crate::transcribe::Segment;

#[derive(Serialize, Clone)]
pub struct GpuSupport {
    pub gpu_available: bool,
    pub gpu_cli_found: bool,
    pub message: String,
}

#[tauri::command]
pub fn check_gpu_support() -> GpuSupport {
    let gpu_available = detect_gpu() && check_gpu_runtime();
    let cli_found = find_whisper_cli_gpu().is_some();

    let message = if cli_found {
        "GPU アクセラレーション有効".into()
    } else if gpu_available {
        "GPU を検出。build.rs が whisper-cli-gpu を自動生成します。".into()
    } else {
        "GPU は検出されませんでした（CPU で動作します）".into()
    };

    GpuSupport { gpu_available, gpu_cli_found: cli_found, message }
}

pub fn is_gpu_available() -> bool {
    find_whisper_cli_gpu().is_some()
}

/// whisper-cli-gpu.exe を探す。
fn find_whisper_cli_gpu() -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") { "whisper-cli-gpu.exe" }
        else { "whisper-cli-gpu" };

    // ビルド時に生成されたパス（環境変数）
    if let Ok(path) = std::env::var("WHISPER_CLI_GPU") {
        let p = PathBuf::from(&path);
        if p.exists() { return Some(p); }
    }
    // 実行ファイルと同じ場所
    if let Ok(exe) = std::env::current_exe() {
        let beside = exe.parent().unwrap_or(std::path::Path::new(".")).join(name);
        if beside.exists() { return Some(beside); }
    }
    // ターゲットディレクトリ
    for dir in &["target/release", "target/debug"] {
        let p = PathBuf::from(dir).join(name);
        if p.exists() { return Some(p); }
    }
    // OUT_DIR（ビルド時）
    if let Ok(out) = std::env::var("OUT_DIR") {
        let p = PathBuf::from(&out).join(name);
        if p.exists() { return Some(p); }
    }
    None
}

/// whisper-cli-gpu で文字起こし。
/// WAV に変換してから渡す（whisper-cli は WAV のみ対応）。
pub fn transcribe_with_gpu(
    app: &AppHandle,
    audio_path: &str,
    model_path: &str,
    language: &str,
    translate: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let cli = find_whisper_cli_gpu()
        .ok_or("whisper-cli-gpu が見つかりません。build.rs が生成しているか確認してください。")?;

    // 音声デコード → 一時WAV
    let samples = crate::audio::decode_to_mono_16k(audio_path)?;
    if cancel.load(Ordering::SeqCst) { return Err("キャンセルされました".into()); }
    let wav_path = write_temp_wav(&samples)?;

    let mut cmd = Command::new(&cli);
    cmd.arg("-m").arg(model_path);
    cmd.arg("-f").arg(&wav_path);
    cmd.arg("-l").arg(language);
    cmd.arg("--output-json");
    if translate { cmd.arg("--translate"); }

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("whisper-cli-gpu の起動に失敗: {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // 別スレッドで stdout を読み取り、チャネルで送る
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let cancel_flag = cancel.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if cancel_flag.load(Ordering::SeqCst) { break; }
            if let Ok(line) = line {
                if tx.send(line).is_err() { break; }
            }
        }
    });

    let mut segments = Vec::new();
    let mut raw_err = String::new();
    let mut stderr_reader = BufReader::new(stderr);

    // ポーリングループ: stdout 行を処理しつつキャンセルをチェック
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&wav_path);
            return Err("キャンセルされました".into());
        }

        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() { continue; }

                // パターンA: フルJSON配列1行
                if let Ok(items) = serde_json::from_str::<Vec<WhisperCliItem>>(&line) {
                    for item in items {
                        let text = item.text.trim().to_string();
                        if text.is_empty() { continue; }
                        let seg = Segment {
                            start_ms: parse_timecode(&item.timestamps.from),
                            end_ms: parse_timecode(&item.timestamps.to),
                            text,
                            source: "speech".into(),
                        };
                        let _ = app.emit("transcribe-segment", seg.clone());
                        segments.push(seg);
                    }
                }
                // パターンB: 1セグメント1行
                else if let Ok(s) = serde_json::from_str::<WhisperCliSegment>(&line) {
                    let text = s.text.trim().to_string();
                    if text.is_empty() { continue; }
                    let seg = Segment {
                        start_ms: parse_timecode(&s.from),
                        end_ms: parse_timecode(&s.to),
                        text,
                        source: "speech".into(),
                    };
                    let _ = app.emit("transcribe-segment", seg.clone());
                    segments.push(seg);
                }
                // パターンC: フルJSONオブジェクト（transcriptionキーあり）
                else if let Ok(full) = serde_json::from_str::<WhisperCliFull>(&line) {
                    for item in full.transcription {
                        let text = item.text.trim().to_string();
                        if text.is_empty() { continue; }
                        let seg = Segment {
                            start_ms: parse_timecode(&item.timestamps.from),
                            end_ms: parse_timecode(&item.timestamps.to),
                            text,
                            source: "speech".into(),
                        };
                        let _ = app.emit("transcribe-segment", seg.clone());
                        segments.push(seg);
                    }
                }
                // stderr からのプログレス行（数値のみ）
                else if let Ok(pct) = line.parse::<i32>() {
                    let _ = app.emit("transcribe-progress", pct.min(100));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // stdout が詰まってる間、stderr を読む（プログレス用）
                let mut buf = String::new();
                if stderr_reader.read_line(&mut buf).unwrap_or(0) > 0 {
                    raw_err.push_str(&buf);
                    // プログレス行を探す
                    for word in buf.split_whitespace() {
                        if let Ok(pct) = word.trim_end_matches('%').parse::<i32>() {
                            let _ = app.emit("transcribe-progress", pct.min(100));
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break; // stdout スレッド終了
            }
        }
    }

    // 残りの stderr を読む
    let mut buf = String::new();
    stderr_reader.read_to_string(&mut buf).ok();
    raw_err.push_str(&buf);

    let status = child.wait().map_err(|e| format!("プロセス待機エラー: {e}"))?;
    let _ = fs::remove_file(&wav_path);

    if cancel.load(Ordering::SeqCst) { return Err("キャンセルされました".into()); }

    if !status.success() {
        return Err(format!(
            "whisper-cli-gpu 異常終了 (exit: {})\n{}",
            status.code().unwrap_or(-1),
            raw_err.trim()
        ));
    }

    if segments.is_empty() {
        return Err("文字起こし結果が空です。モデルが正しいか確認してください。".into());
    }

    Ok(segments)
}

/// 16kHz mono f32 → 一時WAVファイル
fn write_temp_wav(samples: &[f32]) -> Result<PathBuf, String> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| format!("時刻取得失敗: {e}"))?.as_nanos();
    let path = std::env::temp_dir().join(format!("koemoji-gpu-{nonce}.wav"));
    let mut file = fs::File::create(&path).map_err(|e| format!("WAV作成失敗: {e}"))?;
    let data_len = (samples.len() * 2) as u32;
    let riff_size = 36u32.wrapping_add(data_len);
    file.write_all(b"RIFF").ok();
    file.write_all(&riff_size.to_le_bytes()).ok();
    file.write_all(b"WAVE").ok();
    file.write_all(b"fmt ").ok();
    file.write_all(&16u32.to_le_bytes()).ok();
    file.write_all(&1u16.to_le_bytes()).ok();
    file.write_all(&1u16.to_le_bytes()).ok();
    file.write_all(&16000u32.to_le_bytes()).ok();
    file.write_all(&32000u32.to_le_bytes()).ok();
    file.write_all(&2u16.to_le_bytes()).ok();
    file.write_all(&16u16.to_le_bytes()).ok();
    file.write_all(b"data").ok();
    file.write_all(&data_len.to_le_bytes()).ok();
    for &s in samples {
        let v = (s * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        file.write_all(&v.to_le_bytes()).map_err(|e| format!("WAV書込失敗: {e}"))?;
    }
    file.flush().ok();
    Ok(path)
}


#[derive(serde::Deserialize)]
struct WhisperCliFull { transcription: Vec<WhisperCliItem> }
#[derive(serde::Deserialize)]
struct WhisperCliItem { timestamps: WhisperCliTimestamps, text: String }
#[derive(serde::Deserialize)]
struct WhisperCliTimestamps { from: String, to: String }
#[derive(serde::Deserialize)]
struct WhisperCliSegment { from: String, to: String, text: String }

fn parse_timecode(tc: &str) -> i64 {
    if let Ok(s) = tc.parse::<f64>() { return (s * 1000.0) as i64; }
    let cleaned = tc.replace(',', ".");
    let parts: Vec<&str> = cleaned.split(':').collect();
    if parts.len() == 3 {
        let h: i64 = parts[0].parse().unwrap_or(0);
        let m: i64 = parts[1].parse().unwrap_or(0);
        let s: f64 = parts[2].parse().unwrap_or(0.0);
        return h * 3_600_000 + m * 60_000 + (s * 1000.0) as i64;
    }
    0
}

// ---- 検出 ----

fn detect_gpu() -> bool {
    if Command::new("nvidia-smi").output().map(|o| o.status.success()).unwrap_or(false) { return true; }
    #[cfg(target_os = "windows")]
    {
        if PathBuf::from("C:\\Windows\\System32\\nvidia-smi.exe").exists()
            && Command::new("C:\\Windows\\System32\\nvidia-smi.exe").output().map(|o| o.status.success()).unwrap_or(false)
        { return true; }
        let s = PathBuf::from("C:\\Windows\\System32");
        if s.join("nvml.dll").exists() || s.join("nvcuda.dll").exists() { return true; }
    }
    false
}

fn check_gpu_runtime() -> bool {
    #[cfg(target_os = "windows")]
    {
        let s = PathBuf::from("C:\\Windows\\System32");
        if s.join("nvcuda.dll").exists() { return true; }
        if let Ok(e) = fs::read_dir(&s) {
            for e in e.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if (n.starts_with("cudart64_") || n.starts_with("cublas64_")) && n.ends_with(".dll") { return true; }
            }
        }
        return false;
    }
    #[cfg(not(target_os = "windows"))]
    { Command::new("nvidia-smi").output().map(|o| o.status.success()).unwrap_or(false) }
}
