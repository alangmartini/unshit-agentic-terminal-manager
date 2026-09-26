//! Voice dictation belongs to the UI process; daemon PTYs remain fire-and-forget.
mod audio;
pub mod provider;
pub mod ui;

use crate::state::{AppState, MutexExt, SharedState};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub custom: bool,
    pub endpoint: String,
    pub model: String,
    pub headers: String,
    pub body: String,
    pub json_body: bool,
    pub response_pointer: String,
    pub microphone: String,
    pub clipboard: bool,
    pub live: bool,
    pub hold: bool,
    pub hotkey: String,
    pub history_hotkey: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            custom: false,
            endpoint: provider::OPENAI_ENDPOINT.into(),
            model: "gpt-4o-transcribe".into(),
            headers: "{}".into(),
            body: "{}".into(),
            json_body: false,
            response_pointer: "/text".into(),
            microphone: String::new(),
            clipboard: true,
            live: false,
            hold: false,
            hotkey: "Control+Alt+Space".into(),
            history_hotkey: "Control+Alt+KeyV".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub text: String,
    pub seconds: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    settings: Settings,
    history: Vec<Entry>,
}

#[derive(Clone)]
pub struct VoiceState {
    pub settings: Settings,
    pub history: Arc<Vec<Entry>>,
    pub devices: Vec<String>,
    pub status: String,
    pub hotkey_status: String,
    pub recording: bool,
    pub busy: bool,
    pub peak: u32,
    pub seconds: u64,
    pub history_open: bool,
    pub api_key_draft: String,
    pub key_saved: bool,
    pub key_revision: u64,
    pub selected: usize,
    pub preview: String,
    pub debug_audio: Arc<Vec<i16>>,
    pub stop: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}
impl Default for VoiceState {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            history: Arc::default(),
            devices: Vec::new(),
            status: "Ready. Test your microphone before dictating.".into(),
            hotkey_status: String::new(),
            recording: false,
            busy: false,
            peak: 0,
            seconds: 0,
            history_open: false,
            api_key_draft: String::new(),
            key_saved: false,
            key_revision: 0,
            selected: 0,
            preview: String::new(),
            debug_audio: Arc::default(),
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl std::fmt::Debug for VoiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceState")
            .field("recording", &self.recording)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}
pub struct Hooks {
    pub shared: SharedState,
    pub rebuild: Box<dyn Fn() + Send + Sync>,
    pub activate: Box<dyn Fn() + Send + Sync>,
}
static HOOKS: OnceLock<Arc<Hooks>> = OnceLock::new();
fn update(h: &Hooks, f: impl FnOnce(&mut AppState)) {
    {
        let mut s = h.shared.lock_recover();
        f(&mut s);
    }
    (h.rebuild)();
}
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|p| p.join("voice.json"))
}
fn credential(custom: bool) -> Result<keyring::Entry, String> {
    keyring::Entry::new(
        &format!(
            "terminal-manager.voice.{}",
            crate::profile::active_profile().unwrap_or("default")
        ),
        if custom { "custom" } else { "openai" },
    )
    .map_err(|_| "System credential store unavailable".into())
}
fn key(custom: bool) -> Result<String, String> {
    match credential(custom)?.get_password() {
        Ok(k) => Ok(k),
        Err(keyring::Error::NoEntry) if custom => Ok(String::new()),
        Err(keyring::Error::NoEntry) => {
            Err("Save an OpenAI API key in Voice settings first".into())
        }
        Err(_) => Err("Cannot read API key from system credential store".into()),
    }
}
fn save(stored: &Stored) -> Result<(), String> {
    let path = path().ok_or("No configuration directory")?;
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|_| "Cannot create voice settings directory")?;
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(&tmp)
        .map_err(|_| "Cannot save voice settings")?;
    file.write_all(&serde_json::to_vec_pretty(stored).map_err(|_| "Cannot encode voice settings")?)
        .map_err(|_| "Cannot write voice settings")?;
    file.sync_all().map_err(|_| "Cannot sync voice settings")?;
    std::fs::rename(tmp, path).map_err(|_| "Cannot replace voice settings".to_string())
}
fn persist(h: &Hooks) -> Result<(), String> {
    static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _save = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let stored = {
        let s = h.shared.lock_recover();
        Stored {
            settings: s.voice.settings.clone(),
            history: (*s.voice.history).clone(),
        }
    };
    save(&stored)
}

