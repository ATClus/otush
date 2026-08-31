#![allow(dead_code)]
use crate::audio_feedback;
use crate::audio_toolkit::audio::{list_input_devices, list_output_devices, AudioRecorder};
use crate::context::AppContext;
use crate::managers::audio::{AudioRecordingManager, MicrophoneMode};
use crate::settings::{get_settings, write_settings};
use log::warn;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub struct CustomSounds {
    start: bool,
    stop: bool,
}

fn custom_sound_exists(ctx: &AppContext, sound_type: &str) -> bool {
    ctx.paths
        .data_dir
        .join(format!("custom_{}.wav", sound_type))
        .exists()
}

/// Check whether custom start/stop audio feedback WAV files exist.
pub fn check_custom_sounds(ctx: &AppContext) -> CustomSounds {
    CustomSounds {
        start: custom_sound_exists(ctx, "start"),
        stop: custom_sound_exists(ctx, "stop"),
    }
}

/// Represents an audio input or output device available on the system.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AudioDevice {
    /// Device identifier or "default".
    pub index: String,
    /// Human-readable device name.
    pub name: String,
    /// Whether this is the system default device.
    pub is_default: bool,
}

/// Configure whether the microphone stream stays active in the background ("always on") or opens on demand.
pub async fn update_microphone_mode(ctx: &AppContext, always_on: bool) -> Result<(), String> {
    // Update settings (fast, stays inline)
    let mut settings = get_settings(ctx);
    settings.always_on_microphone = always_on;
    write_settings(ctx, settings);

    // Update the audio manager mode. update_mode can stop/start the cpal stream
    // (blocking) and takes the manager std mutexes — run it on a blocking
    // thread, NOT inline on the GTK main loop.
    let rm = ctx.audio.clone();
    let new_mode = if always_on {
        MicrophoneMode::AlwaysOn
    } else {
        MicrophoneMode::OnDemand
    };

    crate::runtime::spawn_blocking(move || rm.update_mode(new_mode))
        .await
        .map_err(|e| format!("audio task join failed: {}", e))?
        .map_err(|e| format!("Failed to update microphone mode: {}", e))
}

/// Get the current microphone operation mode (true for always on).
pub fn get_microphone_mode(ctx: &AppContext) -> Result<bool, String> {
    let settings = get_settings(ctx);
    Ok(settings.always_on_microphone)
}

/// Query system audio input devices via cpal.
pub async fn get_available_microphones() -> Result<Vec<AudioDevice>, String> {
    // cpal device enumeration can stall — run it off the GTK main loop.
    crate::runtime::spawn_blocking(|| {
        let devices =
            list_input_devices().map_err(|e| format!("Failed to list audio devices: {}", e))?;

        let mut result = vec![AudioDevice {
            index: "default".to_string(),
            name: "Default".to_string(),
            is_default: true,
        }];

        result.extend(devices.into_iter().map(|d| AudioDevice {
            index: d.index,
            name: d.name,
            is_default: false, // The explicit default is handled separately
        }));

        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| format!("audio task join failed: {}", e))?
}

/// Update and persist the selected audio input microphone.
pub async fn set_selected_microphone(ctx: &AppContext, device_name: String) -> Result<(), String> {
    let mut settings = get_settings(ctx);
    settings.selected_microphone = if device_name == "default" {
        None
    } else {
        Some(device_name)
    };
    write_settings(ctx, settings);

    // Update the audio manager to use the new device. update_selected_device
    // can restart the cpal stream (blocking) — run it on a blocking thread.
    let rm = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || rm.update_selected_device())
        .await
        .map_err(|e| format!("audio task join failed: {}", e))?
        .map_err(|e| format!("Failed to update selected device: {}", e))
}

/// Retrieve the currently selected microphone device name.
pub fn get_selected_microphone(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings
        .selected_microphone
        .unwrap_or_else(|| "default".to_string()))
}

/// Query system audio output devices via cpal.
pub async fn get_available_output_devices() -> Result<Vec<AudioDevice>, String> {
    // cpal device enumeration can stall — run it off the GTK main loop.
    crate::runtime::spawn_blocking(|| {
        let devices =
            list_output_devices().map_err(|e| format!("Failed to list output devices: {}", e))?;

        let mut result = vec![AudioDevice {
            index: "default".to_string(),
            name: "Default".to_string(),
            is_default: true,
        }];

        result.extend(devices.into_iter().map(|d| AudioDevice {
            index: d.index,
            name: d.name,
            is_default: false, // The explicit default is handled separately
        }));

        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| format!("audio task join failed: {}", e))?
}

/// Update and persist the selected audio output device.
pub fn set_selected_output_device(ctx: &AppContext, device_name: String) -> Result<(), String> {
    let mut settings = get_settings(ctx);
    settings.selected_output_device = if device_name == "default" {
        None
    } else {
        Some(device_name)
    };
    write_settings(ctx, settings);
    Ok(())
}

/// Retrieve the currently selected output device name.
pub fn get_selected_output_device(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings
        .selected_output_device
        .unwrap_or_else(|| "default".to_string()))
}

/// Play a test audio feedback sound asynchronously.
pub async fn play_test_sound(ctx: &AppContext, sound_type: String) {
    let sound = match sound_type.as_str() {
        "start" => audio_feedback::SoundType::Start,
        "stop" => audio_feedback::SoundType::Stop,
        _ => {
            warn!("Unknown sound type: {}", sound_type);
            return;
        }
    };
    audio_feedback::play_test_sound(ctx, sound);
}

