use cpal::traits::{DeviceTrait, HostTrait};

pub struct CpalDeviceInfo {
    pub index: String,
    pub name: String,
    pub is_default: bool,
    pub device: cpal::Device,
}

pub fn list_input_devices() -> Result<Vec<CpalDeviceInfo>, Box<dyn std::error::Error>> {
    let host = crate::audio_toolkit::get_cpal_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut out = Vec::<CpalDeviceInfo>::new();

    for (index, device) in host.input_devices()?.enumerate() {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());

        let is_default = Some(name.clone()) == default_name;

        out.push(CpalDeviceInfo {
            index: index.to_string(),
            name,
            is_default,
            device,
        });
    }

    Ok(out)
}

pub fn list_output_devices() -> Result<Vec<CpalDeviceInfo>, Box<dyn std::error::Error>> {
    let host = crate::audio_toolkit::get_cpal_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());

    let mut out = Vec::<CpalDeviceInfo>::new();

    for (index, device) in host.output_devices()?.enumerate() {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());

        let is_default = Some(name.clone()) == default_name;

        out.push(CpalDeviceInfo {
            index: index.to_string(),
            name,
            is_default,
            device,
        });
    }

    Ok(out)
}

/// Check whether an input device name indicates a desktop/system audio monitor or loopback.
pub fn is_monitor_device(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("monitor")
        || lower.contains("loopback")
        || lower.contains("stereo mix")
        || lower.contains("what u hear")
}

/// List all system audio / desktop output monitor devices for meeting participant capture.
pub fn list_system_audio_sources() -> Result<Vec<CpalDeviceInfo>, Box<dyn std::error::Error>> {
    let all_inputs = list_input_devices()?;
    let mut monitors: Vec<CpalDeviceInfo> = all_inputs
        .into_iter()
        .filter(|d| is_monitor_device(&d.name))
        .collect();

    // If no explicit monitor device is listed by ALSA/Pulse, provide all output names as monitor hints
    if monitors.is_empty() {
        if let Ok(outputs) = list_output_devices() {
            for out_dev in outputs {
                monitors.push(CpalDeviceInfo {
                    index: format!("mon_{}", out_dev.index),
                    name: format!("Monitor of {}", out_dev.name),
                    is_default: out_dev.is_default,
                    device: out_dev.device,
                });
            }
        }
    }

    Ok(monitors)
}