/// Called on the event-loop thread; global-hotkey requires this on macOS/Windows.
/// Keep the manager alive until the application exits.
pub fn start(hooks: Hooks) -> Option<GlobalHotKeyManager> {
    let h = Arc::new(hooks);
    let _ = HOOKS.set(h.clone());
    if let Some(path) = path() {
        if let Ok(bytes) = std::fs::read(path) {
            match serde_json::from_slice::<Stored>(&bytes) {
                Ok(mut stored) => {
                    stored.history.truncate(100);
                    let mut s = h.shared.lock_recover();
                    s.voice.settings = stored.settings;
                    s.voice.history = Arc::new(stored.history);
                }
                Err(_) => {
                    h.shared.lock_recover().voice.status =
                        "Cannot load voice.json; using defaults".into()
                }
            }
        }
    }
    let settings = h.shared.lock_recover().voice.settings.clone();
    let registration = (|| -> Result<_, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let record: HotKey = settings
            .hotkey
            .parse()
            .map_err(|_| "Invalid recording hotkey")?;
        let history: HotKey = settings
            .history_hotkey
            .parse()
            .map_err(|_| "Invalid history hotkey")?;
        if record.id() == history.id() {
            return Err("Recording and history hotkeys must differ".into());
        }
        manager
            .register(record)
            .map_err(|e| format!("Recording hotkey unavailable: {e}"))?;
        if let Err(e) = manager.register(history) {
            let _ = manager.unregister(record);
            return Err(format!("History hotkey unavailable: {e}"));
        }
        let events_h = h.clone();
        std::thread::spawn(move || {
            let mut down = false;
            let mut held_recording = false;
            while let Ok(event) = GlobalHotKeyEvent::receiver().recv() {
                if event.id == history.id() && event.state == HotKeyState::Pressed {
                    (events_h.activate)();
                }
                update(&events_h, |s| {
                    if event.id == history.id() && event.state == HotKeyState::Pressed {
                        s.voice.history_open = !s.voice.history_open;
                        s.voice.selected = 0;
                    }
                    if event.id == record.id() {
                        match event.state {
                            HotKeyState::Pressed if !down => {
                                down = true;
                                held_recording = s.voice.settings.hold;
                                dispatch(s, "voice.toggle");
                            }
                            HotKeyState::Released => {
                                down = false;
                                if held_recording && s.voice.recording {
                                    dispatch(s, "voice.stop");
                                }
                            }
                            _ => (),
                        }
                    }
                });
            }
        });
        Ok(manager)
    })();
    let manager = match registration {
        Ok(manager) => {
            h.shared.lock_recover().voice.hotkey_status =
                "Global hotkeys active (also while other apps are focused).".into();
            Some(manager)
        }
        Err(e) => {
            h.shared.lock_recover().voice.hotkey_status = e;
            None
        }
    };
    refresh(h);
    manager
}
fn refresh(h: Arc<Hooks>) {
    std::thread::spawn(move || {
        let devices = audio::devices();
        let custom = h.shared.lock_recover().voice.settings.custom;
        let saved = credential(custom)
            .and_then(|e| e.get_password().map_err(|_| String::new()))
            .is_ok();
        update(&h, |s| {
            if s.voice.settings.custom == custom {
                s.voice.key_saved = saved;
            }
            match devices {
                Ok(d) => s.voice.devices = d,
                Err(e) => s.voice.status = e,
            }
        });
    });
}

