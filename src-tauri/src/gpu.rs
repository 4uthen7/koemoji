// @4uthent / tkmt_wonderkid

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::transcribe::Segment;

// ---- ダウンロード元の whisper-cli (CUDA ビルド) ----
// GitHub Releases にアップロードされた CUDA 対応 whisper-cli の URL。
// タグ名とファイル名を埋めて使う。
const CUDA_WHISPER_RELEASE_TAG: &str = "cuda-runtime";
const CUDA_WHISPER_BINARY: &str = "whisper-cli-cuda.exe";

#[derive(Serialize, Clone)]
pub struct GpuSupport {
    pub cuda_available: bool,
    pub cuda_runtime_installed: bool,
    pub cuda_whisper_downloaded: bool,
    pub message: String,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

/// CUDA が利用可能か総合的にチェックする。
#[tauri::command]
pub fn check_gpu_support(app: AppHandle) -> GpuSupport {
    let cuda_available = detect_cuda();
    let cuda_runtime_installed = check_cuda_runtime();
    let downloaded = cuda_whisper_path(&app).map(|p| p.exists()).unwrap_or(false);

    let message = if downloaded {
        "GPU アクセラレーションが有効です。文字起こしに CUDA が使われます。".into()
    } else if cuda_available && cuda_runtime_installed {
        "NVIDIA GPU を検出しました。「GPU を有効化」をクリックすると CUDA 対応エンジンをダウンロードします。".into()
    } else if cuda_available {
        "NVIDIA GPU は検出されましたが CUDA ランタイムが見つかりません。CUDA Toolkit をインストールしてください。".into()
    } else {
        "NVIDIA GPU が検出されませんでした。CPU で文字起こしを行います。".into()
    };

    GpuSupport {
        cuda_available,
        cuda_runtime_installed,
        cuda_whisper_downloaded: downloaded,
        message,
    }
}

/// CUDA 対応 whisper-cli をダウンロードする。
/// モデルダウンロードと同じ仕組み（進捗イベント付き）。
#[tauri::command]
pub async fn download_cuda_whisper(app: AppHandle) -> Result<(), String> {
    let final_path = cuda_whisper_path(&app)?;
    if final_path.exists() {
        return Ok(());
    }

    let part_path = final_path.with_extension("exe.part");

    let url = format!(
        "https://github.com/4uthen7/koemoji/releases/download/{tag}/{binary}",
        tag = CUDA_WHISPER_RELEASE_TAG,
        binary = CUDA_WHISPER_BINARY,
    );

    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("ダウンロードを開始できませんでした: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "ダウンロードに失敗しました (HTTP {})。GitHub Releases に CUDA バイナリがアップロードされているか確認してください。",
            response.status()
        ));
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(&part_path)
        .map_err(|e| format!("ファイルを作成できませんでした: {e}"))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emitted_mb: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("ダウンロード中にエラー: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("書き込みに失敗: {e}"))?;
        downloaded += chunk.len() as u64;

        let mb = downloaded >> 20;
        if mb != last_emitted_mb {
            last_emitted_mb = mb;
            let _ = app.emit(
                "gpu-download-progress",
                DownloadProgress { downloaded, total },
            );
        }
    }

    file.flush().map_err(|e| format!("書き込みに失敗: {e}"))?;
    drop(file);
    fs::rename(&part_path, &final_path)
        .map_err(|e| format!("ファイルの確定に失敗: {e}"))?;

    #[cfg(target_os = "windows")]
    {
        // Windows では exe に実行権限は不要（拡張子で判定される）
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&final_path)
            .map_err(|e| format!("メタデータ取得失敗: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&final_path, perms)
            .map_err(|e| format!("実行権限の設定に失敗: {e}"))?;
    }

    let _ = app.emit(
        "gpu-download-progress",
        DownloadProgress {
            downloaded,
            total: if total > 0 { total } else { downloaded },
        },
    );
    Ok(())
}

/// CUDA whisper-cli のバイナリパスを返す。
pub fn cuda_whisper_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("アプリデータフォルダ取得失敗: {e}"))?
        .join("gpu");
    fs::create_dir_all(&dir).map_err(|e| format!("フォルダ作成失敗: {e}"))?;
    Ok(dir.join(CUDA_WHISPER_BINARY))
}

