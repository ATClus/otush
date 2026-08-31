//! Subtitle, transcript, and meeting notes exporters.
//!
//! Provides conversion to standard industry formats:
//! - SubRip (.srt)
//! - WebVTT (.vtt)
//! - Plain Text with timestamps (.txt)
//! - Structured JSON (.json)
//! - Markdown meeting notes (.md)

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A single timestamped speech segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    /// 1-indexed sequential segment ID.
    pub id: usize,
    /// Start timestamp in milliseconds.
    pub start_ms: u64,
    /// End timestamp in milliseconds.
    pub end_ms: u64,
    /// Transcribed text content.
    pub text: String,
    /// Optional speaker name or tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// Full transcription document containing metadata, segments, and optional AI summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptDocument {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub duration_secs: f64,
    pub created_at_unix: i64,
    pub segments: Vec<TranscriptSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_or_post_processed: Option<String>,
}

/// Format milliseconds into SRT timestamp format (`HH:MM:SS,mmm`).
pub fn format_timestamp_srt(ms: u64) -> String {
    let total_secs = ms / 1000;
    let millis = ms % 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}:{:02},{:03}", hours, mins, secs, millis)
}

/// Format milliseconds into WebVTT timestamp format (`HH:MM:SS.mmm`).
pub fn format_timestamp_vtt(ms: u64) -> String {
    let total_secs = ms / 1000;
    let millis = ms % 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}:{:02}.{:03}", hours, mins, secs, millis)
}

/// Format milliseconds into short human readable timestamp (`[MM:SS]` or `[HH:MM:SS]`).
pub fn format_timestamp_human(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("[{:02}:{:02}:{:02}]", hours, mins, secs)
    } else {
        format!("[{:02}:{:02}]", mins, secs)
    }
}

/// Export segments as SubRip (.srt) subtitle string.
pub fn export_to_srt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let idx = i + 1;
        let start = format_timestamp_srt(seg.start_ms);
        let end = format_timestamp_srt(seg.end_ms.max(seg.start_ms + 100));
        let text = seg.text.trim();
        if let Some(ref speaker) = seg.speaker {
            out.push_str(&format!(
                "{}\n{} --> {}\n[{}] {}\n\n",
                idx, start, end, speaker, text
            ));
        } else {
            out.push_str(&format!("{}\n{} --> {}\n{}\n\n", idx, start, end, text));
        }
    }
    out
}

/// Export segments as WebVTT (.vtt) subtitle string.
pub fn export_to_vtt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for (i, seg) in segments.iter().enumerate() {
        let idx = i + 1;
        let start = format_timestamp_vtt(seg.start_ms);
        let end = format_timestamp_vtt(seg.end_ms.max(seg.start_ms + 100));
        let text = seg.text.trim();
        if let Some(ref speaker) = seg.speaker {
            out.push_str(&format!(
                "{}\n{} --> {}\n<v {}>{}",
                idx, start, end, speaker, text
            ));
        } else {
            out.push_str(&format!("{}\n{} --> {}\n{}", idx, start, end, text));
        }
        out.push_str("\n\n");
    }
    out
}

/// Export segments as Plain Text (.txt), with optional timestamps and speaker tags.
pub fn export_to_txt(segments: &[TranscriptSegment], include_timestamps: bool) -> String {
    let mut out = String::new();
    for seg in segments {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        if include_timestamps {
            let ts = format_timestamp_human(seg.start_ms);
            if let Some(ref speaker) = seg.speaker {
                out.push_str(&format!("{} {}: {}\n", ts, speaker, text));
            } else {
                out.push_str(&format!("{} {}\n", ts, text));
            }
        } else {
            if let Some(ref speaker) = seg.speaker {
                out.push_str(&format!("{}: {}\n", speaker, text));
            } else {
                out.push_str(&format!("{}\n", text));
            }
        }
    }
    out
}

/// Export entire transcript document to formatted JSON.
pub fn export_to_json(doc: &TranscriptDocument) -> Result<String> {
    serde_json::to_string_pretty(doc).map_err(Into::into)
}

