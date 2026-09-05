//! Low-level audio processing toolkit (GUI-agnostic).
//!
//! Submodules: `audio` (cpal capture, decode, DSP, resampling, WAV), `vad`
//! (Silero ONNX and Earshot energy voice-activity detection), `text`
//! (filler-word removal, normalization), `lang_id` (output language
//! detection), and `export` (SRT/VTT/JSON/Markdown transcript export).

pub mod audio;
pub mod constants;
pub mod export;
pub mod lang_id;
pub mod text;
pub mod utils;
pub mod vad;

pub use audio::{
    decode_media_bytes, decode_media_file, is_microphone_access_denied, is_monitor_device,
    is_no_input_device_error, list_input_devices, list_output_devices, list_system_audio_sources,
    read_wav_samples, resample_audio, save_wav_file, verify_wav_file, AudioRecorder,
    CpalDeviceInfo, DecodedAudio, VadPolicy,
};
pub use export::{
    export_to_json, export_to_markdown, export_to_srt, export_to_srt_raw, export_to_txt,
    export_to_vtt, export_to_vtt_raw, format_timestamp_human, format_timestamp_srt,
    format_timestamp_vtt, to_professional_subtitles, wrap_subtitle_lines, SubtitleConfig,
    TranscriptDocument, TranscriptSegment,
};
pub use lang_id::detect_output_language;
pub use text::{
    apply_custom_words, normalize_transcription_output, remove_filler_words, OutputLanguageEvidence,
};
pub use utils::get_cpal_host;
pub use vad::{EarshotVad, SileroVad, VoiceActivityDetector};
