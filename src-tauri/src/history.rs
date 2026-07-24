// @4uthent / tkmt_wonderkid

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::transcribe::Segment;

/// 履歴1件の完全なデータ(セグメント込み)
#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub id: String,
    pub file_name: String,
    pub model_id: String,
    pub language: String,
    pub created_at_ms: u64,
    pub segments: Vec<Segment>,
}

/// 一覧表示用の軽量なサマリ
#[derive(Serialize, Clone)]
pub struct HistorySummary {
    pub id: String,
    pub file_name: String,
    pub model_id: String,
    pub language: String,
    pub created_at_ms: u64,
    pub segment_count: usize,
    pub duration_ms: i64,
}

/// 履歴保存先: <アプリデータ>/history/<unixミリ秒>.json
fn history_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("アプリデータフォルダを取得できませんでした: {e}"))?
        .join("history");
    fs::create_dir_all(&dir).map_err(|e| format!("フォルダの作成に失敗しました: {e}"))?;
    Ok(dir)
}

/// id は数字のみを許可する(パストラバーサル防止)
fn entry_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("不正な履歴IDです".to_string());
    }
    Ok(history_dir(app)?.join(format!("{id}.json")))
}

/// 文字起こし成功時に履歴として保存する
pub fn save_entry(
    app: &AppHandle,
    file_name: &str,
    model_id: &str,
    language: &str,
    segments: &[Segment],
) -> Result<String, String> {
    let dir = history_dir(app)?;
    let mut id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("時刻の取得に失敗しました: {e}"))?
        .as_millis() as u64;
    // 同一ミリ秒の衝突を回避
    while dir.join(format!("{id}.json")).exists() {
        id += 1;
    }

    let entry = HistoryEntry {
        id: id.to_string(),
        file_name: file_name.to_string(),
        model_id: model_id.to_string(),
        language: language.to_string(),
        created_at_ms: id,
        segments: segments.to_vec(),
    };
    let json = serde_json::to_string(&entry)
        .map_err(|e| format!("履歴のシリアライズに失敗しました: {e}"))?;
    fs::write(dir.join(format!("{id}.json")), json)
        .map_err(|e| format!("履歴の保存に失敗しました: {e}"))?;
    Ok(entry.id)
}

/// 履歴一覧を新しい順で返す
#[tauri::command]
pub fn list_history(app: AppHandle) -> Result<Vec<HistorySummary>, String> {
    let dir = history_dir(&app)?;
    let mut summaries = Vec::new();

    let entries = fs::read_dir(&dir).map_err(|e| format!("履歴の読み込みに失敗しました: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // 壊れたファイルはスキップして続行
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(parsed) = serde_json::from_str::<HistoryEntry>(&text) else { continue };

        let duration_ms = parsed
            .segments
            .iter()
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0);
        summaries.push(HistorySummary {
            id: parsed.id,
            file_name: parsed.file_name,
            model_id: parsed.model_id,
            language: parsed.language,
            created_at_ms: parsed.created_at_ms,
            segment_count: parsed.segments.len(),
            duration_ms,
        });
    }

    summaries.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    Ok(summaries)
}

/// 履歴1件をセグメント込みで読み込む
#[tauri::command]
pub fn load_history(app: AppHandle, id: String) -> Result<HistoryEntry, String> {
    let path = entry_path(&app, &id)?;
    let text =
        fs::read_to_string(&path).map_err(|e| format!("履歴の読み込みに失敗しました: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("履歴の解析に失敗しました: {e}"))
}

/// 履歴1件を削除する
#[tauri::command]
pub fn delete_history(app: AppHandle, id: String) -> Result<(), String> {
    let path = entry_path(&app, &id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("削除に失敗しました: {e}"))?;
    }
    Ok(())
}
