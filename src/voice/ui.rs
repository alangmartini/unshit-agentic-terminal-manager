use super::{Settings, VoiceState};
use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};
use unshit::core::element::*;

fn text(value: impl Into<String>) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("voice-help")
        .with_text(value)
}
fn button(label: &str, command: &str, shared: &SharedState) -> ElementDef {
    let s = shared.clone();
    let command = command.to_string();
    ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_text(label)
        .on_click(move || {
            mutate_with(&s, |st| {
                dispatch(st, &command);
            });
        })
}
fn field(
    label: &str,
    id: &str,
    value: &str,
    shared: &SharedState,
    set: fn(&mut Settings, String),
) -> ElementDef {
    let s = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("voice-field")
        .with_child(text(label))
        .with_child(
            ElementDef::new(Tag::Input)
                .with_class("input-text")
                .with_id(format!("voice-{id}"))
                .with_value(value)
                .on_change(move |v| {
                    mutate_with(&s, |st| set(&mut st.voice.settings, v.to_string()));
                }),
        )
}
fn toggle(label: &str, active: bool, shared: &SharedState, set: fn(&mut Settings)) -> ElementDef {
    let s = shared.clone();
    ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_text(format!("{} {label}", if active { "✓" } else { "○" }))
        .on_click(move || {
            mutate_with(&s, |st| {
                let custom = st.voice.settings.custom;
                set(&mut st.voice.settings);
                if custom != st.voice.settings.custom {
                    st.voice.api_key_draft.clear();
                    st.voice.key_saved = false;
                    dispatch(st, "voice.refresh");
                }
            });
        })
}
fn row() -> ElementDef {
    ElementDef::new(Tag::Div).with_class("voice-actions")
}
fn section(title: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("voice-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("voice-title")
                .with_text(title),
        )
}

