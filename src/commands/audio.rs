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

pub fn check_custom_sounds(ctx: &AppContext) -> CustomSounds {
    CustomSounds {
        start: custom_sound_exists(ctx, "start"),
        stop: custom_sound_exists(ctx, "stop"),
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AudioDevice {
    pub index: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAccess {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WindowsMicrophonePermissionStatus {
    pub supported: bool,
    pub overall_access: PermissionAccess,
    pub device_access: PermissionAccess,
    pub app_access: PermissionAccess,
    pub desktop_app_access: PermissionAccess,
}

pub fn get_windows_microphone_permission_status() -> WindowsMicrophonePermissionStatus {
    // Linux (GNOME/PipeWire): microphone access is granted by the portal at
    // capture time; there is no registry-style permission to query.
    WindowsMicrophonePermissionStatus {
        supported: false,
        overall_access: PermissionAccess::Unknown,
        device_access: PermissionAccess::Unknown,
        app_access: PermissionAccess::Unknown,
        desktop_app_access: PermissionAccess::Unknown,
    }
}

pub fn open_microphone_privacy_settings() -> Result<(), String> {
    Err("Opening microphone privacy settings is only supported on Windows".to_string())
}

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

pub fn get_microphone_mode(ctx: &AppContext) -> Result<bool, String> {
    let settings = get_settings(ctx);
    Ok(settings.always_on_microphone)
}

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

pub fn get_selected_microphone(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings
        .selected_microphone
        .unwrap_or_else(|| "default".to_string()))
}

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

pub fn get_selected_output_device(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings
        .selected_output_device
        .unwrap_or_else(|| "default".to_string()))
}

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

// Keep the type alias used by callers that pass the audio manager around.
pub type AudioManagerRef = Arc<AudioRecordingManager>;
