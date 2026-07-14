use std::fs::File;
use std::path::Path;
use std::process::Command;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Whisper が要求するサンプルレート
const TARGET_RATE: u32 = 16_000;

/// 音声・動画ファイルをデコードして 16kHz モノラルの f32 PCM に変換する。
/// 動画ファイル(mp4 / mov / mkv など)の場合は音声トラックを抽出する。
pub fn decode_to_mono_16k(path: &str) -> Result<Vec<f32>, String> {
    match decode_with_symphonia(path) {
        Ok(audio) => Ok(audio),
        Err(primary_error) => decode_with_ffmpeg(path).map_err(|fallback_error| {
            format!(
                "音声をデコードできませんでした。内蔵デコーダ: {primary_error} / ffmpeg: {fallback_error}"
            )
        }),
    }
}

fn decode_with_symphonia(path: &str) -> Result<Vec<f32>, String> {
    let file = File::open(path).map_err(|e| format!("ファイルを開けませんでした: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| {
            format!("対応していない形式です({e})。webm(Opus 音声)などは未対応です。")
        })?;

    let mut format = probed.format;

    // デコード可能な音声トラックを探す。
    // 動画ファイルでは映像トラックが混ざっているため、
    // 「実際にデコーダを作成できた最初のトラック」を採用する。
    let mut selected: Option<(u32, Box<dyn symphonia::core::codecs::Decoder>, u32)> = None;
    for track in format.tracks() {
        if track.codec_params.codec == CODEC_TYPE_NULL {
            continue;
        }
        if let Ok(decoder) =
            symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())
        {
            let rate = track.codec_params.sample_rate.unwrap_or(0);
            selected = Some((track.id, decoder, rate));
            break;
        }
    }
    let (track_id, mut decoder, mut sample_rate) = selected.ok_or_else(|| {
        "デコード可能な音声トラックが見つかりませんでした。\
         webm(Opus 音声)などは未対応です。"
            .to_string()
    })?;
    let mut mono: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // 終端に達した
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("読み込み中にエラーが発生しました: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                if sample_rate == 0 {
                    sample_rate = spec.rate;
                }
                let channels = spec.channels.count().max(1);

                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                let samples = buf.samples();

                if channels == 1 {
                    mono.extend_from_slice(samples);
                } else {
                    // 多チャンネルは平均してモノラル化
                    for frame in samples.chunks_exact(channels) {
                        mono.push(frame.iter().sum::<f32>() / channels as f32);
                    }
                }
            }
            // 壊れたパケットはスキップして続行
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("デコード中にエラーが発生しました: {e}")),
        }
    }

    if mono.is_empty() {
        return Err("音声データを取り出せませんでした".to_string());
    }
    if sample_rate == 0 {
        return Err("サンプルレートを取得できませんでした".to_string());
    }

    Ok(resample_linear(&mono, sample_rate, TARGET_RATE))
}

/// Symphonia が扱えない WebM/Opus などは ffmpeg で直接 16kHz mono f32 にする。
fn decode_with_ffmpeg(path: &str) -> Result<Vec<f32>, String> {
    let ffmpeg = crate::tools::find_executable("ffmpeg")
        .ok_or_else(|| "ffmpeg が見つかりません".to_string())?;
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-f", "f32le", "-ac", "1", "-ar", "16000", "pipe:1"])
        .output()
        .map_err(|e| format!("起動に失敗しました: {e}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "変換に失敗しました".into()
        } else {
            detail
        });
    }
    let mut audio = Vec::with_capacity(output.stdout.len() / 4);
    for bytes in output.stdout.chunks_exact(4) {
        audio.push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    }
    if audio.is_empty() {
        return Err("音声データを取り出せませんでした".into());
    }
    Ok(audio)
}

/// 線形補間による簡易リサンプリング。
/// 文字起こし用途(音声認識)では十分な品質が得られる。
fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = if idx + 1 < input.len() { input[idx + 1] } else { a };
        out.push(a + (b - a) * frac);
    }
    out
}
