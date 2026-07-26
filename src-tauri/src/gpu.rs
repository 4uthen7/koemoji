// @4uthent / tkmt_wonderkid
#![allow(unreachable_code)]
// gpu.rs — GPU 検出 + whisper-cli-gpu の実行

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

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
        "GPU を検出。ビルド時に whisper-cli-gpu が生成されます。".into()
    } else {
        "GPU は検出されませんでした（CPU で動作します）。".into()
    };
    GpuSupport { gpu_available, gpu_cli_found: cli_found, message }
}

pub fn is_gpu_available() -> bool {
    find_whisper_cli_gpu().is_some()
}

fn find_whisper_cli_gpu() -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") { "whisper-cli-gpu.exe" }
        else { "whisper-cli-gpu" };
    if let Ok(path) = std::env::var("WHISPER_CLI_GPU") {
        let p = PathBuf::from(&path);
        if p.exists() { return Some(p); }
    }
    if let Ok(exe) = std::env::current_exe() {
        let beside = exe.parent().unwrap_or(std::path::Path::new(".")).join(name);
        if beside.exists() { return Some(beside); }
    }
    for dir in &["target/release", "target/debug"] {
        let p = PathBuf::from(dir).join(name);
        if p.exists() { return Some(p); }
    }
    if let Ok(out) = std::env::var("OUT_DIR") {
        let p = PathBuf::from(&out).join(name);
        if p.exists() { return Some(p); }
    }
    None
}

/// whisper-cli-gpu で文字起こし。
/// 音声ファイル(mp3/wav/flac/ogg)は直接、動画はffmpegで音声抽出してから渡す。
pub fn transcribe_with_gpu(
    app: &AppHandle,
    audio_path: &str,
    model_path: &str,
    language: &str,
    translate: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Segment>, String> {
    let cli = find_whisper_cli_gpu()
        .ok_or("whisper-cli-gpu が見つかりません")?;

    // 入力ファイルの準備（動画ならffmpegで音声抽出）
    let (input_path, _temp_file) = prepare_audio(audio_path)?;
    if cancel.load(Ordering::SeqCst) { return Err("キャンセルされました".into()); }

    let mut cmd = Command::new(&cli);
    cmd.arg("-m").arg(model_path);
    cmd.arg("-l").arg(language);
    cmd.arg("-oj"); // JSONファイル出力
    cmd.arg("-pp"); // プログレス表示
    if translate { cmd.arg("-tr"); }
    cmd.arg(&input_path);

    // Windows: CUDA Toolkit の bin ディレクトリを PATH に追加
    // （インストール済みでも PATH が通ってないケースが多いため）
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsString;
        let extra_paths = find_cuda_bin_dirs();
        if !extra_paths.is_empty() {
            let separator = if cfg!(target_os = "windows") { ";" } else { ":" };
            let existing = std::env::var("PATH").unwrap_or_default();
            let mut new_path = existing;
            for p in &extra_paths {
                new_path.push_str(separator);
                new_path.push_str(&p.to_string_lossy());
            }
            cmd.env("PATH", new_path);
        }
    }

    let mut child = cmd
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("whisper-cli-gpu 起動失敗: {e}"))?;

    let stderr = child.stderr.take().unwrap();
    let stderr_reader = BufReader::new(stderr);

    // stderr からプログレスを読む
    let cancel_flag = cancel.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        for line in stderr_reader.lines() {
            if cancel_flag.load(Ordering::SeqCst) { break; }
            if let Ok(line) = line {
                for word in line.split_whitespace() {
                    if let Ok(p) = word.trim_end_matches('%').parse::<i32>() {
                        let _ = app_clone.emit("transcribe-progress", p.min(100));
                    }
                }
            }
        }
    });

    // キャンセルをポーリングしながらプロセス終了を待つ
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(ref f) = _temp_file { let _ = fs::remove_file(f); }
            return Err("キャンセルされました".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => {
                if let Some(ref f) = _temp_file { let _ = fs::remove_file(f); }
                return Err(format!("プロセス待機エラー: {e}"));
            }
        }
    };

    if !status.success() {
        if let Some(ref f) = _temp_file { let _ = fs::remove_file(f); }
        return Err(format!(
            "whisper-cli-gpu 異常終了 (exit: {})",
            status.code().unwrap_or(-1)
        ));
    }

    // JSON出力ファイルを読む（{input_path}.json）
    let json_path = format!("{input_path}.json");
    let json_str = fs::read_to_string(&json_path)
        .map_err(|e| format!("JSON出力の読み取り失敗: {e}"))?;
    let _ = fs::remove_file(&json_path);
    if let Some(ref f) = _temp_file { let _ = fs::remove_file(f); }

    let parsed: WhisperOutput = serde_json::from_str(&json_str)
        .map_err(|e| format!("JSONパース失敗: {e}"))?;

    let mut segments = Vec::new();
    for item in parsed.transcription {
        let text = item.text.trim().to_string();
        if text.is_empty() { continue; }
        // offsets.from/to はミリ秒（整数）
        let seg = Segment {
            start_ms: item.offsets.from,
            end_ms: item.offsets.to,
            text,
            source: "speech".into(),
        };
        let _ = app.emit("transcribe-segment", seg.clone());
        segments.push(seg);
    }

    if segments.is_empty() {
        return Err("文字起こし結果が空です".into());
    }

    Ok(segments)
}

