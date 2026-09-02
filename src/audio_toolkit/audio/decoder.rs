//! Universal media audio decoder for speech-to-text processing.
//!
//! Uses `symphonia` to decode arbitrary media containers (MP3, WAV, AAC, M4A, MP4,
//! FLAC, OGG Vorbis/Opus) and resamples to 16 kHz mono f32 PCM suitable for Whisper
//! and ONNX models.

use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_SAMPLE_RATE: u32 = 16000;

/// Decoded audio information normalized to 16 kHz mono f32 samples.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// 16 kHz mono normalized PCM audio samples [-1.0, 1.0].
    pub samples: Vec<f32>,
    /// Target sample rate (always 16000).
    pub sample_rate: u32,
    /// Total duration in seconds.
    pub duration_secs: f64,
    /// Original audio channel count before mono downmixing.
    pub original_channels: u16,
    /// Original sample rate before resampling.
    pub original_sample_rate: u32,
}

/// Progress reporting callback receiving progress fraction from 0.0 to 1.0.
pub type ProgressCallback = Arc<dyn Fn(f32) + Send + Sync>;

/// Decode an audio or video file from disk into 16 kHz mono PCM samples.
pub fn decode_media_file<P: AsRef<Path>>(
    path: P,
    progress: Option<ProgressCallback>,
) -> Result<DecodedAudio> {
    let path_ref = path.as_ref();
    let file = File::open(path_ref)
        .with_context(|| format!("Failed to open media file: {}", path_ref.display()))?;

    let mut hint = Hint::new();
    if let Some(ext) = path_ref.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    decode_media_source(Box::new(file), hint, progress)
}

/// Decode an in-memory audio byte slice into 16 kHz mono PCM samples.
pub fn decode_media_bytes(
    bytes: Vec<u8>,
    extension_hint: Option<&str>,
    progress: Option<ProgressCallback>,
) -> Result<DecodedAudio> {
    let cursor = std::io::Cursor::new(bytes);
    let mut hint = Hint::new();
    if let Some(ext) = extension_hint {
        hint.with_extension(ext);
    }
    decode_media_source(Box::new(cursor), hint, progress)
}

fn decode_media_source(
    source: Box<dyn MediaSource>,
    hint: Hint,
    progress: Option<ProgressCallback>,
) -> Result<DecodedAudio> {
    let mss = MediaSourceStream::new(source, Default::default());

    let meta_opts = MetadataOptions::default();
    let fmt_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| anyhow!("Unsupported or corrupted audio format: {}", e))?;

    let mut format = probed.format;

    // Find the first supported audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or_else(|| anyhow!("No supported audio track found in media container"))?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let original_sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Audio track missing sample rate information"))?;
    let original_channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(1);
    let total_frames = codec_params.n_frames;

    debug!(
        "Found audio track {}: {} Hz, {} channels, codec {:?}",
        track_id, original_sample_rate, original_channels, codec_params.codec
    );

    let dec_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &dec_opts)
        .map_err(|e| anyhow!("Failed to initialize audio decoder: {}", e))?;

    let mut raw_mono_samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut decoded_frames: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                warn!("Symphonia packet read error: {}", e);
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let num_frames = audio_buf.frames();
                decoded_frames += num_frames as u64;

                let sbuf = sample_buf.get_or_insert_with(|| {
                    SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec)
                });

                sbuf.copy_interleaved_ref(audio_buf);
                let interleaved = sbuf.samples();

                let ch_count = spec.channels.count();
                if ch_count == 1 {
                    raw_mono_samples.extend_from_slice(interleaved);
                } else {
                    // Downmix multi-channel audio to mono (average channels)
                    raw_mono_samples.reserve(interleaved.len() / ch_count);
                    for frame in interleaved.chunks_exact(ch_count) {
                        let sum: f32 = frame.iter().sum();
                        raw_mono_samples.push(sum / ch_count as f32);
                    }
                }

                if let Some(ref cb) = progress {
                    if let Some(total) = total_frames {
                        if total > 0 {
                            let fraction = (decoded_frames as f32 / total as f32).clamp(0.0, 1.0);
                            cb(fraction * 0.5); // 0.0 to 0.5 for decoding stage
                        }
                    }
                }
            }
            Err(SymphoniaError::DecodeError(e)) => {
                warn!("Symphonia decode error (skipping frame): {}", e);
                continue;
            }
            Err(e) => {
                warn!("Fatal Symphonia decode error: {}", e);
                break;
            }
        }
    }

    if raw_mono_samples.is_empty() {
        return Err(anyhow!("Decoded audio is empty (0 samples extracted)"));
    }

    info!(
        "Decoded {} raw mono samples at {} Hz ({} channels originally)",
        raw_mono_samples.len(),
        original_sample_rate,
        original_channels
    );

    // Resample to TARGET_SAMPLE_RATE (16 kHz) if needed
    let final_samples = if original_sample_rate == TARGET_SAMPLE_RATE {
        raw_mono_samples
    } else {
        resample_audio(
            &raw_mono_samples,
            original_sample_rate as usize,
            TARGET_SAMPLE_RATE as usize,
            progress.as_ref(),
        )?
    };

    let duration_secs = final_samples.len() as f64 / TARGET_SAMPLE_RATE as f64;

    if let Some(ref cb) = progress {
        cb(1.0);
    }

    Ok(DecodedAudio {
        samples: final_samples,
        sample_rate: TARGET_SAMPLE_RATE,
        duration_secs,
        original_channels,
        original_sample_rate,
    })
}

