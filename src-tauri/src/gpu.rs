// @4uthent / tkmt_wonderkid
// gpu.rs — CUDA 検出 + whisper-cli-cuda.exe の実行（build.rs が自動生成）

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;

use crate::transcribe::Segment;

#[derive(Serialize, Clone)]
pub struct GpuSupport {
    pub cuda_available: bool,
    pub cuda_cli_found: bool,
    pub message: String,
}

#[tauri::command]
pub fn check_gpu_support() -> GpuSupport {
    let cuda_available = detect_cuda() && check_cuda_runtime();
    let cli_found = find_whisper_cli_cuda().is_some();

    let message = if cli_found {
        "GPU アクセラレーション有効（whisper-cli-cuda.exe 検出）".into()
    } else if cuda_available {
        "NVIDIA GPU + CUDA を検出。build.rs が whisper-cli-cuda.exe を自動生成します。".into()
    } else {
        "GPU は検出されませんでした（CPU で動作します）。".into()
    };

    GpuSupport { cuda_available, cuda_cli_found: cli_found, message }
}

pub fn is_cuda_available() -> bool {
    find_whisper_cli_cuda().is_some()
}

/// whisper-cli-cuda.exe を探す。
fn find_whisper_cli_cuda() -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") { "whisper-cli-cuda.exe" }
        else { "whisper-cli-cuda" };

    // ビルド時に生成されたパス（環境変数）
    if let Ok(path) = std::env::var("WHISPER_CLI_CUDA") {
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

/// whisper-cli-cuda.exe で文字起こし。
pub fn transcribe_with_cuda(
    app: &AppHandle,
    audio_path: &str,
    model_path: &str,
    language: &str,
    translate: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let cli = find_whisper_cli_cuda()
        .ok_or("whisper-cli-cuda.exe が見つかりません。nvcc をインストールして再ビルドしてください。")?;

    let mut cmd = Command::new(&cli);
    cmd.arg("-m").arg(model_path);
    cmd.arg("-f").arg(audio_path);
    cmd.arg("-l").arg(language);
    cmd.arg("--output-json");
    if translate { cmd.arg("--translate"); }

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("whisper-cli-cuda の起動に失敗: {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut raw = String::new();
    reader.read_to_string(&mut raw).map_err(|e| format!("出力読み取りエラー: {e}"))?;

    if cancel.load(Ordering::SeqCst) {
        let _ = child.kill(); let _ = child.wait();
        return Err("キャンセルされました".into());
    }

    let status = child.wait().map_err(|e| format!("プロセス待機エラー: {e}"))?;
    if !status.success() {
        return Err("whisper-cli-cuda が異常終了しました".into());
    }

    let mut segments = Vec::new();

    // パターンA: フルJSON
    if let Ok(full) = serde_json::from_str::<WhisperCliFull>(&raw) {
        for item in full.transcription {
            let text = item.text.trim().to_string();
            if text.is_empty() { continue; }
            segments.push(Segment {
                start_ms: parse_timecode(&item.timestamps.from),
                end_ms: parse_timecode(&item.timestamps.to),
                text,
                source: "speech".into(),
            });
        }
    } else {
        // パターンB: 1行ずつ
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Ok(s) = serde_json::from_str::<WhisperCliSegment>(line) {
                let text = s.text.trim().to_string();
                if text.is_empty() { continue; }
                segments.push(Segment {
                    start_ms: parse_timecode(&s.from),
                    end_ms: parse_timecode(&s.to),
                    text,
                    source: "speech".into(),
                });
            }
        }
    }

    Ok(segments)
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
    let parts: Vec<&str> = tc.replace(',', ".").split(':').collect();
    if parts.len() == 3 {
        let h: i64 = parts[0].parse().unwrap_or(0);
        let m: i64 = parts[1].parse().unwrap_or(0);
        let s: f64 = parts[2].parse().unwrap_or(0.0);
        return h * 3_600_000 + m * 60_000 + (s * 1000.0) as i64;
    }
    0
}

// ---- 検出 ----

fn detect_cuda() -> bool {
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

fn check_cuda_runtime() -> bool {
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
