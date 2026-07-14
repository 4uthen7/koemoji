use std::fs;
use std::io::Write;
use std::path::PathBuf;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// 利用可能な Whisper モデル一覧: (id, 説明, おおよそのサイズ MB)
/// ggml 形式のモデルは Hugging Face の ggerganov/whisper.cpp から取得する
const MODELS: &[(&str, &str, u64)] = &[
    ("tiny", "最小・最速(精度は低め)", 78),
    ("base", "軽量・高速", 148),
    ("small", "バランス型(まずはこれ)", 488),
    ("medium", "高精度(やや重い)", 1530),
    ("large-v3-turbo", "高精度かつ高速化版(おすすめ)", 1620),
    ("large-v3", "最高精度(重い)", 3100),
];

#[derive(Serialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub description: String,
    pub size_mb: u64,
    pub downloaded: bool,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    id: String,
    downloaded: u64,
    total: u64,
}

/// モデル保存先ディレクトリ(アプリデータフォルダ配下)
fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("アプリデータフォルダを取得できませんでした: {e}"))?
        .join("models");
    fs::create_dir_all(&dir).map_err(|e| format!("フォルダの作成に失敗しました: {e}"))?;
    Ok(dir)
}

/// 指定モデルのファイルパスを返す(存在チェックはしない)
pub fn model_path(app: &AppHandle, model_id: &str) -> Result<PathBuf, String> {
    if !MODELS.iter().any(|(id, _, _)| *id == model_id) {
        return Err(format!("不明なモデルです: {model_id}"));
    }
    Ok(models_dir(app)?.join(format!("ggml-{model_id}.bin")))
}

#[tauri::command]
pub fn list_models(app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    let dir = models_dir(&app)?;
    Ok(MODELS
        .iter()
        .map(|(id, desc, size)| ModelInfo {
            id: (*id).to_string(),
            description: (*desc).to_string(),
            size_mb: *size,
            downloaded: dir.join(format!("ggml-{id}.bin")).exists(),
        })
        .collect())
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_id: String) -> Result<(), String> {
    let final_path = model_path(&app, &model_id)?;
    if final_path.exists() {
        return Ok(());
    }
    let part_path = final_path.with_extension("bin.part");

    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model_id}.bin"
    );

    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("ダウンロードを開始できませんでした: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "ダウンロードに失敗しました(HTTP {})",
            response.status()
        ));
    }

    let total = response.content_length().unwrap_or(0);
    let mut file =
        fs::File::create(&part_path).map_err(|e| format!("ファイルを作成できませんでした: {e}"))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emitted_mb: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = fs::remove_file(&part_path);
                return Err(format!("ダウンロード中にエラーが発生しました: {e}"));
            }
        };
        if let Err(e) = file.write_all(&chunk) {
            let _ = fs::remove_file(&part_path);
            return Err(format!("書き込みに失敗しました: {e}"));
        }
        downloaded += chunk.len() as u64;

        // 1MB ごとに進捗イベントを送る(イベント洪水を防ぐ)
        let mb = downloaded >> 20;
        if mb != last_emitted_mb {
            last_emitted_mb = mb;
            let _ = app.emit(
                "model-download-progress",
                DownloadProgress {
                    id: model_id.clone(),
                    downloaded,
                    total,
                },
            );
        }
    }

    file.flush().map_err(|e| format!("書き込みに失敗しました: {e}"))?;
    drop(file);
    fs::rename(&part_path, &final_path)
        .map_err(|e| format!("ファイルの確定に失敗しました: {e}"))?;

    // 完了イベント(バーを確実に 100% にする。total 不明時にも対応)
    let _ = app.emit(
        "model-download-progress",
        DownloadProgress {
            id: model_id.clone(),
            downloaded,
            total: if total > 0 { total } else { downloaded },
        },
    );
    Ok(())
}

#[tauri::command]
pub fn delete_model(app: AppHandle, model_id: String) -> Result<(), String> {
    let path = model_path(&app, &model_id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("削除に失敗しました: {e}"))?;
    }
    Ok(())
}
