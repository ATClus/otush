//! Audio & Voice settings page — consolidating all audio hardware,
//! digital signal processing (DSP), voice activity detection (VAD),
//! feedback sound effects, and real-time microphone testing into one suite.

use crate::audio_feedback;
use crate::commands;
use crate::context::{AppContext, AppEvent};
use crate::settings::{AudioCaptureSource, SoundTheme, VadBackend};
use crate::shortcut;
use gtk4::prelude::*;
use libadwaita::prelude::*;

/// Build the Audio & Voice preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Audio &amp; Voice");
    page.set_icon_name(Some("audio-input-microphone-symbolic"));

    let settings = ctx.settings();

    // ========================================================================
    // 1. Input Device & Recording Mode
    // ========================================================================
    let input_group = libadwaita::PreferencesGroup::new();
    input_group.set_title("Microphone &amp; Input Devices");
    input_group.set_description(Some(
        "Configure your primary recording device and capture behavior.",
    ));
    input_group.set_hexpand(true);

    // Audio Capture Source Mode (Microphone / System Audio / Meeting Mode)
    let source_row = libadwaita::ComboRow::new();
    source_row.set_title("Audio Capture Source");
    source_row.set_subtitle("Capture microphone, desktop system audio (Meet/Teams), or both");
    let source_icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
    source_row.add_prefix(&source_icon);

    let source_options = [
        "Microphone Only (Standard)",
        "System Audio Only (Live Meet / Video)",
        "Meeting Mode (Mic + System Audio Mixed)",
    ];
    let source_model = gtk4::StringList::new(&source_options);
    source_row.set_model(Some(&source_model));
    let initial_source_idx = match settings.audio_capture_source {
        AudioCaptureSource::MicrophoneOnly => 0,
        AudioCaptureSource::SystemAudioOnly => 1,
        AudioCaptureSource::Mixed => 2,
    };
    source_row.set_selected(initial_source_idx);

    // Microphone selection (combo, populated asynchronously)
    let mic_row = libadwaita::ComboRow::new();
    mic_row.set_title("Microphone Device");
    mic_row.set_subtitle("Select input device for recording");
    let mic_icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
    mic_row.add_prefix(&mic_icon);
    let current_mic = settings.selected_microphone.clone().unwrap_or_default();
    mic_row.set_subtitle(if current_mic.is_empty() {
        "Default"
    } else {
        &current_mic
    });
    mic_row.set_visible(settings.audio_capture_source != AudioCaptureSource::SystemAudioOnly);
    populate_microphones(ctx, &mic_row);

    // System Audio Loopback Device selection
    let sys_audio_row = libadwaita::ComboRow::new();
    sys_audio_row.set_title("System Audio Device");
    sys_audio_row.set_subtitle("Select desktop output monitor source for meeting transcription");
    let sys_icon = gtk4::Image::from_icon_name("audio-speakers-symbolic");
    sys_audio_row.add_prefix(&sys_icon);
    let current_sys = settings
        .selected_system_audio_device
        .clone()
        .unwrap_or_default();
    sys_audio_row.set_subtitle(if current_sys.is_empty() {
        "Default System Audio Monitor"
    } else {
        &current_sys
    });
    sys_audio_row.set_visible(settings.audio_capture_source != AudioCaptureSource::MicrophoneOnly);
    populate_system_audio_sources(ctx, &sys_audio_row);

    // Push-to-talk
    let ptt = libadwaita::SwitchRow::new();
    ptt.set_title("Push-to-Talk");
    ptt.set_subtitle("Hold the shortcut key to record; release to transcribe");
    let ptt_icon = gtk4::Image::from_icon_name("input-keyboard-symbolic");
    ptt.add_prefix(&ptt_icon);
    ptt.set_active(settings.push_to_talk);
    let ptt_ctx = ctx.clone();
    ptt.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_ptt_setting(&ptt_ctx, row.is_active()) {
            ptt_ctx.report_error("change_ptt_setting", err);
        }
    });

    // Always-on microphone
    let always_on = libadwaita::SwitchRow::new();
    always_on.set_title("Always-On Microphone");
    always_on.set_subtitle("Keep input stream open for instant zero-latency capture");
    let ao_icon = gtk4::Image::from_icon_name("media-record-symbolic");
    always_on.add_prefix(&ao_icon);
    always_on.set_active(settings.always_on_microphone);
    always_on.set_visible(settings.audio_capture_source != AudioCaptureSource::SystemAudioOnly);
    let always_on_ctx = ctx.clone();
    always_on.connect_active_notify(move |row| {
        let ctx = always_on_ctx.clone();
        let enabled = row.is_active();
        crate::runtime::spawn(async move {
            if let Err(err) = commands::audio::update_microphone_mode(&ctx, enabled).await {
                ctx.report_error("update_microphone_mode", err);
            }
        });
    });

    // Wire progressive disclosure on source_row selection
    let src_ctx = ctx.clone();
    let sys_row_weak = glib::SendWeakRef::from(sys_audio_row.downgrade());
    let mic_row_weak = glib::SendWeakRef::from(mic_row.downgrade());
    let ao_weak = glib::SendWeakRef::from(always_on.downgrade());
    source_row.connect_selected_notify(move |row| {
        let mode = match row.selected() {
            1 => AudioCaptureSource::SystemAudioOnly,
            2 => AudioCaptureSource::Mixed,
            _ => AudioCaptureSource::MicrophoneOnly,
        };

        if let (Some(sys), Some(mic), Some(ao)) = (
            sys_row_weak.clone().into_weak_ref().upgrade(),
            mic_row_weak.clone().into_weak_ref().upgrade(),
            ao_weak.clone().into_weak_ref().upgrade(),
        ) {
            match mode {
                AudioCaptureSource::MicrophoneOnly => {
                    sys.set_visible(false);
                    mic.set_visible(true);
                    ao.set_visible(true);
                }
                AudioCaptureSource::SystemAudioOnly => {
                    sys.set_visible(true);
                    mic.set_visible(false);
                    ao.set_visible(false);
                }
                AudioCaptureSource::Mixed => {
                    sys.set_visible(true);
                    mic.set_visible(true);
                    ao.set_visible(true);
                }
            }
        }

        let ctx = src_ctx.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = commands::audio::set_audio_capture_source(&ctx, mode).await {
                ctx.report_error("set_audio_capture_source", err);
            }
        });
    });

    input_group.add(&source_row);
    input_group.add(&mic_row);
    input_group.add(&sys_audio_row);
    input_group.add(&ptt);
    input_group.add(&always_on);

    page.add(&input_group);

    // ========================================================================
    // 2. Live Microphone Level & Test Tool
    // ========================================================================
    let test_group = libadwaita::PreferencesGroup::new();
    test_group.set_title("Microphone Test &amp; Input Level");
    test_group.set_description(Some(
        "Monitor live microphone signal levels and verify voice capture.",
    ));
    test_group.set_hexpand(true);

    let meter_row = libadwaita::ActionRow::new();
    meter_row.set_title("Live Audio Input Level");
    meter_row.set_subtitle("Speak into your microphone to verify volume and clarity");
    let meter_icon = gtk4::Image::from_icon_name("audio-volume-high-symbolic");
    meter_row.add_prefix(&meter_icon);

    let level_bar = gtk4::LevelBar::new();
    level_bar.set_min_value(0.0);
    level_bar.set_max_value(1.0);
    level_bar.set_value(0.0);
    level_bar.set_valign(gtk4::Align::Center);
    level_bar.set_size_request(160, 12);
    meter_row.add_suffix(&level_bar);
    test_group.add(&meter_row);

    // Live Monitor Switch
    let monitor_row = libadwaita::SwitchRow::new();
    monitor_row.set_title("Monitor Microphone Input");
    monitor_row.set_subtitle("Enable real-time input meter to test and calibrate your microphone");
    let mon_icon = gtk4::Image::from_icon_name("media-record-symbolic");
    monitor_row.add_prefix(&mon_icon);
    monitor_row.set_active(false);
    let mon_ctx = ctx.clone();
    let level_bar_ref = level_bar.clone();
    monitor_row.connect_active_notify(move |row| {
        if row.is_active() {
            commands::audio::start_mic_monitor(&mon_ctx);
        } else {
            commands::audio::stop_mic_monitor();
            level_bar_ref.set_value(0.0);
        }
    });
    test_group.add(&monitor_row);

    // Wire live mic levels from EventBus into the level bar
    let level_bar_weak = glib::SendWeakRef::from(level_bar.downgrade());
    ctx.bus.subscribe(move |event| {
        if let AppEvent::MicLevel(level) = event {
            let level_bar_weak = level_bar_weak.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(bar) = level_bar_weak.into_weak_ref().upgrade() {
                    let scaled = (level.clamp(0.0, 1.0) as f64).sqrt();
                    bar.set_value(scaled);
                }
            });
        }
    });

    page.add(&test_group);

    // ========================================================================
    // 3. Voice Enhancement & DSP Suite
    // ========================================================================
    let dsp_group = libadwaita::PreferencesGroup::new();
    dsp_group.set_title("Voice Enhancement &amp; DSP");
    dsp_group.set_description(Some(
        "Real-time audio processing to boost clarity, volume, and suppress background noise.",
    ));
    dsp_group.set_hexpand(true);

    // Software Input Gain Boost
    let gain_adjustment =
        gtk4::Adjustment::new(settings.audio_input_gain as f64, 0.5, 4.0, 0.1, 0.5, 0.0);
    let gain_row = libadwaita::SpinRow::new(Some(&gain_adjustment), 0.1, 1);
    gain_row.set_title("Software Input Gain");
    gain_row.set_subtitle("Boost quiet microphones (1.0x = normal, 2.0x = +6 dB, 4.0x = +12 dB)");
    let gain_icon = gtk4::Image::from_icon_name("audio-volume-high-symbolic");
    gain_row.add_prefix(&gain_icon);
    gain_row.set_snap_to_ticks(true);
    let gain_ctx = ctx.clone();
    gain_adjustment.connect_value_changed(move |adj| {
        if let Err(err) = shortcut::change_audio_input_gain_setting(&gain_ctx, adj.value() as f32) {
            gain_ctx.report_error("change_audio_input_gain_setting", err);
        }
    });
    dsp_group.add(&gain_row);

    // Dynamic Voice Normalization (AGC)
    let norm_row = libadwaita::SwitchRow::new();
    norm_row.set_title("Automatic Voice Normalization (AGC)");
    norm_row.set_subtitle("Level speech amplitude to optimal loudness with soft-knee limiter");
    let norm_icon = gtk4::Image::from_icon_name("format-justify-fill-symbolic");
    norm_row.add_prefix(&norm_icon);
    norm_row.set_active(settings.audio_normalization_enabled);
    let norm_ctx = ctx.clone();
    norm_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_audio_normalization_setting(&norm_ctx, row.is_active()) {
            norm_ctx.report_error("change_audio_normalization_setting", err);
        }
    });
    dsp_group.add(&norm_row);

    // High-Pass Rumble Filter (80 Hz)
    let hpf_row = libadwaita::SwitchRow::new();
    hpf_row.set_title("High-Pass Rumble Filter (80 Hz)");
    hpf_row.set_subtitle("Eliminate 50/60 Hz electrical hum, desk thumps, and low rumble");
    let hpf_icon = gtk4::Image::from_icon_name("preferences-system-symbolic");
    hpf_row.add_prefix(&hpf_icon);
    hpf_row.set_active(settings.audio_high_pass_filter_enabled);
    let hpf_ctx = ctx.clone();
    hpf_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_audio_high_pass_filter_setting(&hpf_ctx, row.is_active())
        {
            hpf_ctx.report_error("change_audio_high_pass_filter_setting", err);
        }
    });
    dsp_group.add(&hpf_row);

    // Adaptive Noise Gate / Noise Reduction
    let noise_row = libadwaita::SwitchRow::new();
    noise_row.set_title("Noise Suppression (Adaptive Noise Gate)");
    noise_row.set_subtitle("Attenuate ambient room hiss, PC fans, and typing during pauses");
    let noise_icon = gtk4::Image::from_icon_name("dialog-password-symbolic");
    noise_row.add_prefix(&noise_icon);
    noise_row.set_active(settings.audio_noise_reduction_enabled);

    // Noise Gate Sensitivity Threshold (gated on noise_row active)
    let threshold_adj = gtk4::Adjustment::new(
        settings.audio_noise_gate_threshold_db as f64,
        -60.0,
        -25.0,
        1.0,
        5.0,
        0.0,
    );
    let threshold_row = libadwaita::SpinRow::new(Some(&threshold_adj), 1.0, 0);
    threshold_row.set_title("Noise Gate Sensitivity Threshold (dB)");
    threshold_row.set_subtitle(
        "Lower values keep subtle whispers; higher values cut louder background noise",
    );
    let thresh_icon = gtk4::Image::from_icon_name("format-indent-more-symbolic");
    threshold_row.add_prefix(&thresh_icon);
    threshold_row.set_snap_to_ticks(true);
    threshold_row.set_visible(settings.audio_noise_reduction_enabled);

    let thresh_ctx = ctx.clone();
    threshold_adj.connect_value_changed(move |adj| {
        let _ =
            shortcut::change_audio_noise_gate_threshold_setting(&thresh_ctx, adj.value() as f32);
    });

    let noise_ctx = ctx.clone();
    let threshold_weak = glib::SendWeakRef::from(threshold_row.downgrade());
    noise_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        if let Err(err) = shortcut::change_audio_noise_reduction_setting(&noise_ctx, is_active) {
            noise_ctx.report_error("change_audio_noise_reduction_setting", err);
        }
        if let Some(tr) = threshold_weak.clone().into_weak_ref().upgrade() {
            tr.set_visible(is_active);
        }
    });
    dsp_group.add(&noise_row);
    dsp_group.add(&threshold_row);

    page.add(&dsp_group);

    // ========================================================================
    // 4. Voice Activity Detection (VAD)
    // ========================================================================
    let vad_group = libadwaita::PreferencesGroup::new();
    vad_group.set_title("Voice Activity Detection (VAD)");
    vad_group.set_description(Some(
        "Detect speech segments automatically and trim background silence.",
    ));
    vad_group.set_hexpand(true);

    let vad_enabled = libadwaita::SwitchRow::new();
    vad_enabled.set_title("Enable Voice Activity Detection");
    vad_enabled.set_subtitle("Automatically identify speech and trim silence");
    let vad_icon = gtk4::Image::from_icon_name("media-record-symbolic");
    vad_enabled.add_prefix(&vad_icon);
    vad_enabled.set_active(settings.vad_enabled);

    let vad_backend_row = libadwaita::ComboRow::new();
    vad_backend_row.set_title("VAD Engine Backend");
    let backend_icon = gtk4::Image::from_icon_name("preferences-system-symbolic");
    vad_backend_row.add_prefix(&backend_icon);
    let vad_labels = ["Silero VAD (Recommended)", "Earshot (Experimental)"];
    let vad_model = gtk4::StringList::new(&vad_labels);
    vad_backend_row.set_model(Some(&vad_model));
    vad_backend_row.set_selected(match settings.vad_backend {
        VadBackend::Silero => 0,
        VadBackend::Earshot => 1,
    });
    vad_backend_row.set_visible(settings.vad_enabled);

    let ctx2 = ctx.clone();
    vad_backend_row.connect_selected_notify(move |row| {
        let backend = match row.selected() {
            1 => VadBackend::Earshot,
            _ => VadBackend::Silero,
        };
        let ctx = ctx2.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = shortcut::change_vad_backend_setting(&ctx, backend).await {
                ctx.report_error("change_vad_backend_setting", err);
            }
        });
    });

    let ctx1 = ctx.clone();
    let backend_weak = glib::SendWeakRef::from(vad_backend_row.downgrade());
    vad_enabled.connect_active_notify(move |row| {
        let is_active = row.is_active();
        if let Err(err) = shortcut::change_vad_enabled_setting(&ctx1, is_active) {
            ctx1.report_error("change_vad_enabled_setting", err);
        }
        if let Some(br) = backend_weak.clone().into_weak_ref().upgrade() {
            br.set_visible(is_active);
        }
    });
    vad_group.add(&vad_enabled);
    vad_group.add(&vad_backend_row);

    let filler = libadwaita::SwitchRow::new();
    filler.set_title("Remove Filler Words");
    filler.set_subtitle("Strip speech disfluencies ('um', 'uh', 'éé') from transcripts");
    let filler_icon = gtk4::Image::from_icon_name("edit-clear-symbolic");
    filler.add_prefix(&filler_icon);
    filler.set_active(settings.filler_word_removal_enabled);
    let ctx3 = ctx.clone();
    filler.connect_active_notify(move |row| {
        if let Err(err) =
            shortcut::change_filler_word_removal_enabled_setting(&ctx3, row.is_active())
        {
            ctx3.report_error("change_filler_word_removal_enabled_setting", err);
        }
    });
    vad_group.add(&filler);

    page.add(&vad_group);

    // ========================================================================
    // 5. Sound Effects & Audio Feedback
    // ========================================================================
    let feedback_group = libadwaita::PreferencesGroup::new();
    feedback_group.set_title("Sound Effects &amp; Audio Feedback");
    feedback_group.set_description(Some("Audio cues for recording start and completion."));
    feedback_group.set_hexpand(true);

    let audio_feedback_row = libadwaita::SwitchRow::new();
    audio_feedback_row.set_title("Play Audio Feedback Cues");
    audio_feedback_row.set_subtitle("Play sound when recording starts and finishes");
    let fb_icon = gtk4::Image::from_icon_name("audio-speakers-symbolic");
    audio_feedback_row.add_prefix(&fb_icon);
    audio_feedback_row.set_active(settings.audio_feedback);

    // Sound Theme
    let theme_row = libadwaita::ComboRow::new();
    theme_row.set_title("Sound Theme");
    let snd_icon = gtk4::Image::from_icon_name("emblem-music-symbolic");
    theme_row.add_prefix(&snd_icon);
    let theme_labels = [("marimba", "Marimba"), ("pop", "Pop"), ("custom", "Custom")];
    let model = gtk4::StringList::new(
        &theme_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    theme_row.set_model(Some(&model));
    theme_row.set_selected(match settings.sound_theme {
        SoundTheme::Marimba => 0,
        SoundTheme::Pop => 1,
        SoundTheme::Custom => 2,
    });
    theme_row.set_visible(settings.audio_feedback);

    let theme_ctx = ctx.clone();
    theme_row.connect_selected_notify(move |row| {
        let theme = match row.selected() {
            1 => SoundTheme::Pop,
            2 => SoundTheme::Custom,
            _ => SoundTheme::Marimba,
        };
        let mut s = theme_ctx.settings();
        s.sound_theme = theme;
        theme_ctx.write_settings(&s);
    });

    // Test start/stop sounds row
    let test_sound_row = libadwaita::ActionRow::new();
    test_sound_row.set_title("Test Feedback Sounds");
    test_sound_row.set_subtitle("Preview start and stop auditory cues");
    let test_icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    test_sound_row.add_prefix(&test_icon);
    test_sound_row.set_visible(settings.audio_feedback);

    let play_start_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
    play_start_btn.set_tooltip_text(Some("Preview Start Sound"));
    play_start_btn.add_css_class("flat");
    play_start_btn.set_valign(gtk4::Align::Center);
    let start_ctx = ctx.clone();
    play_start_btn.connect_clicked(move |_| {
        audio_feedback::play_feedback_sound(&start_ctx, audio_feedback::SoundType::Start);
    });
    test_sound_row.add_suffix(&play_start_btn);

    let play_stop_btn = gtk4::Button::from_icon_name("media-playback-stop-symbolic");
    play_stop_btn.set_tooltip_text(Some("Preview Stop Sound"));
    play_stop_btn.add_css_class("flat");
    play_stop_btn.set_valign(gtk4::Align::Center);
    let stop_ctx = ctx.clone();
    play_stop_btn.connect_clicked(move |_| {
        audio_feedback::play_feedback_sound(&stop_ctx, audio_feedback::SoundType::Stop);
    });
    test_sound_row.add_suffix(&play_stop_btn);

    // Feedback Volume
    let vol_adj = gtk4::Adjustment::new(
        (settings.audio_feedback_volume * 100.0) as f64,
        0.0,
        100.0,
        5.0,
        10.0,
        0.0,
    );
    let vol_row = libadwaita::SpinRow::new(Some(&vol_adj), 5.0, 0);
    vol_row.set_title("Sound Feedback Volume (%)");
    let vol_icon = gtk4::Image::from_icon_name("audio-volume-medium-symbolic");
    vol_row.add_prefix(&vol_icon);
    vol_row.set_snap_to_ticks(true);
    vol_row.set_visible(settings.audio_feedback);

    let vol_ctx = ctx.clone();
    vol_adj.connect_value_changed(move |adj| {
        let mut s = vol_ctx.settings();
        s.audio_feedback_volume = (adj.value() / 100.0) as f32;
        vol_ctx.write_settings(&s);
    });

    let af_ctx = ctx.clone();
    let theme_weak = glib::SendWeakRef::from(theme_row.downgrade());
    let test_snd_weak = glib::SendWeakRef::from(test_sound_row.downgrade());
    let vol_weak = glib::SendWeakRef::from(vol_row.downgrade());
    audio_feedback_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        let mut s = af_ctx.settings();
        s.audio_feedback = is_active;
        af_ctx.write_settings(&s);

        if let Some(tr) = theme_weak.clone().into_weak_ref().upgrade() {
            tr.set_visible(is_active);
        }
        if let Some(ts) = test_snd_weak.clone().into_weak_ref().upgrade() {
            ts.set_visible(is_active);
        }
        if let Some(vr) = vol_weak.clone().into_weak_ref().upgrade() {
            vr.set_visible(is_active);
        }
    });

    feedback_group.add(&audio_feedback_row);
    feedback_group.add(&theme_row);
    feedback_group.add(&test_sound_row);
    feedback_group.add(&vol_row);

    // Mute while recording
    let mute_row = libadwaita::SwitchRow::new();
    mute_row.set_title("Mute System Audio While Recording");
    mute_row.set_subtitle("Pause background audio playback during active capture");
    let mute_icon = gtk4::Image::from_icon_name("audio-volume-muted-symbolic");
    mute_row.add_prefix(&mute_icon);
    mute_row.set_active(settings.mute_while_recording);
    let mute_ctx = ctx.clone();
    mute_row.connect_active_notify(move |row| {
        let mut s = mute_ctx.settings();
        s.mute_while_recording = row.is_active();
        mute_ctx.write_settings(&s);
    });
    feedback_group.add(&mute_row);

    page.add(&feedback_group);

    page.upcast::<gtk4::Widget>()
}

