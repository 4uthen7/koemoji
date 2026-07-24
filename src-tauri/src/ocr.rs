// @4uthent / tkmt_wonderkid

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::tools::find_executable;
use crate::transcribe::Segment;

const OCR_LANGUAGES: &str = "jpn+eng";

/// OCR の1スナップショット（フレーム間隔ごとに記録）。
/// 累積テキスト出力と、スライド切り替えの検出に使う。
#[derive(Serialize, Clone)]
pub struct OcrSnapshot {
    pub start_ms: i64,
    pub text: String,
    pub is_new_slide: bool,
}

#[derive(Serialize)]
pub struct OcrSupport {
    pub available: bool,
    pub ffmpeg_found: bool,
    pub tesseract_found: bool,
    pub japanese_found: bool,
    pub english_found: bool,
    pub message: String,
}

#[derive(Serialize, Clone)]
struct StatusPayload {
    stage: String,
}

/// ffmpeg / Tesseract / 日英学習データが利用できるかを UI 向けに返す。
#[tauri::command]
pub fn check_ocr_support() -> OcrSupport {
    let ffmpeg = find_executable("ffmpeg");
    let tesseract = find_executable("tesseract");

    let mut langs = HashSet::new();
    if let Some(path) = &tesseract {
        if let Ok(output) = Command::new(path).arg("--list-langs").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            langs.extend(
                stdout
                    .lines()
                    .chain(stderr.lines())
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string),
            );
        }
    }

    let japanese_found = langs.contains("jpn");
    let english_found = langs.contains("eng");
    let available = ffmpeg.is_some()
        && tesseract.is_some()
        && japanese_found
        && english_found;

    let message = if available {
        "画面OCRを利用できます(Tesseract: 日本語 + 英語)".to_string()
    } else {
        let mut missing = Vec::new();
        if ffmpeg.is_none() {
            missing.push("ffmpeg");
        }
        if tesseract.is_none() {
            missing.push("Tesseract");
        } else {
            if !japanese_found {
                missing.push("Tesseract日本語データ(jpn)");
            }
            if !english_found {
                missing.push("Tesseract英語データ(eng)");
            }
        }
        format!(
            "画面OCRには {} が必要です。インストール後にアプリを再起動してください。",
            missing.join(" / ")
        )
    };

    OcrSupport {
        available,
        ffmpeg_found: ffmpeg.is_some(),
        tesseract_found: tesseract.is_some(),
        japanese_found,
        english_found,
        message,
    }
}

pub fn ensure_ocr_support() -> Result<(), String> {
    let support = check_ocr_support();
    if support.available {
        Ok(())
    } else {
        Err(support.message)
    }
}

/// 動画を一定間隔で PNG にし、Tesseract (jpn+eng) で文字を抽出する。
/// 直前フレームと同じ行は落とし、画面の差分だけをタイムラインに残す。
pub fn extract_text_segments(
    app: &AppHandle,
    input_path: &str,
    interval_secs: u32,
    cancel: &Arc<AtomicBool>,
) -> Result<(Vec<Segment>, Vec<OcrSnapshot>), String> {
    let interval_secs = interval_secs.clamp(1, 60);
    let ffmpeg = find_executable("ffmpeg")
        .ok_or_else(|| "ffmpeg が見つかりません。画面OCRを利用できません。".to_string())?;
    let tesseract = find_executable("tesseract")
        .ok_or_else(|| "Tesseract が見つかりません。画面OCRを利用できません。".to_string())?;

    let frames_dir = TempFramesDir::new()?;
    let output_pattern = frames_dir.path().join("frame-%08d.png");

    let _ = app.emit(
        "transcribe-status",
        StatusPayload {
            stage: "extracting_frames".into(),
        },
    );
    let fps_filter = format!("fps=1/{interval_secs}");
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input_path)
        .args(["-vf", &fps_filter, "-vsync", "vfr"])
        .arg(&output_pattern)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg の起動に失敗しました: {e}"))?;

    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("キャンセルされました".into());
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("フレーム抽出の待機中にエラーが発生しました: {e}")),
        }
    }

    let mut frames: Vec<PathBuf> = fs::read_dir(frames_dir.path())
        .map_err(|e| format!("抽出フレームを読み込めませんでした: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    frames.sort();

    // 音声だけのファイルには映像ストリームがないため、OCR結果なしで正常終了する。
    if frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let _ = app.emit(
        "transcribe-status",
        StatusPayload {
            stage: "ocr_running".into(),
        },
    );

    let mut segments = Vec::new();
    let mut snapshots: Vec<OcrSnapshot> = Vec::new();
    let mut previous_lines: Vec<String> = Vec::new();
    let total = frames.len();

    for (index, frame) in frames.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err("キャンセルされました".into());
        }

        let output = Command::new(&tesseract)
            .arg(frame)
            .arg("stdout")
            .args(["-l", OCR_LANGUAGES, "--psm", "6"])
            .output()
            .map_err(|e| format!("Tesseract の実行に失敗しました: {e}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if detail.is_empty() {
                "Tesseract OCR に失敗しました".into()
            } else {
                format!("Tesseract OCR に失敗しました: {detail}")
            });
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        let current_lines = clean_lines(&raw);
        let novel_lines = remove_repeated_lines(&current_lines, &previous_lines);
        previous_lines = current_lines;

        if !novel_lines.is_empty() {
            let start_ms = index as i64 * interval_secs as i64 * 1000;
            let is_new_slide = novel_lines.len() == current_lines.len()
                || (novel_lines.len() * 2 > current_lines.len());
            segments.push(Segment {
                start_ms,
                end_ms: start_ms + interval_secs as i64 * 1000,
                text: novel_lines.join("\n"),
                source: "ocr".into(),
            });
            snapshots.push(OcrSnapshot {
                start_ms,
                text: if is_new_slide { current_lines.join("\n") } else { novel_lines.join("\n") },
                is_new_slide,
            });
        }

        let progress = ((index + 1) * 100 / total) as i32;
        let _ = app.emit("transcribe-progress", progress);
    }

    Ok((segments, snapshots))
}

