mod audio;
mod history;
mod model;
mod ocr;
mod tools;
mod transcribe;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// アプリ全体で共有する状態
pub struct AppState {
    /// 文字起こしのキャンセルフラグ
    pub cancel_flag: Arc<AtomicBool>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