pub fn set_clamshell_microphone(ctx: &AppContext, device_name: String) -> Result<(), String> {
    let mut settings = get_settings(ctx);
    settings.clamshell_microphone = if device_name == "default" {
        None
    } else {
        Some(device_name)
    };
    write_settings(ctx, settings);
    Ok(())
}

pub fn get_clamshell_microphone(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings
        .clamshell_microphone
        .unwrap_or_else(|| "default".to_string()))
}

pub fn is_recording(ctx: &AppContext) -> bool {
    ctx.audio.is_recording()
}

pub async fn get_microphone_channels(device_name: String) -> Result<u16, String> {
    // cpal device enumeration and config queries can stall, so keep them off
    // the GTK main loop.
    crate::runtime::spawn_blocking(move || {
        use cpal::traits::HostTrait;

        let device = if device_name.eq_ignore_ascii_case("default") {
            crate::audio_toolkit::get_cpal_host().default_input_device()
        } else {
            list_input_devices()
                .map_err(|e| format!("Failed to list audio devices: {e}"))?
                .into_iter()
                .find(|device| device.name == device_name)
                .map(|device| device.device)
        };

        match device {
            Some(device) => AudioRecorder::preferred_input_channel_count(&device)
                .map_err(|e| format!("Failed to get microphone config: {e}")),
            None => Ok(1),
        }
    })
    .await
    .map_err(|e| format!("audio task join failed: {e}"))?
}

pub async fn set_selected_channel(ctx: &AppContext, channel: Option<u16>) -> Result<(), String> {
    // Restarting cpal can block, so keep it off the GTK main loop. Apply
    // the runtime change before persisting it so a rejected active-recording
    // change does not become effective on the next launch.
    let manager = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || manager.update_selected_channel(channel))
        .await
        .map_err(|e| format!("audio task join failed: {e}"))?
        .map_err(|e| format!("Failed to update channel selection: {e}"))?;

    let mut settings = get_settings(ctx);
    settings.selected_channel = channel;
    write_settings(ctx, settings);
    Ok(())
}

static ACTIVE_MONITOR_STREAM: std::sync::Mutex<Option<cpal::Stream>> = std::sync::Mutex::new(None);

pub fn start_mic_monitor(ctx: &AppContext) {
    stop_mic_monitor();

    let settings = get_settings(ctx);
    let selected_mic = settings.selected_microphone;
    let gain = settings.audio_input_gain;
    let bus = ctx.bus.clone();

    crate::runtime::spawn(async move {
        let _ = crate::runtime::spawn_blocking(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
            let host = crate::audio_toolkit::get_cpal_host();
            let device = match selected_mic {
                Some(ref name) => {
                    let devices = list_input_devices().map_err(|e| format!("{e}"))?;
                    devices
                        .into_iter()
                        .find(|d| &d.name == name)
                        .map(|d| d.device)
                        .or_else(|| host.default_input_device())
                }
                None => host.default_input_device(),
            }
            .ok_or_else(|| "No input device available".to_string())?;

            let config = device
                .default_input_config()
                .map_err(|e| format!("Failed to get input config: {e}"))?;

            let sample_format = config.sample_format();
            let stream_config: cpal::StreamConfig = config.into();

            let last_emit = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let last_emit_f32 = last_emit.clone();
            let last_emit_i16 = last_emit;
            let bus_f32 = bus.clone();
            let bus_i16 = bus;

            let err_fn = |err| log::warn!("Mic monitor stream error: {err}");

            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        if now.saturating_sub(
                            last_emit_f32.load(std::sync::atomic::Ordering::Relaxed),
                        ) >= 33
                        {
                            last_emit_f32.store(now, std::sync::atomic::Ordering::Relaxed);
                            let peak = data.iter().copied().fold(0.0f32, |a, b| a.max(b.abs()));
                            bus_f32
                                .send(crate::context::AppEvent::MicLevel((peak * gain).min(1.0)));
                        }
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        if now.saturating_sub(
                            last_emit_i16.load(std::sync::atomic::Ordering::Relaxed),
                        ) >= 33
                        {
                            last_emit_i16.store(now, std::sync::atomic::Ordering::Relaxed);
                            let peak = data
                                .iter()
                                .copied()
                                .fold(0.0f32, |a, b| a.max((b as f32 / i16::MAX as f32).abs()));
                            bus_i16
                                .send(crate::context::AppEvent::MicLevel((peak * gain).min(1.0)));
                        }
                    },
                    err_fn,
                    None,
                ),
                _ => return Err("Unsupported sample format".to_string()),
            }
            .map_err(|e| format!("Failed to build monitor stream: {e}"))?;

            stream
                .play()
                .map_err(|e| format!("Failed to play monitor stream: {e}"))?;
            if let Ok(mut guard) = ACTIVE_MONITOR_STREAM.lock() {
                *guard = Some(stream);
            }
            Ok::<_, String>(())
        })
        .await;
    });
}

pub fn stop_mic_monitor() {
    if let Ok(mut guard) = ACTIVE_MONITOR_STREAM.lock() {
        if let Some(stream) = guard.take() {
            drop(stream);
        }
    }
}

// Keep the type alias used by callers that pass the audio manager around.
pub type AudioManagerRef = Arc<AudioRecordingManager>;