fn clean_lines(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for line in text.lines() {
        let clean = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.chars().count() < 2 {
            continue;
        }
        if !result.iter().any(|existing| lines_are_similar(existing, &clean)) {
            result.push(clean);
        }
    }
    result
}

fn remove_repeated_lines(current: &[String], previous: &[String]) -> Vec<String> {
    // 画面全体が切り替わった場合は、その画面の文脈を保つため全文を採用する。
    let repeated = current
        .iter()
        .filter(|line| previous.iter().any(|old| lines_are_similar(line, old)))
        .count();
    if !current.is_empty() && repeated * 2 < current.len() {
        return current.to_vec();
    }

    current
        .iter()
        .filter(|line| !previous.iter().any(|old| lines_are_similar(line, old)))
        .cloned()
        .collect()
}

fn lines_are_similar(a: &str, b: &str) -> bool {
    let a = normalize_for_compare(a);
    let b = normalize_for_compare(b);
    if a == b {
        return true;
    }
    let a_len = a.chars().count();
    let b_len = b.chars().count();
    if a_len < 4 || b_len < 4 {
        return false;
    }
    let distance = levenshtein(&a, &b);
    let max_len = a_len.max(b_len);
    1.0 - distance as f64 / max_len as f64 >= 0.90
}

fn normalize_for_compare(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .flat_map(char::to_lowercase)
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ac) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, bc) in b.iter().enumerate() {
            let cost = usize::from(ac != bc);
            current[j + 1] = (current[j] + 1)
                .min(previous[j + 1] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// 累積 OCR テキストを Markdown 調の読みやすい形式に整形する。
/// スライド切り替え時は区切りを入れて全文を、差分のみのときは追加行だけを表示する。
pub fn format_cumulative_ocr_text(
    snapshots: &[OcrSnapshot],
    source_name: &str,
) -> String {
    let generated_at = chrono_now();
    let mut out = String::new();
    out.push_str(&format!("# OCR Slide Text: {source_name}\n"));
    out.push_str(&format!("# Generated: {generated_at}\n"));
    out.push_str("\n---\n\n");

    for snap in snapshots {
        let secs = snap.start_ms / 1000;
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        let ts = format!("{h:02}:{m:02}:{s:02}");

        if snap.is_new_slide {
            out.push_str(&format!("## [{ts}] ← Slide\n"));
            for line in snap.text.lines() {
                out.push_str(&format!("{line}\n"));
            }
        } else {
            out.push_str(&format!("## [{ts}] +\n"));
            for line in snap.text.lines() {
                out.push_str(&format!("+ {line}\n"));
            }
        }
        out.push_str("\n---\n\n");
    }

    out
}



fn chrono_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs() as i64 + 9 * 3600; // JST
    let ss = total_secs % 60;
    let total_mins = total_secs / 60;
    let mm = total_mins % 60;
    let total_hours = total_mins / 60;
    let hh = total_hours % 24;
    let total_days = total_hours / 24;

    // 年を計算（うるう年対応）
    let mut y = 1970i64;
    let mut remaining = total_days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let mon_lengths = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut mon = 1usize;
    for &days in mon_lengths.iter() {
        if remaining < days {
            break;
        }
        remaining -= days;
        mon += 1;
    }
    let d = remaining + 1;

    format!("{y:04}-{mon:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

struct TempFramesDir(PathBuf);

impl TempFramesDir {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("一時フォルダ名を作成できませんでした: {e}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("koemoji-ocr-{nonce}"));
        fs::create_dir_all(&path)
            .map_err(|e| format!("OCR用一時フォルダを作成できませんでした: {e}"))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFramesDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_ocr_lines_are_removed() {
        let previous = vec!["会議のアジェンダ".into(), "Project roadmap".into()];
        let current = vec![
            "会議のアジェンダ".into(),
            "Project roadmap".into(),
            "次のステップ".into(),
        ];
        assert_eq!(remove_repeated_lines(&current, &previous), vec!["次のステップ"]);
    }

    #[test]
    fn a_new_screen_keeps_its_context() {
        let previous = vec!["Old title".into(), "Old body".into()];
        let current = vec!["New title".into(), "新しい本文".into(), "Conclusion".into()];
        assert_eq!(remove_repeated_lines(&current, &previous), current);
    }

    #[test]
    fn small_ocr_variations_count_as_duplicates() {
        assert!(lines_are_similar(
            "KoeMoji timeline integration",
            "KoeMoji timeline integratlon"
        ));
    }
}