/// Populate the microphone combo asynchronously (cpal enumeration can stall).
fn populate_microphones(ctx: &AppContext, row: &libadwaita::ComboRow) {
    let ctx = ctx.clone();
    let row_weak = glib::SendWeakRef::from(row.downgrade());
    crate::runtime::spawn(async move {
        let devices = commands::audio::get_available_microphones().await;
        let row_weak = row_weak.clone();
        let ctx = ctx.clone();
        glib::MainContext::default().invoke(move || {
            let Some(row) = row_weak.into_weak_ref().upgrade() else {
                return;
            };
            match devices {
                Ok(devices) => {
                    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
                    let model = gtk4::StringList::new(&names);
                    row.set_model(Some(&model));
                    let current = ctx.settings().selected_microphone.unwrap_or_default();
                    if let Some(i) = names.iter().position(|n| *n == current) {
                        row.set_selected(i as u32);
                    }
                    let ctx_for_select = ctx.clone();
                    row.connect_selected_notify(move |row| {
                        if let Some(item) = row.selected_item() {
                            let name = item
                                .downcast_ref::<gtk4::StringObject>()
                                .map(|s| s.string().to_string())
                                .unwrap_or_default();
                            let ctx = ctx_for_select.clone();
                            glib::spawn_future_local(async move {
                                if let Err(err) =
                                    commands::audio::set_selected_microphone(&ctx, name).await
                                {
                                    ctx.report_error("set_selected_microphone", err);
                                }
                            });
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to enumerate microphones: {}", e);
                }
            }
        });
    });
}

/// Populate system audio monitor sources combo asynchronously.
fn populate_system_audio_sources(ctx: &AppContext, row: &libadwaita::ComboRow) {
    let ctx = ctx.clone();
    let row_weak = glib::SendWeakRef::from(row.downgrade());
    crate::runtime::spawn(async move {
        let devices = commands::audio::get_available_system_audio_sources().await;
        let row_weak = row_weak.clone();
        let ctx = ctx.clone();
        glib::MainContext::default().invoke(move || {
            let Some(row) = row_weak.into_weak_ref().upgrade() else {
                return;
            };
            match devices {
                Ok(devices) => {
                    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
                    let model = gtk4::StringList::new(&names);
                    row.set_model(Some(&model));
                    let current = ctx
                        .settings()
                        .selected_system_audio_device
                        .unwrap_or_default();
                    if let Some(i) = names.iter().position(|n| *n == current) {
                        row.set_selected(i as u32);
                    }
                    let ctx_for_select = ctx.clone();
                    row.connect_selected_notify(move |row| {
                        if let Some(item) = row.selected_item() {
                            let name = item
                                .downcast_ref::<gtk4::StringObject>()
                                .map(|s| s.string().to_string())
                                .unwrap_or_default();
                            let ctx = ctx_for_select.clone();
                            glib::spawn_future_local(async move {
                                let _ =
                                    commands::audio::set_selected_system_audio_device(&ctx, name)
                                        .await;
                            });
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to enumerate system audio sources: {}", e);
                }
            }
        });
    });
}