/// 入力ファイルを準備。動画ならffmpegで音声抽出した一時ファイルを返す。
/// 戻り値: (実際にwhisperに渡すパス, 後始末用の一時ファイルパス(あれば))
fn prepare_audio(path: &str) -> Result<(String, Option<PathBuf>), String> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // whisper-cli が直接読める形式
    let supported = ["wav", "mp3", "flac", "ogg", "oga", "m4a", "aac"];
    if supported.contains(&ext.as_str()) {
        return Ok((path.to_string(), None));
    }

    // 動画など → ffmpeg で 16kHz mono wav に変換
    let ffmpeg = crate::tools::find_executable("ffmpeg")
        .ok_or("ffmpeg が見つかりません")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("時刻取得失敗: {e}"))?
        .as_nanos();
    let tmp = std::env::temp_dir().join(format!("koemoji-gpu-{nonce}.wav"));
    let out = Command::new(&ffmpeg)
        .args(["-y", "-i", path, "-vn", "-ar", "16000", "-ac", "1", "-sample_fmt", "s16"])
        .arg(&tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg 起動失敗: {e}"))?;
    if !out.status.success() {
        let _ = fs::remove_file(&tmp);
        return Err("ffmpeg で音声抽出に失敗しました".into());
    }
    let tmp_str = tmp.to_string_lossy().to_string();
    Ok((tmp_str, Some(tmp)))
}

// ---- JSONパース用 ----

#[derive(serde::Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperItem>,
}

#[derive(serde::Deserialize)]
struct WhisperItem {
    text: String,
    offsets: WhisperOffsets,
}

#[derive(serde::Deserialize)]
struct WhisperOffsets {
    from: i64,
    to: i64,
}

// ---- CUDA パス解決（Windows） ----

#[cfg(target_os = "windows")]
fn find_cuda_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // CUDA Toolkit 標準インストール先
    let base = PathBuf::from("C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA");
    if let Ok(entries) = fs::read_dir(&base) {
        for e in entries.flatten() {
            let bin = e.path().join("bin");
            if bin.exists() { dirs.push(bin); }
        }
    }
    // CUDA が Program Files 以外に入ってる場合の環境変数
    for key in &["CUDA_PATH", "CUDA_HOME", "CUDA_ROOT"] {
        if let Ok(val) = std::env::var(key) {
            let p = PathBuf::from(&val).join("bin");
            if p.exists() && !dirs.contains(&p) { dirs.push(p); }
        }
    }
    dirs
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn find_cuda_bin_dirs() -> Vec<PathBuf> { Vec::new() }

// ---- 検出 ----

fn detect_gpu() -> bool {
    #[allow(unreachable_code)]
    #[cfg(target_os = "macos")]
    { return true; }
    #[cfg(target_os = "windows")]
    {
        if Command::new("nvidia-smi").output().map(|o| o.status.success()).unwrap_or(false) { return true; }
        let s = PathBuf::from("C:\\Windows\\System32");
        if s.join("nvml.dll").exists() || s.join("nvcuda.dll").exists() { return true; }
    }
    false
}

fn check_gpu_runtime() -> bool {
    #[allow(unreachable_code)]
    #[cfg(target_os = "macos")]
    { return true; }
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
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    { Command::new("nvidia-smi").output().map(|o| o.status.success()).unwrap_or(false) }
}
