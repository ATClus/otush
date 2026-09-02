//! Template engine and environment context extraction for AI prompt templates.
//!
//! Supports dynamic variables in prompt templates:
//! - `${output}`: The input text (transcription or selected text).
//! - `${selected_text}`: Text selected by the user.
//! - `${clipboard}`: System clipboard contents.
//! - `${active_window}`: Title or class of the focused window.
//! - `${date}`: Current local date (YYYY-MM-DD).
//! - `${datetime}`: Current local date and time (YYYY-MM-DD HH:MM:SS).
//! - `${time}`: Current local time (HH:MM:SS).
//! - `${language}`: Active transcription language.

use crate::context::AppContext;
use log::debug;

/// Context variables gathered from the system and application state
/// to interpolate into prompt templates.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    /// The primary input text (transcription or selection)
    pub output: String,
    /// Explicitly captured selected text from the active application
    pub selected_text: Option<String>,
    /// System clipboard text
    pub clipboard: Option<String>,
    /// Title or class of the active window at invocation time
    pub active_window: Option<String>,
    /// Formatted date string (defaults to local YYYY-MM-DD)
    pub date: Option<String>,
    /// Active language code or name
    pub language: Option<String>,
}

impl TemplateContext {
    /// Create a minimal template context with only the primary input text.
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            selected_text: None,
            clipboard: None,
            active_window: None,
            date: None,
            language: None,
        }
    }

    /// Gather full context from the running application and system state.
    pub fn gather(
        ctx: &AppContext,
        output: &str,
        pre_selection: Option<String>,
        pre_window: Option<String>,
        pre_clipboard: Option<String>,
    ) -> Self {
        let settings = ctx.settings();
        let language = crate::actions::resolve_effective_language(ctx, &settings);

        let clipboard = pre_clipboard.or_else(|| crate::clipboard::read_clipboard_text().ok());
        let active_window = pre_window.or_else(get_active_window_title);
        let selected_text = pre_selection;
        let date = Some(chrono::Local::now().format("%Y-%m-%d").to_string());

        Self {
            output: output.to_string(),
            selected_text,
            clipboard,
            active_window,
            date,
            language: if language.is_empty() {
                None
            } else {
                Some(language)
            },
        }
    }
}

/// Checks if a template explicitly references the input payload placeholder (`${output}` or `${selected_text}`).
pub fn has_input_placeholder(template: &str) -> bool {
    template.contains("${output}") || template.contains("${selected_text}")
}

/// Expands all dynamic variables in the prompt template.
pub fn expand_template(template: &str, context: &TemplateContext) -> String {
    let now = chrono::Local::now();
    let date_str = context
        .date
        .clone()
        .unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let datetime_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let time_str = now.format("%H:%M:%S").to_string();

    let selected_str = context
        .selected_text
        .as_deref()
        .unwrap_or(context.output.as_str());

    let clipboard_str = context.clipboard.as_deref().unwrap_or("");
    let window_str = context.active_window.as_deref().unwrap_or("");
    let lang_str = context.language.as_deref().unwrap_or("");

    template
        .replace("${output}", &context.output)
        .replace("${selected_text}", selected_str)
        .replace("${clipboard}", clipboard_str)
        .replace("${active_window}", window_str)
        .replace("${date}", &date_str)
        .replace("${datetime}", &datetime_str)
        .replace("${time}", &time_str)
        .replace("${language}", lang_str)
}

/// Expands variables for system prompts in structured output mode,
/// removing the `${output}` placeholder since the input is sent as user content.
pub fn expand_template_for_system_prompt(template: &str, context: &TemplateContext) -> String {
    let now = chrono::Local::now();
    let date_str = context
        .date
        .clone()
        .unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let datetime_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let time_str = now.format("%H:%M:%S").to_string();

    let selected_str = context
        .selected_text
        .as_deref()
        .unwrap_or(context.output.as_str());

    let clipboard_str = context.clipboard.as_deref().unwrap_or("");
    let window_str = context.active_window.as_deref().unwrap_or("");
    let lang_str = context.language.as_deref().unwrap_or("");

    template
        .replace("${output}", "")
        .replace("${selected_text}", selected_str)
        .replace("${clipboard}", clipboard_str)
        .replace("${active_window}", window_str)
        .replace("${date}", &date_str)
        .replace("${datetime}", &datetime_str)
        .replace("${time}", &time_str)
        .replace("${language}", lang_str)
        .trim()
        .to_string()
}

