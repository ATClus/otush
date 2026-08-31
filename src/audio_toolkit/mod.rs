pub mod audio;
pub mod constants;
pub mod export;
pub mod lang_id;
pub mod text;
pub mod utils;
pub mod vad;

pub use audio::{
    decode_media_bytes, decode_media_file, is_microphone_access_denied, is_no_input_device_error,
    list_input_devices, list_output_devices, read_wav_samples, resample_audio, save_wav_file,
    verify_wav_file, AudioRecorder, CpalDeviceInfo, DecodedAudio, VadPolicy,
};
pub use export::{
    export_to_json, export_to_markdown, export_to_srt, export_to_txt, export_to_vtt,
    format_timestamp_human, format_timestamp_srt, format_timestamp_vtt, TranscriptDocument,
    TranscriptSegment,
};
pub use lang_id::detect_output_language;
pub use text::{
    apply_custom_words, normalize_transcription_output, remove_filler_words, OutputLanguageEvidence,
};
pub use utils::get_cpal_host;
pub use vad::{EarshotVad, SileroVad, VoiceActivityDetector};