pub fn settings(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let v = &snap.voice;
    let cfg = &v.settings;
    let mut provider = section("Transcription provider")
        .with_child(
            row()
                .with_child(toggle("OpenAI (default)", !cfg.custom, shared, |s| {
                    s.custom = false
                }))
                .with_child(toggle("Custom POST endpoint", cfg.custom, shared, |s| {
                    s.custom = true
                })),
        )
        .with_child(field("Model", "model", &cfg.model, shared, |s, v| {
            s.model = v
        }));
    if cfg.custom {
        provider=provider
            .with_child(field("Endpoint URL","endpoint",&cfg.endpoint,shared,|s,v|s.endpoint=v))
            .with_child(row().with_child(toggle("Multipart WAV upload",!cfg.json_body,shared,|s|s.json_body=false))
                .with_child(toggle("JSON body",cfg.json_body,shared,|s|s.json_body=true)))
            .with_child(field("Additional headers (JSON object; use {{api_key}} for the saved key)","headers",&cfg.headers,shared,|s,v|s.headers=v))
            .with_child(field(if cfg.json_body { "JSON body — {{audio_base64}} and {{model}} placeholders" } else { "Additional multipart fields (JSON object of strings)" },"body",&cfg.body,shared,|s,v|s.body=v))
            .with_child(text("JSON example: {\"audio\":\"{{audio_base64}}\",\"model\":\"{{model}}\"}. Multipart always includes file=voice.wav and model."))
            .with_child(field("Response JSON pointer (/text, /result/text); empty for plain text","pointer",&cfg.response_pointer,shared,|s,v|s.response_pointer=v));
    }
    let s = shared.clone();
    provider=provider
        .with_child(text("API key — stored in the system credential vault, separately for OpenAI and Custom. A custom endpoint receives only its own key."))
        .with_child(ElementDef::new(Tag::Input).with_class("input-text").with_id(format!("voice-api-key-{}-{}",cfg.custom,v.key_revision))
            .with_input_type(InputType::Password).with_placeholder("Paste a key, then Save voice settings")
            .on_change(move |value| { mutate_with(&s,|st|st.voice.api_key_draft=value.to_string()); }))
        .with_child(text(if v.key_saved { "A key is saved in the credential vault." } else { "No saved key detected for this provider." }))
        .with_child(button("Remove saved key","voice.forget_key",shared))
        .with_child(text("Light personal use can be around US$1/month. At US$0.006/min for gpt-4o-transcribe, about 167 min costs US$1. This is an estimate, not a monthly plan. Check actual charges in your OpenAI account."))
        .with_child(row().with_child(link("OpenAI usage","https://platform.openai.com/usage"))
            .with_child(link("Current pricing","https://developers.openai.com/api/docs/pricing")));
    let mut mic = section("Microphone lab")
        .with_child(text(format!(
            "Selected: {}",
            if cfg.microphone.is_empty() {
                "System default"
            } else {
                &cfg.microphone
            }
        )))
        .with_child(
            row()
                .with_child(button("Refresh microphones", "voice.refresh", shared))
                .with_child(toggle(
                    "System default",
                    cfg.microphone.is_empty(),
                    shared,
                    |s| s.microphone.clear(),
                )),
        );
    for (i, name) in v.devices.iter().enumerate() {
        let s = shared.clone();
        let name = name.clone();
        let chosen = cfg.microphone == name;
        mic = mic.with_child(
            ElementDef::new(Tag::Button)
                .with_id(format!("voice-device-{i}"))
                .with_class("btn")
                .with_text(format!("{} {name}", if chosen { "✓" } else { "○" }))
                .on_click(move || {
                    mutate_with(&s, |st| st.voice.settings.microphone = name.clone());
                }),
        );
    }
    mic=mic.with_child(text("Record a local sample and listen through your speakers/headphones. Test transcription records a new sample and sends it to the selected provider when stopped."))
        .with_child(row().with_child(button("Record local sample","voice.mic",shared))
            .with_child(button("Listen to sample","voice.play",shared))
            .with_child(button("Test transcription","voice.test",shared)))
        .with_child(controls(v,shared));
    let behavior=section("Dictation")
        .with_child(row().with_child(toggle("Copy final text to clipboard",cfg.clipboard,shared,|s|s.clipboard = !s.clipboard)))
        .with_child(row().with_child(toggle("Press to start / press to stop",!cfg.hold,shared,|s|s.hold=false))
            .with_child(toggle("Hold to talk",cfg.hold,shared,|s|s.hold=true)))
        .with_child(row().with_child(toggle("Transcribe only when finished (default)",!cfg.live,shared,|s|s.live=false))
            .with_child(toggle("Live dictation to current terminal / Codex CLI",cfg.live,shared,|s|s.live=true)))
        .with_child(text("Live mode inserts speech in roughly 6-second chunks plus network latency, into the terminal session focused when recording starts. It never presses Enter. Codex CLI and other chats running inside that terminal receive the text. External chat apps use clipboard paste."))
        .with_child(text("Final-only sends one recording; live mode makes multiple requests. Cost depends on the provider/model and audio duration, so final-only is not guaranteed to be cheaper. Recordings stop after 8 minutes."))
        .with_child(field("Global recording hotkey","hotkey",&cfg.hotkey,shared,|s,v|s.hotkey=v))
        .with_child(field("Global history hotkey","history-hotkey",&cfg.history_hotkey,shared,|s,v|s.history_hotkey=v))
        .with_child(text("Examples: Control+Alt+Space, Control+Alt+KeyV, Super+Shift+KeyV. Restart after saving new hotkeys. Hotkey conflicts appear below."))
        .with_child(text(&v.hotkey_status))
        .with_child(row().with_child(button("Start dictation","voice.toggle",shared))
            .with_child(button("Open history popup","voice.history",shared))
            .with_child(button("Save voice settings","voice.save",shared)));
    ElementDef::new(Tag::Div)
        .with_class("voice-settings")
        .with_child(provider)
        .with_child(mic)
        .with_child(behavior)
        .with_child(history(v, shared))
}
fn link(label: &str, url: &'static str) -> ElementDef {
    ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_text(label)
        .on_click(move || {
            let _ = crate::browser::open_url(url);
        })
}
fn controls(v: &VoiceState, shared: &SharedState) -> ElementDef {
    let bars = (v.peak / 5) as usize;
    section("Status")
        .with_child(text(&v.status))
        .with_child(text(format!(
            "{}s · microphone {}% {}{}",
            v.seconds,
            v.peak,
            "█".repeat(bars),
            "░".repeat(20 - bars.min(20))
        )))
        .with_child(
            row()
                .with_child(button("Stop and finish", "voice.stop", shared))
                .with_child(button("Cancel / stop playback", "voice.cancel", shared)),
        )
        .with_child(text(&v.preview))
}
fn history(v: &VoiceState, shared: &SharedState) -> ElementDef {
    let mut out=section("Voice history — latest 100")
        .with_child(text("Transcripts stay on this device. Raw recordings are not saved; the local microphone sample stays in memory until replaced or the app closes."))
        .with_child(button("Clear history","voice.clear",shared));
    if v.history.is_empty() {
        out = out.with_child(text("No transcriptions yet."));
    }
    for (index, entry) in v.history.iter().enumerate() {
        out = out.with_child(
            ElementDef::new(Tag::Div)
                .with_class(if v.history_open && index == v.selected {
                    "voice-entry voice-selected"
                } else {
                    "voice-entry"
                })
                .with_child(text(format!("{}s · {}", entry.seconds, entry.text)))
                .with_child(button("Copy", &format!("voice.copy:{}", entry.id), shared)),
        );
    }
    out
}
pub fn overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let v = &snap.voice;
    if v.history_open {
        ElementDef::new(Tag::Div)
            .with_class("voice-scrim")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("voice-popup")
                    .with_child(button("Close history (Esc)", "voice.close", shared))
                    .with_child(text("↑ / ↓ select · Enter copies and closes · Esc closes"))
                    .with_child(history(v, shared)),
            )
    } else if v.busy {
        ElementDef::new(Tag::Div)
            .with_class("voice-recording")
            .with_child(text(format!(
                "Voice · {}s · {}% · {}",
                v.seconds, v.peak, v.status
            )))
            .with_child(
                row()
                    .with_child(button("Stop", "voice.stop", shared))
                    .with_child(button("Cancel", "voice.cancel", shared)),
            )
    } else {
        ElementDef::new(Tag::Div).with_class("voice-hidden")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    fn visit(el: &ElementDef, f: &mut impl FnMut(&ElementDef)) {
        f(el);
        for child in &el.children {
            visit(child, f);
        }
    }
    #[test]
    fn provider_switch_discards_unsaved_key_and_key_input_is_masked() {
        let shared = Arc::new(Mutex::new(crate::state::seed_state()));
        shared.lock().unwrap().voice.api_key_draft = "secret fixture".into();
        let snap = shared.lock().unwrap().ui_snapshot();
        let tree = settings(&snap, &shared);
        let mut masked = false;
        visit(&tree, &mut |el| {
            if el.input_type == InputType::Password {
                masked = true;
            }
            if matches!(&el.content, ElementContent::Text(t) if t.contains("Custom POST endpoint"))
            {
                el.on_click.as_ref().unwrap()();
            }
        });
        assert!(masked);
        let state = shared.lock().unwrap();
        assert!(state.voice.settings.custom);
        assert!(state.voice.api_key_draft.is_empty());
    }
    #[test]
    fn history_keyboard_does_not_leak_keys_to_terminal() {
        let mut state = crate::state::seed_state();
        state.voice.history_open = true;
        state.voice.history = Arc::new(vec![
            super::super::Entry {
                id: 1,
                text: "hello".into(),
                seconds: 1,
            },
            super::super::Entry {
                id: 2,
                text: "world".into(),
                seconds: 1,
            },
        ]);
        assert!(super::super::history_key(
            &mut state,
            &unshit::core::shortcut::KeyCombo::parse("Down").unwrap()
        ));
        assert_eq!(state.voice.selected, 1);
        assert!(super::super::history_key(
            &mut state,
            &unshit::core::shortcut::KeyCombo::parse("Down").unwrap()
        ));
        assert_eq!(state.voice.selected, 1);
        assert!(super::super::history_key(
            &mut state,
            &unshit::core::shortcut::KeyCombo::parse("A").unwrap()
        ));
        assert!(super::super::history_key(
            &mut state,
            &unshit::core::shortcut::KeyCombo::parse("Escape").unwrap()
        ));
        assert!(!state.voice.history_open);
        assert!(!super::super::history_key(
            &mut state,
            &unshit::core::shortcut::KeyCombo::parse("A").unwrap()
        ));
    }
}
