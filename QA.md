# Otush QA checklist — Ubuntu 24.04 (GNOME 46, Wayland)

Run these after any UI/backend change. Check "PASS" or note the failure with
the log excerpt (`~/.local/share/com.clusterat.otush/logs/otush.log`).

## Build & launch
- [ ] `cargo build --release` completes with no errors
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] App window opens on the Wayland session (`$GDK_BACKEND=wayland`); verify
      no XWayland window: `xlsclients | grep -i otush` shows nothing while the
      app is visible
- [ ] `otush --list-models` and `otush --list-devices` work headlessly

## Onboarding & settings
- [ ] First run shows the welcome banner and the Models page is focused
- [ ] "Get Started" dismisses the banner and persists `onboarding_completed`
- [ ] Each sidebar section (General, Models, Post-Processing, History,
      Advanced, Debug, About) renders and its controls persist across restart
- [ ] Language / theme changes apply immediately (dark/light/system)

## Recording loop (the core path)
- [ ] Global shortcut (or push-to-talk) starts recording; the overlay pill
      appears at the configured edge with a live mic-level bar
- [ ] Releasing/stopping transcribes; overlay shows "Transcribing…" then hides
- [ ] Text is pasted into the active application (GNOME Wayland clipboard path)
- [ ] Model auto-unload fires per the configured timeout
- [ ] Cancel (`Escape` binding or tray Cancel) aborts recording mid-flight

## Tray
- [ ] StatusNotifierItem appears (needs the AppIndicator extension; Ubuntu's
      GNOME session ships it)
- [ ] Menu: Settings raises the window; Copy Last Transcript; Unload Model;
      model submenu switches models; Quit exits
- [ ] Tray icon switches between idle/recording/transcribing states
      (blue→violet brand icons, light/dark variants)

## CLI remote control
- [ ] With the app running: `otush --toggle-transcription` starts recording
      (and toggles it off)
- [ ] `otush --toggle-post-process` transcribes with post-processing
- [ ] `otush --cancel` cancels; `otush` (no flags) raises the window
- [ ] `otush --start-hidden` launches minimized to tray

## Wayland specifics
- [ ] Overlay is a layer-shell surface (top/bottom) that does not steal focus
      and does not reserve space (`exclusive zone` 0)
- [ ] Global shortcuts bind via the GlobalShortcuts portal (GNOME Settings
      shortcut configuration) or the evdev fallback
- [ ] Paste works in a terminal (direct typing) and a browser (clipboard)

## Data
- [ ] Settings persist across restarts in
      `~/.local/share/com.clusterat.otush/settings_store.json`
- [ ] Installed models and history DB live under
      `~/.local/share/com.clusterat.otush/`
