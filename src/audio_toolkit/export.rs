//! Subtitle, transcript, and meeting notes exporters.
//!
//! Provides conversion to standard industry formats:
//! - SubRip (.srt) with professional timing, character-per-line (CPL) limits, and line balancing
//! - WebVTT (.vtt) with professional formatting
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

/// Standard industry configuration for broadcast and streaming subtitles (Netflix / BBC / EBU standards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubtitleConfig {
    /// Maximum characters allowed per line (industry standard: 37 - 42).
    pub max_chars_per_line: usize,
    /// Maximum lines per subtitle block (industry standard: 2).
    pub max_lines: usize,
    /// Minimum duration for a subtitle block in ms (industry standard: 1000 - 1200ms).
    pub min_duration_ms: u64,
    /// Maximum duration for a subtitle block in ms (industry standard: 5000 - 6000ms).
    pub max_duration_ms: u64,
    /// Gap between consecutive subtitles in ms (usually 40 - 80ms to avoid subtitle bleeding).
    pub gap_ms: u64,
}

impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            max_chars_per_line: 40,
            max_lines: 2,
            min_duration_ms: 1200,
            max_duration_ms: 5500,
            gap_ms: 60,
        }
    }
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

/// Intelligently wrap a subtitle text into balanced lines respecting max characters per line.
pub fn wrap_subtitle_lines(text: &str, max_chars_per_line: usize, max_lines: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars_per_line || max_lines <= 1 {
        return trimmed.to_string();
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() <= 1 {
        return trimmed.to_string();
    }

    if max_lines == 2 {
        let total_chars: usize =
            words.iter().map(|w| w.chars().count()).sum::<usize>() + (words.len() - 1);
        let target_half = total_chars / 2;

        let mut best_split = 1;
        let mut best_diff = usize::MAX;

        let mut current_len = 0;
        for (i, word) in words.iter().enumerate().take(words.len().saturating_sub(1)) {
            current_len += word.chars().count() + if i > 0 { 1 } else { 0 };
            let diff = current_len.abs_diff(target_half);

            let line1_len = current_len;
            let line2_len = total_chars - line1_len - 1;

            if line1_len <= max_chars_per_line
                && line2_len <= max_chars_per_line
                && diff < best_diff
            {
                best_diff = diff;
                best_split = i + 1;
            }
        }

        // If no split fit within max_chars_per_line, pick the first greedily
        if best_diff == usize::MAX {
            let mut line1 = Vec::new();
            let mut line1_len = 0;
            let mut split_idx = 0;
            for (i, w) in words.iter().enumerate() {
                let w_len = w.chars().count();
                let added = if line1_len == 0 { w_len } else { w_len + 1 };
                if line1_len + added <= max_chars_per_line {
                    line1.push(*w);
                    line1_len += added;
                    split_idx = i + 1;
                } else {
                    break;
                }
            }
            if split_idx > 0 && split_idx < words.len() {
                best_split = split_idx;
            }
        }

        let line1 = words[..best_split].join(" ");
        let line2 = words[best_split..].join(" ");
        format!("{}\n{}", line1, line2)
    } else {
        trimmed.to_string()
    }
}

/// Split text into natural sentence and phrase units.
pub fn split_into_phrases(text: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        current.push(c);

        let is_terminal = matches!(c, '.' | '!' | '?' | ';' | '\n');
        let next_is_space_or_end = i + 1 == chars.len() || chars[i + 1].is_whitespace();

        if is_terminal && next_is_space_or_end {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                phrases.push(trimmed);
            }
            current.clear();
        }
        i += 1;
    }

    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        phrases.push(remaining);
    }

    // Secondary pass: if any sentence is too long (> 65 chars or > 12 words), split at comma or conjunction
    let mut result = Vec::new();
    for p in phrases {
        if p.chars().count() > 65 {
            let sub_parts: Vec<&str> = p.split(',').collect();
            if sub_parts.len() > 1 {
                for (idx, part) in sub_parts.iter().enumerate() {
                    let mut s = part.trim().to_string();
                    if idx + 1 < sub_parts.len() {
                        s.push(',');
                    }
                    if !s.trim().is_empty() {
                        result.push(s);
                    }
                }
                continue;
            }
        }
        result.push(p);
    }

    if result.is_empty() && !text.trim().is_empty() {
        result.push(text.trim().to_string());
    }

    result
}