/// Export transcript document to Markdown (.md) meeting notes format.
pub fn export_to_markdown(doc: &TranscriptDocument) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", doc.title));

    let mins = (doc.duration_secs / 60.0).floor() as u64;
    let secs = (doc.duration_secs % 60.0).round() as u64;
    out.push_str(&format!("- **Duration:** {:02}m {:02}s\n", mins, secs));
    if let Some(ref lang) = doc.language {
        out.push_str(&format!("- **Language:** {}\n", lang));
    }
    if let Some(ref model) = doc.model_name {
        out.push_str(&format!("- **Model:** {}\n", model));
    }
    out.push_str("\n---\n\n");

    if let Some(ref summary) = doc.summary_or_post_processed {
        out.push_str("## 📝 Summary & Key Takeaways\n\n");
        out.push_str(summary.trim());
        out.push_str("\n\n---\n\n");
    }

    out.push_str("## 🎙️ Transcript\n\n");
    for seg in &doc.segments {
        let ts = format_timestamp_human(seg.start_ms);
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(ref speaker) = seg.speaker {
            out.push_str(&format!("**`{}` {}**: {}\n\n", ts, speaker, text));
        } else {
            out.push_str(&format!("**`{}`** {}\n\n", ts, text));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_segments() -> Vec<TranscriptSegment> {
        vec![
            TranscriptSegment {
                id: 1,
                start_ms: 1200,
                end_ms: 4500,
                text: "Hello everyone, welcome to the project meeting.".to_string(),
                speaker: Some("Alice".to_string()),
            },
            TranscriptSegment {
                id: 2,
                start_ms: 5000,
                end_ms: 8200,
                text: "Thanks Alice. Let's review the roadmap.".to_string(),
                speaker: Some("Bob".to_string()),
            },
        ]
    }

    #[test]
    fn test_format_timestamps() {
        assert_eq!(format_timestamp_srt(1234), "00:00:01,234");
        assert_eq!(format_timestamp_srt(3661500), "01:01:01,500");
        assert_eq!(format_timestamp_vtt(1234), "00:00:01.234");
        assert_eq!(format_timestamp_human(65000), "[01:05]");
        assert_eq!(format_timestamp_human(3665000), "[01:01:05]");
    }

    #[test]
    fn test_export_to_srt() {
        let srt = export_to_srt(&sample_segments());
        assert!(srt.contains("1\n00:00:01,200 --> 00:00:04,500"));
        assert!(srt.contains("[Alice] Hello everyone"));
        assert!(srt.contains("2\n00:00:05,000 --> 00:00:08,200"));
    }

    #[test]
    fn test_export_to_vtt() {
        let vtt = export_to_vtt(&sample_segments());
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.200 --> 00:00:04.500"));
        assert!(vtt.contains("<v Alice>Hello everyone"));
    }

    #[test]
    fn test_export_to_txt() {
        let txt = export_to_txt(&sample_segments(), true);
        assert!(txt.contains("[00:01] Alice: Hello everyone"));
        assert!(txt.contains("[00:05] Bob: Thanks Alice"));

        let txt_plain = export_to_txt(&sample_segments(), false);
        assert!(!txt_plain.contains("[00:01]"));
        assert!(txt_plain.contains("Alice: Hello everyone"));
    }

    #[test]
    fn test_export_to_markdown() {
        let doc = TranscriptDocument {
            title: "Sprint Planning".to_string(),
            language: Some("en".to_string()),
            model_name: Some("whisper-large-v3".to_string()),
            duration_secs: 125.0,
            created_at_unix: 1725123456,
            segments: sample_segments(),
            summary_or_post_processed: Some("Action items: complete migration.".to_string()),
        };

        let md = export_to_markdown(&doc);
        assert!(md.contains("# Sprint Planning"));
        assert!(md.contains("## 📝 Summary & Key Takeaways"));
        assert!(md.contains("Action items: complete migration."));
        assert!(md.contains("## 🎙️ Transcript"));
        assert!(md.contains("**`[00:01]` Alice**: Hello everyone"));
    }
}