/// CUDA whisper-cli がダウンロード済みで実行可能か。
pub fn is_cuda_available(app: &AppHandle) -> bool {
    cuda_whisper_path(app)
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// CUDA whisper-cli を使って文字起こしを実行する。
/// whisper-cli の JSON 出力をパースして Segment の配列を返す。
pub fn transcribe_with_cuda(
    app: &AppHandle,
    audio_path: &str,
    model_path: &str,
    language: &str,
    translate: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let whisper_bin = cuda_whisper_path(app)?;
    if !whisper_bin.exists() {
        return Err("CUDA whisper バイナリが見つかりません。GPU 有効化を実行してください。".into());
    }

    let mut cmd = Command::new(&whisper_bin);
    cmd.arg("-m").arg(model_path);
    cmd.arg("-f").arg(audio_path);
    cmd.arg("-l").arg(language);
    cmd.arg("--output-json");
    cmd.arg("--print-progress");

    if translate {
        cmd.arg("--translate");
    }

    // whisper-cli は stdout に JSON を出力する（--output-json 指定時）
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("CUDA whisper の起動に失敗: {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let reader = std::io::BufReader::new(stdout);
    let mut segments = Vec::new();

    // whisper-cli --output-json の出力をパースする。
    // フォーマットはバージョンによって異なるため複数パターンを試す:
    //   A) {"transcription": [{"timestamps": {"from":"HH:MM:SS.mmm","to":...}, "text":"..."}]}
    //   B) 1行ずつ {"from":"...","to":"...","text":"..."}
    let mut raw_output = String::new();
    use std::io::Read;
    reader
        .read_to_string(&mut raw_output)
        .map_err(|e| format!("出力読み取りエラー: {e}"))?;
    if cancel.load(Ordering::SeqCst) {
        let _ = child.kill();
        let _ = child.wait();
        return Err("キャンセルされました".into());
    }

    // パターンA: フルJSON
    if let Ok(full) = serde_json::from_str::<WhisperCliFull>(&raw_output) {
        for item in full.transcription {
            if cancel.load(Ordering::SeqCst) { break; }
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
    } else {
        // パターンB: 1行ずつ
        for line in raw_output.lines() {
            if cancel.load(Ordering::SeqCst) { break; }
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Ok(s) = serde_json::from_str::<WhisperCliSegment>(line) {
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
        }
    }

    let status = child.wait().map_err(|e| format!("プロセス待機エラー: {e}"))?;
    if !status.success() {
        return Err("CUDA whisper が異常終了しました".into());
    }

    Ok(segments)
}

#[derive(serde::Deserialize)]
struct WhisperCliFull {
    transcription: Vec<WhisperCliItem>,
}

#[derive(serde::Deserialize)]
struct WhisperCliItem {
    timestamps: WhisperCliTimestamps,
    text: String,
}

#[derive(serde::Deserialize)]
struct WhisperCliTimestamps {
    from: String,
    to: String,
}

/// 1行単位のシンプルな JSON 出力用
#[derive(serde::Deserialize)]
struct WhisperCliSegment {
    from: String,
    to: String,
    text: String,
}

/// "HH:MM:SS.mmm" または "HH:MM:SS,mmm" または秒数文字列をミリ秒に変換
fn parse_timecode(tc: &str) -> i64 {
    // 秒数(float)の場合: "123.456"
    if let Ok(secs) = tc.parse::<f64>() {
        return (secs * 1000.0) as i64;
    }
    // "HH:MM:SS.mmm" または "HH:MM:SS,mmm"
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

// ---- 内部検出ロジック ----

/// NVIDIA GPU + CUDA ドライバが存在するか。
fn detect_cuda() -> bool {
    // nvidia-smi が使えるか
    if let Ok(output) = Command::new("nvidia-smi").output() {
        if output.status.success() {
            return true;
        }
    }
    // Windows: NVML の存在チェック
    #[cfg(target_os = "windows")]
    {
        if std::path::Path::new("C:\\Windows\\System32\\nvml.dll").exists()
            || std::path::Path::new("C:\\Windows\\System32\\nvcuda.dll").exists()
        {
            return true;
        }
    }
    false
}

/// CUDA Toolkit ランタイムがインストールされているか。
fn check_cuda_runtime() -> bool {
    // nvcc が使えるか
    if let Ok(output) = Command::new("nvcc").arg("--version").output() {
        if output.status.success() {
            return true;
        }
    }
    // Windows: CUDA ランタイム DLL の存在
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            "C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA",
            "C:\\Program Files\\NVIDIA Corporation\\NvStreamSrv",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return true;
            }
        }
    }
    false
}
