//! Audio device management and microphone monitoring commands.

use super::errors::{CommandError, CommandResult};
use crate::audio_toolkit::audio::list_input_devices;
use crate::context::AppContext;
use crate::managers::audio::MicrophoneMode;
use crate::settings::{get_settings, write_settings};
use serde::{Deserialize, Serialize};

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
pub async fn update_microphone_mode(ctx: &AppContext, always_on: bool) -> CommandResult<()> {
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
        .map_err(CommandError::task_join)?
        .map_err(|e| CommandError::MicrophoneMode(e.to_string()))
}

/// Query system audio input devices via cpal.
pub async fn get_available_microphones() -> CommandResult<Vec<AudioDevice>> {
    // cpal device enumeration can stall — run it off the GTK main loop.
    crate::runtime::spawn_blocking(|| {
        let devices = list_input_devices().map_err(CommandError::list_devices)?;

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

        Ok::<_, CommandError>(result)
    })
    .await
    .map_err(CommandError::task_join)?
}

/// Update and persist the selected audio input microphone.
pub async fn set_selected_microphone(ctx: &AppContext, device_name: String) -> CommandResult<()> {
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
        .map_err(CommandError::task_join)?
        .map_err(|e| CommandError::SelectedDevice(e.to_string()))
}

/// Query available system audio / desktop output monitor sources for live meeting capture.
pub async fn get_available_system_audio_sources() -> CommandResult<Vec<AudioDevice>> {
    crate::runtime::spawn_blocking(|| {
        let devices = crate::audio_toolkit::list_system_audio_sources()
            .map_err(CommandError::list_system_sources)?;

        let mut result = vec![AudioDevice {
            index: "default".to_string(),
            name: "Default System Audio Monitor".to_string(),
            is_default: true,
        }];

        result.extend(devices.into_iter().map(|d| AudioDevice {
            index: d.index,
            name: d.name,
            is_default: false,
        }));

        Ok::<_, CommandError>(result)
    })
    .await
    .map_err(CommandError::task_join)?
}

/// Update and persist the audio capture source mode (MicrophoneOnly, SystemAudioOnly, Mixed).
pub async fn set_audio_capture_source(
    ctx: &AppContext,
    source: crate::settings::AudioCaptureSource,
) -> CommandResult<()> {
    let mut settings = get_settings(ctx);
    settings.audio_capture_source = source;
    write_settings(ctx, settings);

    let rm = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || rm.update_selected_device())
        .await
        .map_err(CommandError::task_join)?
        .map_err(|e| CommandError::CaptureSource(e.to_string()))
}

/// Update and persist the selected system audio loopback device.
pub async fn set_selected_system_audio_device(
    ctx: &AppContext,
    device_name: String,
) -> CommandResult<()> {
    let mut settings = get_settings(ctx);
    let trimmed = device_name.trim();
    settings.selected_system_audio_device = if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("default")
        || trimmed == "Default System Audio Monitor"
    {
        None
    } else {
        Some(trimmed.to_string())
    };
    write_settings(ctx, settings);

    let rm = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || rm.update_selected_device())
        .await
        .map_err(CommandError::task_join)?
        .map_err(|e| CommandError::SystemAudioDevice(e.to_string()))
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
                    let devices = list_input_devices()
                        .map_err(|e| CommandError::AudioDevices(format!("{e}")))?;
                    devices
                        .into_iter()
                        .find(|d| &d.name == name)
                        .map(|d| d.device)
                        .or_else(|| host.default_input_device())
                }
                None => host.default_input_device(),
            }
            .ok_or(CommandError::NoInputDevice)?;

            let config = device
                .default_input_config()
                .map_err(CommandError::input_config)?;

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
                _ => return Err(CommandError::UnsupportedSampleFormat),
            }
            .map_err(CommandError::build_monitor_stream)?;

            stream.play().map_err(CommandError::play_monitor_stream)?;
            if let Ok(mut guard) = ACTIVE_MONITOR_STREAM.lock() {
                *guard = Some(stream);
            }
            Ok::<_, CommandError>(())
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