/// The history popup owns keyboard input, including keys that would otherwise reach a PTY.
pub fn history_key(state: &mut AppState, combo: &unshit::core::shortcut::KeyCombo) -> bool {
    use unshit::core::event::Key;
    if !state.voice.history_open {
        return false;
    }
    match combo.key {
        Key::Escape => state.voice.history_open = false,
        Key::ArrowDown => {
            state.voice.selected =
                (state.voice.selected + 1).min(state.voice.history.len().saturating_sub(1))
        }
        Key::ArrowUp => state.voice.selected = state.voice.selected.saturating_sub(1),
        Key::Enter => {
            if let Some(entry) = state.voice.history.get(state.voice.selected) {
                let command = format!("voice.copy:{}", entry.id);
                dispatch(state, &command);
                state.voice.history_open = false;
            }
        }
        _ => (),
    }
    true
}

pub fn dispatch(state: &mut AppState, command: &str) -> bool {
    match command {
        "voice.history" => {
            state.voice.history_open = !state.voice.history_open;
            state.voice.selected = 0;
        }
        "voice.close" => state.voice.history_open = false,
        "voice.stop" => {
            if !state.voice.busy {
                return true;
            }
            if !state.voice.recording {
                state.voice.cancel.store(true, Ordering::Relaxed);
            }
            state.voice.stop.store(true, Ordering::Relaxed);
            state.voice.status = "Finishing recording…".into();
        }
        "voice.cancel" => {
            if !state.voice.busy {
                return true;
            }
            state.voice.status =
                "Cancelling… An in-flight request may take up to 90 seconds to finish.".into();
            state.voice.cancel.store(true, Ordering::Relaxed);
            state.voice.stop.store(true, Ordering::Relaxed);
        }
        "voice.toggle" if state.voice.recording => {
            state.voice.stop.store(true, Ordering::Relaxed);
        }
        "voice.toggle" | "voice.test" | "voice.mic" => {
            if state.voice.busy {
                return true;
            }
            let Some(h) = HOOKS.get().cloned() else {
                state.voice.status = "Voice service is not running".into();
                return true;
            };
            let debug = command == "voice.mic";
            let mut settings = state.voice.settings.clone();
            if command == "voice.test" || debug {
                settings.live = false;
            }
            if !debug {
                if let Err(e) = provider::validate(&settings) {
                    state.voice.status = e;
                    return true;
                }
            }
            let target = if settings.live {
                let pane = state.active_pane.0;
                match state.pty_manager.session_id(pane) {
                    Some(session) => Some((pane, session)),
                    None => {
                        state.voice.status =
                            "Focus a terminal or Codex CLI pane before starting live dictation"
                                .into();
                        return true;
                    }
                }
            } else {
                None
            };
            state.voice.stop = Arc::new(AtomicBool::new(false));
            state.voice.cancel = Arc::new(AtomicBool::new(false));
            let stop = state.voice.stop.clone();
            let cancel = state.voice.cancel.clone();
            state.voice.busy = true;
            state.voice.recording = true;
            state.voice.seconds = 0;
            state.voice.preview.clear();
            state.voice.status = if debug {
                "Recording locally; no audio is sent"
            } else {
                "Opening microphone…"
            }
            .into();
            std::thread::spawn(move || {
                let result = record(&h, settings, target, debug, stop, cancel.clone());
                update(&h, |s| {
                    s.voice.recording = false;
                    s.voice.busy = false;
                    s.voice.peak = 0;
                    if cancel.load(Ordering::Relaxed) {
                        s.voice.status = "Cancelled. Text already inserted is kept.".into();
                    } else if let Err(e) = result {
                        s.voice.status = e;
                    }
                });
            });
        }
        "voice.refresh" => {
            if let Some(h) = HOOKS.get() {
                refresh(h.clone());
            }
        }
        "voice.save" => {
            if let Err(e) = provider::validate(&state.voice.settings) {
                state.voice.status = e;
                return true;
            }
            let parsed = (
                state.voice.settings.hotkey.parse::<HotKey>(),
                state.voice.settings.history_hotkey.parse::<HotKey>(),
            );
            match parsed {
                (Ok(a), Ok(b)) if a.id() != b.id() => (),
                _ => {
                    state.voice.status =
                        "Use two distinct valid global hotkeys, e.g. Control+Alt+Space".into();
                    return true;
                }
            }
            let draft = std::mem::take(&mut state.voice.api_key_draft);
            state.voice.key_revision += 1;
            let custom = state.voice.settings.custom;
            if let Some(h) = HOOKS.get().cloned() {
                std::thread::spawn(move || {
                    let result = (|| {
                        if !draft.is_empty() {
                            credential(custom)?
                                .set_password(&draft)
                                .map_err(|_| "Cannot save key in system credential store")?;
                        }
                        persist(&h)
                    })();
                    update(&h, |s| match result {
                        Ok(()) => {
                            s.voice.status =
                                "Saved. Restart the app to apply global hotkey changes.".into();
                            if !draft.is_empty() {
                                s.voice.key_saved = true;
                            }
                        }
                        Err(e) => s.voice.status = e,
                    });
                });
            }
        }
        "voice.forget_key" => {
            if let Some(h) = HOOKS.get().cloned() {
                let custom = state.voice.settings.custom;
                state.voice.api_key_draft.clear();
                state.voice.key_revision += 1;
                std::thread::spawn(move || {
                    let result = credential(custom).and_then(|e| match e.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                        Err(_) => Err("Cannot remove key from credential store".into()),
                    });
                    update(&h, |s| match result {
                        Ok(()) => {
                            s.voice.key_saved = false;
                            s.voice.status = "API key removed".into();
                        }
                        Err(e) => s.voice.status = e,
                    });
                });
            }
        }
        "voice.clear" => {
            state.voice.history = Arc::default();
            state.voice.selected = 0;
            if let Some(h) = HOOKS.get().cloned() {
                std::thread::spawn(move || {
                    if let Err(e) = persist(&h) {
                        update(&h, |s| s.voice.status = e);
                    }
                });
            }
        }
        "voice.play" => {
            if state.voice.busy || state.voice.debug_audio.is_empty() {
                return true;
            }
            if let Some(h) = HOOKS.get().cloned() {
                state.voice.busy = true;
                state.voice.cancel = Arc::new(AtomicBool::new(false));
                let cancel = state.voice.cancel.clone();
                let audio = state.voice.debug_audio.clone();
                std::thread::spawn(move || {
                    let result = audio::playback(audio, cancel);
                    update(&h, |s| {
                        s.voice.busy = false;
                        s.voice.status = result
                            .map(|_| "Playback complete".into())
                            .unwrap_or_else(|e| e);
                    });
                });
            }
        }
        _ => {
            if let Some(id) = command
                .strip_prefix("voice.copy:")
                .and_then(|s| s.parse::<u64>().ok())
            {
                if let Some(entry) = state.voice.history.iter().find(|e| e.id == id) {
                    state.voice.status = match state.clipboard.write_text(&entry.text) {
                        Ok(()) => "Copied to clipboard".into(),
                        Err(_) => "Clipboard unavailable; transcript remains in history".into(),
                    };
                }
            } else {
                return false;
            }
        }
    }
    true
}