/// Transform raw coarse transcription segments into professional, well-paced subtitle cards.
pub fn to_professional_subtitles(
    segments: &[TranscriptSegment],
    config: SubtitleConfig,
) -> Vec<TranscriptSegment> {
    let mut output = Vec::new();
    let mut current_id = 1;

    for seg in segments {
        let raw_text = seg.text.trim();
        if raw_text.is_empty() {
            continue;
        }

        let phrases = split_into_phrases(raw_text);
        if phrases.is_empty() {
            continue;
        }

        let seg_start = seg.start_ms;
        let seg_end = seg.end_ms.max(seg.start_ms + 500);
        let total_duration = seg_end - seg_start;

        let total_chars: usize = phrases.iter().map(|p| p.chars().count().max(1)).sum();

        let mut current_start = seg_start;
        let phrase_count = phrases.len();

        for (idx, phrase) in phrases.iter().enumerate() {
            let phrase_chars = phrase.chars().count().max(1);
            let raw_phrase_duration =
                ((phrase_chars as f64 / total_chars as f64) * total_duration as f64).round() as u64;

            let phrase_duration = raw_phrase_duration
                .max(config.min_duration_ms)
                .min(config.max_duration_ms);

            let mut phrase_end = current_start + phrase_duration;
            if idx + 1 == phrase_count {
                phrase_end = phrase_end.max(seg_end);
            }

            // Wrap subtitle lines cleanly
            let wrapped_text =
                wrap_subtitle_lines(phrase, config.max_chars_per_line, config.max_lines);

            output.push(TranscriptSegment {
                id: current_id,
                start_ms: current_start,
                end_ms: phrase_end,
                text: wrapped_text,
                speaker: seg.speaker.clone(),
            });

            current_id += 1;
            current_start = phrase_end + config.gap_ms;
        }
    }

    output
}

/// Export segments as professional SubRip (.srt) subtitle string.
/// Automatically breaks into readable sentence units with balanced lines and clean pacing.
pub fn export_to_srt(segments: &[TranscriptSegment]) -> String {
    let pro_segments = to_professional_subtitles(segments, SubtitleConfig::default());
    export_to_srt_raw(&pro_segments)
}

/// Export segments as raw SubRip (.srt) subtitle string without re-chunking.
pub fn export_to_srt_raw(segments: &[TranscriptSegment]) -> String {
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

/// Export segments as professional WebVTT (.vtt) subtitle string.
pub fn export_to_vtt(segments: &[TranscriptSegment]) -> String {
    let pro_segments = to_professional_subtitles(segments, SubtitleConfig::default());
    export_to_vtt_raw(&pro_segments)
}

/// Export segments as raw WebVTT (.vtt) subtitle string.
pub fn export_to_vtt_raw(segments: &[TranscriptSegment]) -> String {
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
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(ref speaker) = seg.speaker {
            out.push_str(&format!("**{}**: {}\n\n", speaker, text));
        } else {
            out.push_str(&format!("{}\n\n", text));
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
                end_ms: 6500,
                text: "Hello everyone, welcome to the project meeting. Today we discuss subtitles."
                    .to_string(),
                speaker: Some("Alice".to_string()),
            },
            TranscriptSegment {
                id: 2,
                start_ms: 7000,
                end_ms: 12000,
                text: "Thanks Alice. Let's review the new professional export pipeline."
                    .to_string(),
                speaker: Some("Bob".to_string()),
            },
        ]
    }

    #[test]
    fn test_wrap_subtitle_lines() {
        let text = "Hello everyone, today we are testing the subtitle line wrapper.";
        let wrapped = wrap_subtitle_lines(text, 35, 2);
        assert!(wrapped.contains('\n'));
        for line in wrapped.lines() {
            assert!(line.chars().count() <= 38);
        }
    }

    #[test]
    fn test_split_into_phrases() {
        let text = "This is sentence one. This is sentence two! And sentence three?";
        let phrases = split_into_phrases(text);
        assert_eq!(phrases.len(), 3);
        assert_eq!(phrases[0], "This is sentence one.");
        assert_eq!(phrases[1], "This is sentence two!");
        assert_eq!(phrases[2], "And sentence three?");
    }

    #[test]
    fn test_to_professional_subtitles() {
        let raw = vec![TranscriptSegment {
            id: 1,
            start_ms: 0,
            end_ms: 10000,
            text: "First sentence here. Second sentence here.".to_string(),
            speaker: None,
        }];

        let pro = to_professional_subtitles(&raw, SubtitleConfig::default());
        assert_eq!(pro.len(), 2);
        assert_eq!(pro[0].id, 1);
        assert_eq!(pro[0].text, "First sentence here.");
        assert_eq!(pro[1].id, 2);
        assert_eq!(pro[1].text, "Second sentence here.");
        assert!(pro[1].start_ms >= pro[0].end_ms);
    }

    #[test]
    fn test_export_to_srt_professional() {
        let srt = export_to_srt(&sample_segments());
        assert!(srt.contains("1\n00:00:01,200 -->"));
        assert!(srt.contains("Hello everyone,"));
    }

    #[test]
    fn test_export_to_vtt_professional() {
        let vtt = export_to_vtt(&sample_segments());
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.200 -->"));
    }

    #[test]
    fn test_export_to_txt() {
        let txt = export_to_txt(&sample_segments(), true);
        assert!(txt.contains("[00:01] Alice: Hello everyone"));
        assert!(txt.contains("[00:07] Bob: Thanks Alice"));
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
    }
}
