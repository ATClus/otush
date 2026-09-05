//! Audio capture and file decoding.
//!
//! The cpal input stream owner plus media probing via symphonia (resampled to
//! 16 kHz mono), the voice-enhancement DSP chain (high-pass, noise gate),
//! and the overlay level-meter visualizer.

pub mod decoder;
mod device;
pub mod dsp;
mod recorder;
mod resampler;
mod utils;
mod visualizer;

pub use decoder::{
    decode_media_bytes, decode_media_file, resample_audio, DecodedAudio, ProgressCallback,
    TARGET_SAMPLE_RATE,
};
pub use device::{
    is_monitor_device, list_input_devices, list_output_devices, list_system_audio_sources,
    CpalDeviceInfo,
};
pub use dsp::{HighPassFilter, NoiseGate, VoiceEnhancer, VoiceEnhancerConfig};
pub use recorder::{
    is_microphone_access_denied, is_no_input_device_error, AudioRecorder, VadPolicy,
};
pub use resampler::FrameResampler;
pub use utils::{read_wav_samples, save_wav_file, verify_wav_file};
pub use visualizer::AudioVisualiser;