fn terminal_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn record(
    h: &Arc<Hooks>,
    settings: Settings,
    target: Option<(u32, u64)>,
    debug: bool,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let key = if debug {
        String::new()
    } else {
        key(settings.custom)?
    };
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let capture = audio::Capture::start(&settings.microphone)?;
    let start = Instant::now();
    let mut last_chunk = Instant::now();
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(8);
    let th = h.clone();
    let ts = settings.clone();
    let tc = cancel.clone();
    let transcriber = std::thread::spawn(move || -> Result<String, String> {
        let mut text = String::new();
        for samples in rx {
            if tc.load(Ordering::Relaxed) {
                break;
            }
            let part = provider::transcribe(&ts, &key, &samples, audio::RATE)?;
            if tc.load(Ordering::Relaxed) {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&part);
            let mut write_error = None;
            update(&th, |s| {
                s.voice.preview = text.clone();
                if let Some((pane, session)) = target {
                    if s.pty_manager.session_id(pane) != Some(session) {
                        write_error = Some(
                            "Dictation destination closed or changed; text remains in preview"
                                .to_string(),
                        );
                    } else {
                        // Never send control characters or a newline that could execute a shell command.
                        let safe = terminal_text(&part);
                        let bytes = format!("{safe} ").into_bytes();
                        if s.pty_manager.write(pane, &bytes).is_err() {
                            write_error = Some(
                                "Cannot write to dictation destination; text remains in preview"
                                    .into(),
                            );
                        }
                    }
                }
            });
            if let Some(e) = write_error {
                return Err(e);
            }
        }
        Ok(text)
    });
    let mut error = None;
    while !stop.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
        let seconds = start.elapsed().as_secs();
        update(h, |s| {
            s.voice.seconds = seconds;
            s.voice.peak = capture.peak.load(Ordering::Relaxed);
            s.voice.status = if debug {
                "Local microphone test (15s maximum)"
            } else {
                "Listening… Stop to finish, Cancel to discard"
            }
            .into();
        });
        if capture.failed.load(Ordering::Relaxed) {
            error = Some("Microphone disconnected or permission was revoked".into());
            break;
        }
        if settings.live && last_chunk.elapsed() >= Duration::from_secs(6) {
            let samples = capture.take();
            if !samples.is_empty() && tx.try_send(samples).is_err() {
                error = Some(
                    "Transcription cannot keep up; recording stopped. Try final-only mode.".into(),
                );
                break;
            }
            last_chunk = Instant::now();
        }
        if seconds >= if debug { 15 } else { audio::MAX_SECONDS as u64 }
            || capture.full.load(Ordering::Relaxed)
        {
            break;
        }
    }
    let samples = capture.finish();
    let recorded_seconds = start.elapsed().as_secs();
    update(h, |s| {
        s.voice.recording = false;
        s.voice.peak = 0;
        s.voice.status = "Transcribing…".into();
    });
    if debug {
        let samples = if cancel.load(Ordering::Relaxed) {
            Vec::new()
        } else {
            samples
        };
        update(h, |s| {
            s.voice.debug_audio = Arc::new(samples);
            s.voice.status = "Local sample ready. Press Listen to hear your microphone.".into();
        });
    } else if !cancel.load(Ordering::Relaxed)
        && error.is_none()
        && !samples.is_empty()
        && tx.send(samples).is_err()
    {
        error = Some("Transcription worker stopped".into());
    }
    drop(tx);
    let result = transcriber
        .join()
        .map_err(|_| "Transcription worker failed")?;
    let text = match result {
        Ok(text) => text,
        Err(e) => {
            error = Some(e);
            h.shared.lock_recover().voice.preview.clone()
        }
    };
    if !debug && !cancel.load(Ordering::Relaxed) && !text.is_empty() {
        update(h, |s| {
            let history = Arc::make_mut(&mut s.voice.history);
            history.insert(
                0,
                Entry {
                    id: now_ms(),
                    text: text.clone(),
                    seconds: recorded_seconds,
                },
            );
            history.truncate(100);
            s.voice.status = if settings.clipboard {
                match s.clipboard.write_text(&text) {
                    Ok(()) => "Copied to clipboard and saved to history".into(),
                    Err(_) => "Saved to history; clipboard unavailable".into(),
                }
            } else {
                "Saved to history (clipboard off)".into()
            };
        });
        persist(h)?;
    }
    if !debug && text.is_empty() && error.is_none() && !cancel.load(Ordering::Relaxed) {
        error =
            Some("No microphone audio was transcribed. Check the microphone and try again.".into());
    }
    if let Some(e) = error {
        Err(e)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_text_cannot_send_terminal_controls_or_submit() {
        let text = terminal_text("Olá\r\nrm -rf /\x1b[201~\x03\x7f");
        assert!(text.starts_with("Olá  rm -rf /"));
        assert!(!text.chars().any(char::is_control));
    }
    #[test]
    fn defaults_and_legacy_config() {
        let stored: Stored = serde_json::from_str("{}").unwrap();
        assert!(stored.settings.clipboard);
        assert!(!stored.settings.live);
        assert!(!stored.settings.custom);
        assert!(stored.settings.hotkey.parse::<HotKey>().is_ok());
        assert!(stored.settings.history_hotkey.parse::<HotKey>().is_ok());
        assert!(!serde_json::to_string(&stored).unwrap().contains("api_key"));
    }
}
