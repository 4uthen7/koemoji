mod audio;
mod gpu;
mod history;
mod model;
mod ocr;
mod tools;
mod transcribe;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::ocr::OcrSnapshot;
use serde::Serialize;
use tauri::State;

/// アプリ全体で共有する状態
pub struct AppState {
    /// 文字起こしのキャンセルフラグ
    pub cancel_flag: Arc<AtomicBool>,
    /// 最後に実行した OCR の累積スナップショット
    pub ocr_snapshots: Mutex<Option<(Vec<OcrSnapshot>, String)>>,
}

#[derive(Serialize)]
struct CumulativeOcrResult {
    text: String,
    available: bool,
}

/// 最後に実行した OCR の累積テキストを取得する。
/// 音声のみのファイルや OCR が未実行の場合は available=false で返る。
#[tauri::command]
fn get_cumulative_ocr_text(
    state: State<'_, AppState>,
) -> Result<CumulativeOcrResult, String> {
    let guard = state
        .ocr_snapshots
        .lock()
        .map_err(|e| format!("内部エラー: {e}"))?;
    match guard.as_ref() {
        Some((snapshots, source_name)) if !snapshots.is_empty() => {
            let name = std::path::Path::new(source_name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("video");
            let text = crate::ocr::format_cumulative_ocr_text(snapshots, name);
            Ok(CumulativeOcrResult {
                text,
                available: true,
            })
        }
        _ => Ok(CumulativeOcrResult {
            text: String::new(),
            available: false,
        }),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            ocr_snapshots: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            gpu::check_gpu_support,
            gpu::download_cuda_whisper,
            model::list_models,
            model::download_model,
            model::delete_model,
            transcribe::transcribe,
            transcribe::cancel_transcribe,
            transcribe::save_text_file,
            ocr::check_ocr_support,
            history::list_history,
            history::load_history,
            history::delete_history,
            get_cumulative_ocr_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
