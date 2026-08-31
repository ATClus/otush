use crate::context::AppContext;
use crate::settings::{self, AppSettings, SoundTheme};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, warn};
use rodio::OutputStreamBuilder;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::thread;

pub enum SoundType {
    Start,
    Stop,
}

fn resolve_sound_path(
    ctx: &AppContext,
    settings: &AppSettings,
    sound_type: SoundType,
) -> Option<PathBuf> {
    let sound_file = get_sound_path(settings, sound_type);
    match settings.sound_theme {
        SoundTheme::Custom => Some(ctx.paths.data_dir.join(sound_file)),
        _ => Some(ctx.paths.resource_dir.join(sound_file)),
    }
}

fn get_sound_path(settings: &AppSettings, sound_type: SoundType) -> String {
    match (settings.sound_theme, sound_type) {
        (SoundTheme::Custom, SoundType::Start) => "custom_start.wav".to_string(),
        (SoundTheme::Custom, SoundType::Stop) => "custom_stop.wav".to_string(),
        (_, SoundType::Start) => settings.sound_theme.to_start_path(),
        (_, SoundType::Stop) => settings.sound_theme.to_stop_path(),
    }
}

pub fn play_feedback_sound(ctx: &AppContext, sound_type: SoundType) {
    let settings = settings::get_settings(ctx);
    if !settings.audio_feedback {
        return;
    }
    if let Some(path) = resolve_sound_path(ctx, &settings, sound_type) {
        play_sound_async(ctx, path);
    }
}

pub fn play_feedback_sound_blocking(ctx: &AppContext, sound_type: SoundType) {
    let settings = settings::get_settings(ctx);
    if !settings.audio_feedback {
        return;
    }
    if let Some(path) = resolve_sound_path(ctx, &settings, sound_type) {
        play_sound_blocking(ctx, &path);
    }
}

#[allow(dead_code)]
pub fn play_test_sound(ctx: &AppContext, sound_type: SoundType) {
    let settings = settings::get_settings(ctx);
    if let Some(path) = resolve_sound_path(ctx, &settings, sound_type) {
        play_sound_blocking(ctx, &path);
    }
}

fn play_sound_async(ctx: &AppContext, path: PathBuf) {
    let ctx = ctx.clone();
    thread::spawn(move || {
        if let Err(e) = play_sound_at_path(&ctx, path.as_path()) {
            error!("Failed to play sound '{}': {}", path.display(), e);
        }
    });
}

fn play_sound_blocking(ctx: &AppContext, path: &Path) {
    if let Err(e) = play_sound_at_path(ctx, path) {
        error!("Failed to play sound '{}': {}", path.display(), e);
    }
}

fn play_sound_at_path(ctx: &AppContext, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let settings = settings::get_settings(ctx);
    let volume = settings.audio_feedback_volume;
    let selected_device = settings.selected_output_device;
    play_audio_file(path, selected_device, volume)
}

fn play_audio_file(
    path: &std::path::Path,
    selected_device: Option<String>,
    volume: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let stream_builder = if let Some(device_name) = selected_device {
        if device_name == "Default" {
            debug!("Using default device");
            OutputStreamBuilder::from_default_device()?
        } else {
            let host = crate::audio_toolkit::get_cpal_host();
            let devices = host.output_devices()?;

            let mut found_device = None;
            for device in devices {
                if device.name()? == device_name {
                    found_device = Some(device);
                    break;
                }
            }

            match found_device {
                Some(device) => OutputStreamBuilder::from_device(device)?,
                None => {
                    warn!("Device '{}' not found, using default device", device_name);
                    OutputStreamBuilder::from_default_device()?
                }
            }
        }
    } else {
        debug!("Using default device");
        OutputStreamBuilder::from_default_device()?
    };

    let stream_handle = stream_builder.open_stream()?;
    let mixer = stream_handle.mixer();

    let file = File::open(path)?;
    let buf_reader = BufReader::new(file);

    let sink = rodio::play(mixer, buf_reader)?;
    sink.set_volume(volume);
    sink.sleep_until_end();

    Ok(())
}