/// Resample single-channel f32 audio buffer from `in_hz` to `out_hz` using `FrameResampler`.
pub fn resample_audio(
    input: &[f32],
    in_hz: usize,
    out_hz: usize,
    progress: Option<&ProgressCallback>,
) -> Result<Vec<f32>> {
    if in_hz == out_hz {
        return Ok(input.to_vec());
    }

    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut resampler = crate::audio_toolkit::audio::FrameResampler::new(
        in_hz,
        out_hz,
        std::time::Duration::from_millis(30),
    );

    let expected_len = ((input.len() as f64 * out_hz as f64 / in_hz as f64).round()) as usize;
    let mut output: Vec<f32> = Vec::with_capacity(expected_len + 1024);

    let chunk_size = 4096;
    let total_input = input.len();
    let mut processed = 0;

    for chunk in input.chunks(chunk_size) {
        resampler.push(chunk, |frame| {
            output.extend_from_slice(frame);
        });
        processed += chunk.len();
        if let Some(cb) = progress {
            let frac = 0.5 + (processed as f32 / total_input as f32) * 0.5;
            cb(frac.clamp(0.5, 0.99));
        }
    }

    resampler.finish(|frame| {
        output.extend_from_slice(frame);
    });

    if output.len() > expected_len {
        output.truncate(expected_len);
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_audio_identity() {
        let input = vec![0.1, -0.2, 0.3, -0.4, 0.5];
        let resampled = resample_audio(&input, 16000, 16000, None).unwrap();
        assert_eq!(input, resampled);
    }

    #[test]
    fn test_resample_audio_48k_to_16k() {
        // 48 kHz sine wave for 0.1s (4800 samples)
        let sample_count = 4800;
        let mut input = Vec::with_capacity(sample_count);
        for i in 0..sample_count {
            let t = i as f32 / 48000.0;
            input.push((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5);
        }

        let resampled = resample_audio(&input, 48000, 16000, None).unwrap();
        // 0.1s at 16kHz should be approx 1600 samples
        assert!((resampled.len() as i32 - 1600).abs() < 100);
    }

    #[test]
    fn test_decode_wav_bytes() {
        // Generate a 16kHz WAV in memory using hound
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for i in 0..8000 {
                let sample = ((i as f32 / 16000.0 * 440.0 * 2.0 * std::f32::consts::PI).sin()
                    * i16::MAX as f32
                    * 0.5) as i16;
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
        }

        let wav_bytes = cursor.into_inner();
        let decoded = decode_media_bytes(wav_bytes, Some("wav"), None).unwrap();
        assert_eq!(decoded.sample_rate, 16000);
        assert!((decoded.duration_secs - 0.5).abs() < 0.05);
        assert_eq!(decoded.original_channels, 1);
    }
}