/// Query the active window title or application name across Linux environments.
/// Returns `None` if detection is unavailable or unsupported on the running compositor.
pub fn get_active_window_title() -> Option<String> {
    // 1. Hyprland
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        if let Ok(output) = std::process::Command::new("hyprctl")
            .args(["activewindow", "-j"])
            .output()
        {
            if output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
                        let trimmed = title.trim();
                        if !trimmed.is_empty() {
                            debug!("Detected active window via hyprctl: {}", trimmed);
                            return Some(trimmed.to_string());
                        }
                    }
                    if let Some(class) = json.get("class").and_then(|v| v.as_str()) {
                        let trimmed = class.trim();
                        if !trimmed.is_empty() {
                            debug!("Detected active window class via hyprctl: {}", trimmed);
                            return Some(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Sway / i3
    if std::env::var("SWAYSOCK").is_ok() || std::env::var("I3SOCK").is_ok() {
        let cmd = if std::env::var("SWAYSOCK").is_ok() {
            "swaymsg"
        } else {
            "i3-msg"
        };
        if let Ok(output) = std::process::Command::new(cmd)
            .args(["-t", "get_tree"])
            .output()
        {
            if output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if let Some(focused) = find_focused_node(&json) {
                        debug!("Detected active window via {}: {}", cmd, focused);
                        return Some(focused);
                    }
                }
            }
        }
    }

    // 3. X11 / XWayland via xdotool
    if let Ok(output) = std::process::Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .output()
    {
        if output.status.success() {
            let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !title.is_empty() {
                debug!("Detected active window via xdotool: {}", title);
                return Some(title);
            }
        }
    }

    // 4. GNOME Shell via gdbus (if eval interface is accessible)
    if let Ok(output) = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell",
            "--object-path",
            "/org/gnome/Shell",
            "--method",
            "org.gnome.Shell.Eval",
            "global.display.focus_window ? (global.display.focus_window.get_title() || global.display.focus_window.get_wm_class()) : ''",
        ])
        .output()
    {
        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            if let Some(start) = out.find('\'') {
                if let Some(end) = out.rfind('\'') {
                    if start < end {
                        let title = out[start + 1..end].trim();
                        if !title.is_empty() {
                            debug!("Detected active window via GNOME Shell eval: {}", title);
                            return Some(title.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

fn find_focused_node(node: &serde_json::Value) -> Option<String> {
    if node
        .get("focused")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        if let Some(name) = node.get("name").and_then(|v| v.as_str()) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(app_id) = node.get("app_id").and_then(|v| v.as_str()) {
            let trimmed = app_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Some(nodes) = node.get("nodes").and_then(|v| v.as_array()) {
        for n in nodes {
            if let Some(found) = find_focused_node(n) {
                return Some(found);
            }
        }
    }
    if let Some(floating) = node.get("floating_nodes").and_then(|v| v.as_array()) {
        for n in floating {
            if let Some(found) = find_focused_node(n) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_template_all_variables() {
        let ctx = TemplateContext {
            output: "Hello World".to_string(),
            selected_text: Some("Selected text snippet".to_string()),
            clipboard: Some("Clipboard contents".to_string()),
            active_window: Some("Visual Studio Code".to_string()),
            date: Some("2026-09-02".to_string()),
            language: Some("pt".to_string()),
        };

        let template = "Window: ${active_window} | Date: ${date} | Lang: ${language}\nClip: ${clipboard}\nSel: ${selected_text}\nOut: ${output}";
        let result = expand_template(template, &ctx);

        assert_eq!(
            result,
            "Window: Visual Studio Code | Date: 2026-09-02 | Lang: pt\nClip: Clipboard contents\nSel: Selected text snippet\nOut: Hello World"
        );
    }

    #[test]
    fn test_expand_template_missing_optionals() {
        let ctx = TemplateContext::new("Sample text");
        let template =
            "Win: '${active_window}' Clip: '${clipboard}' Lang: '${language}' Out: '${output}'";
        let result = expand_template(template, &ctx);

        assert_eq!(result, "Win: '' Clip: '' Lang: '' Out: 'Sample text'");
    }

    #[test]
    fn test_expand_template_for_system_prompt() {
        let ctx = TemplateContext {
            output: "Voice text".to_string(),
            selected_text: None,
            clipboard: Some("clip".to_string()),
            active_window: Some("Browser".to_string()),
            date: Some("2026-09-02".to_string()),
            language: Some("en".to_string()),
        };

        let template = "<instruction>Process window ${active_window}</instruction>\n<transcript>\n${output}\n</transcript>";
        let result = expand_template_for_system_prompt(template, &ctx);

        assert_eq!(
            result,
            "<instruction>Process window Browser</instruction>\n<transcript>\n\n</transcript>"
        );
    }

    #[test]
    fn test_has_input_placeholder() {
        assert!(has_input_placeholder("Here is ${output}"));
        assert!(has_input_placeholder("Here is ${selected_text}"));
        assert!(!has_input_placeholder("Translate to JSON please"));
    }

    #[test]
    fn test_expand_template_selected_text_fallback_to_output() {
        let ctx = TemplateContext::new("Fallback Content");
        let template = "Transform: ${selected_text}";
        let result = expand_template(template, &ctx);
        assert_eq!(result, "Transform: Fallback Content");
    }

    #[test]
    fn test_expand_template_datetime_and_time() {
        let ctx = TemplateContext::new("Test");
        let template = "Date: ${date}, Datetime: ${datetime}, Time: ${time}";
        let result = expand_template(template, &ctx);
        assert!(!result.contains("${date}"));
        assert!(!result.contains("${datetime}"));
        assert!(!result.contains("${time}"));
    }

    #[test]
    fn test_find_focused_node() {
        let json = serde_json::json!({
            "nodes": [
                {
                    "name": "unfocused_window",
                    "focused": false
                },
                {
                    "name": "Editor - main.rs",
                    "app_id": "code",
                    "focused": true
                }
            ]
        });
        let focused = find_focused_node(&json);
        assert_eq!(focused, Some("Editor - main.rs".to_string()));
    }
}
