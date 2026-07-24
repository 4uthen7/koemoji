// @4uthent / tkmt_wonderkid

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
use tauri::{Emitter, Manager, State};

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
    pub ocr_snapshots: Mutex<Option<(Vec<OcrSnapshot>, String)>>,
    pub opened_file: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct CumulativeOcrResult {
    text: String,
    available: bool,
}

#[tauri::command]
fn get_cumulative_ocr_text(
    state: State<'_, AppState>,
) -> Result<CumulativeOcrResult, String> {
    let guard = state.ocr_snapshots.lock().map_err(|e| format!("内部エラー: {e}"))?;
    match guard.as_ref() {
        Some((snapshots, source_name)) if !snapshots.is_empty() => {
            let name = std::path::Path::new(source_name)
                .file_name().and_then(|n| n.to_str()).unwrap_or("video");
            let text = crate::ocr::format_cumulative_ocr_text(snapshots, name);
            Ok(CumulativeOcrResult { text, available: true })
        }
        _ => Ok(CumulativeOcrResult { text: String::new(), available: false }),
    }
}

#[tauri::command]
fn get_opened_file(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut guard = state.opened_file.lock().map_err(|e| format!("内部エラー: {e}"))?;
    Ok(guard.take())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cli_file: Option<String> = std::env::args()
        .nth(1)
        .filter(|p| std::path::Path::new(p).exists());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            ocr_snapshots: Mutex::new(None),
            opened_file: Mutex::new(cli_file),
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let thread_handle = handle.clone();
            let state = handle.state::<AppState>();
            if let Ok(guard) = state.opened_file.lock() {
                if let Some(ref path) = *guard {
                    let path = path.clone();
                    drop(guard);
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        let _ = thread_handle.emit("file-opened", path);
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            gpu::check_gpu_support,
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
            get_opened_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
