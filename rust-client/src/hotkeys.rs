//! Desktop-wide call shortcuts: push-to-talk, toggle mute and toggle deafen
//! while another application has focus.
//!
//! Every backend turns key presses into the same [`Event`]s: the XDG
//! GlobalShortcuts portal on Wayland ([`portal`]) and XInput2 raw key events on
//! X11 ([`x11`]). Windows' low-level keyboard hook (plan step P1) will be a third.
//! The in-window key ([`window`]) stays as the fallback. Both sources feed one
//! talk key here, so a focus change only ends an in-window hold.
//!
//! The push-to-talk key is the one chosen on the Voice & Video page (U2),
//! saved in call_settings.rs; the page's binding dialog names keys with
//! [`name`] and [`combine`].
mod portal;
mod x11;

use crate::Msg;
use iced::{Subscription, Task, futures::Stream};

/// Shortcut ids. The portal stores the user's bindings under them.
const TALK: &str = "push-to-talk";
const MUTE: &str = "toggle-mute";
const DEAFEN: &str = "toggle-deafen";

/// A shortcut and its triggers in the XDG shortcuts format ("CTRL+SHIFT+m",
/// "Control_R").
struct Shortcut {
    id: &'static str,
    description: &'static str,
    /// Suggested to the portal; the compositor's dialog has the final say.
    portal: String,
    /// Listened for on X11. Raw events don't take the key from other
    /// applications, so only push-to-talk listens by default.
    x11: Option<String>,
}

/// KWin's kglobalaccel fires a modifier-only shortcut (Right Ctrl alone) when
/// the modifier is released, which can't hold a microphone open, so the
/// portal is offered a plain key.
const PORTAL_TALK: &str = "Pause";

/// The push-to-talk key in the window when none is chosen: Right Ctrl, as
/// on X11.
const WINDOW_TALK: &str = "Control_R";

/// The push-to-talk key chosen on the Voice & Video page, else
/// UNTETHERED_PTT_KEY (a fixture's), in the same format.
fn chosen(saved: Option<&str>) -> Option<String> {
    saved
        .map(str::to_owned)
        .or_else(|| std::env::var("UNTETHERED_PTT_KEY").ok())
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

/// The key that holds push-to-talk while Untethered has focus.
pub fn window_key(saved: Option<&str>) -> String {
    chosen(saved).unwrap_or_else(|| WINDOW_TALK.into())
}

fn shortcuts(saved: Option<&str>) -> [Shortcut; 3] {
    let talk = chosen(saved);
    [
        Shortcut {
            id: TALK,
            description: "Push to talk",
            portal: talk.clone().unwrap_or_else(|| PORTAL_TALK.into()),
            x11: Some(talk.unwrap_or_else(|| "Control_R".into())),
        },
        Shortcut {
            id: MUTE,
            description: "Toggle mute",
            portal: "CTRL+SHIFT+m".into(),
            x11: None,
        },
        Shortcut {
            id: DEAFEN,
            description: "Toggle deafen",
            portal: "CTRL+SHIFT+d".into(),
            x11: None,
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Desktop-wide shortcuts are live. Names the push-to-talk trigger, if one
    /// is bound.
    Ready(Option<String>),
    /// No desktop-wide shortcuts; the in-window key still works.
    Unavailable(String),
    Talk(bool),
    ToggleMute,
    ToggleDeafen,
}

/// Desktop-wide shortcuts while `enabled` (push-to-talk is armed), with the
/// chosen push-to-talk key. On Wayland, starting them can show the
/// compositor's binding dialog, so they don't run for people who never use
/// push-to-talk. Setting `UNTETHERED_GLOBAL_SHORTCUTS=0` keeps only the
/// in-window key. Another key starts them again.
pub fn subscription(enabled: bool, key: Option<String>) -> Subscription<Event> {
    let off = std::env::var("UNTETHERED_GLOBAL_SHORTCUTS").is_ok_and(|v| v == "0");
    if enabled && !off {
        Subscription::run_with(key, |key| listen(key.clone()))
    } else {
        Subscription::none()
    }
}

/// The push-to-talk key while Untethered has focus, while `enabled`: the
/// fallback for desktops without global shortcuts, and on Wayland, where
/// the compositor's key is separate.
pub fn window(enabled: bool, key: String) -> Subscription<Msg> {
    if !enabled {
        return Subscription::none();
    }
    // A modifier doesn't repeat; an ordinary key's repeats are ignored, so
    // the first repeat after refocus can't look like a fresh press. Release
    // and blur are always handled, even in an editor.
    iced::event::listen_with(|event, _, _| match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            repeat: false,
            ..
        }) => Some((true, name(&key, physical_key)?, modifiers)),
        iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
            key,
            physical_key,
            modifiers,
            ..
        }) => Some((false, name(&key, physical_key)?, modifiers)),
        _ => None,
    })
    .with(key)
    .filter_map(|(trigger, (down, name, modifiers))| {
        hits(&trigger, &name, modifiers, down).then_some(Msg::TalkKey(down))
    })
}

fn listen(key: Option<String>) -> impl Stream<Item = Event> {
    iced::stream::channel(8, async move |mut out| {
        use iced::futures::SinkExt;
        let shortcuts = shortcuts(key.as_deref());
        // On Wayland, X11 clients (XWayland) see no keys while a Wayland
        // window has focus, so only the portal can listen desktop-wide.
        let result = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            portal::run(&shortcuts, &mut out).await
        } else if std::env::var_os("DISPLAY").is_some() {
            x11::run(&shortcuts, &mut out).await
        } else {
            Err("no Wayland or X11 display".into())
        };
        if let Err(reason) = result {
            eprintln!("Desktop-wide call shortcuts are off: {reason}");
            let _ = out.send(Event::Unavailable(reason)).await;
        }
    })
}

/// Keys a trigger may name, by their X keysym name (the XDG trigger format),
/// with their keysym and how the call controls name them.
pub(crate) const NAMED: &[(&str, u32, &str)] = &[
    ("Control_R", 0xffe4, "Right Ctrl"),
    ("Control_L", 0xffe3, "Left Ctrl"),
    ("Shift_R", 0xffe2, "Right Shift"),
    ("Shift_L", 0xffe1, "Left Shift"),
    ("Alt_R", 0xffea, "Right Alt"),
    ("Alt_L", 0xffe9, "Left Alt"),
    ("ISO_Level3_Shift", 0xfe03, "AltGr"),
    ("Super_R", 0xffec, "Right Super"),
    ("Super_L", 0xffeb, "Left Super"),
    ("Caps_Lock", 0xffe5, "Caps Lock"),
    ("Pause", 0xff13, "Pause"),
    ("Scroll_Lock", 0xff14, "Scroll Lock"),
    ("Print", 0xff61, "Print"),
    ("Insert", 0xff63, "Insert"),
    ("Menu", 0xff67, "Menu"),
    ("Home", 0xff50, "Home"),
    ("End", 0xff57, "End"),
    ("Prior", 0xff55, "Page Up"),
    ("Next", 0xff56, "Page Down"),
    ("space", 0x20, "Space"),
    ("grave", 0x60, "`"),
];

/// The keys pressed alone that a binding takes as modifiers.
const MODIFIERS: &[&str] = &[
    "Control_R",
    "Control_L",
    "Shift_R",
    "Shift_L",
    "Alt_R",
    "Alt_L",
    "ISO_Level3_Shift",
    "Super_R",
    "Super_L",
];

pub fn is_modifier(name: &str) -> bool {
    MODIFIERS.contains(&name)
}

/// A key as a trigger names it: its X keysym name ("Control_R", "F9",
/// "t"), or None for a key push-to-talk can't use. Keys that differ by side
/// or have no character go by where they are; the rest by their character.
pub fn name(key: &iced::keyboard::Key, physical: iced::keyboard::key::Physical) -> Option<String> {
    use iced::keyboard::key::{Code, Named, Physical};
    if *key == iced::keyboard::Key::Named(Named::AltGraph) {
        return Some("ISO_Level3_Shift".into());
    }
    if let Physical::Code(code) = physical {
        let named = match code {
            Code::ControlRight => Some("Control_R"),
            Code::ControlLeft => Some("Control_L"),
            Code::ShiftRight => Some("Shift_R"),
            Code::ShiftLeft => Some("Shift_L"),
            Code::AltRight => Some("Alt_R"),
            Code::AltLeft => Some("Alt_L"),
            Code::SuperRight => Some("Super_R"),
            Code::SuperLeft => Some("Super_L"),
            Code::CapsLock => Some("Caps_Lock"),
            Code::Pause => Some("Pause"),
            Code::ScrollLock => Some("Scroll_Lock"),
            Code::PrintScreen => Some("Print"),
            Code::Insert => Some("Insert"),
            Code::ContextMenu => Some("Menu"),
            Code::Home => Some("Home"),
            Code::End => Some("End"),
            Code::PageUp => Some("Prior"),
            Code::PageDown => Some("Next"),
            Code::Space => Some("space"),
            Code::Backquote => Some("grave"),
            _ => None,
        };
        if let Some(named) = named {
            return Some(named.into());
        }
        const F: [Code; 24] = [
            Code::F1,
            Code::F2,
            Code::F3,
            Code::F4,
            Code::F5,
            Code::F6,
            Code::F7,
            Code::F8,
            Code::F9,
            Code::F10,
            Code::F11,
            Code::F12,
            Code::F13,
            Code::F14,
            Code::F15,
            Code::F16,
            Code::F17,
            Code::F18,
            Code::F19,
            Code::F20,
            Code::F21,
            Code::F22,
            Code::F23,
            Code::F24,
        ];
        if let Some(n) = F.iter().position(|&f| f == code) {
            return Some(format!("F{}", n + 1));
        }
    }
    let iced::keyboard::Key::Character(text) = key else {
        return None;
    };
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => Some(c.to_ascii_lowercase().to_string()),
        _ => None,
    }
}

/// An ordinary key with the modifiers held, as a trigger: "CTRL+SHIFT+t".
pub fn combine(name: &str, modifiers: iced::keyboard::Modifiers) -> String {
    let mut parts = Vec::new();
    for (held, part) in [
        (modifiers.control(), "CTRL"),
        (modifiers.shift(), "SHIFT"),
        (modifiers.alt(), "ALT"),
        (modifiers.logo(), "LOGO"),
    ] {
        if held {
            parts.push(part);
        }
    }
    parts.push(name);
    parts.join("+")
}

/// Whether a key event in the window is `trigger`'s: its key, and on a
/// press, its modifiers held too. A release of the key ends a hold whatever
/// is held.
fn hits(trigger: &str, name: &str, modifiers: iced::keyboard::Modifiers, down: bool) -> bool {
    let mut parts: Vec<&str> = trigger.split('+').map(str::trim).collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    if !key.eq_ignore_ascii_case(name) {
        return false;
    }
    !down
        || is_modifier(key)
        || parts.iter().all(|m| match m.to_ascii_uppercase().as_str() {
            "CTRL" => modifiers.control(),
            "SHIFT" => modifiers.shift(),
            "ALT" => modifiers.alt(),
            "LOGO" => modifiers.logo(),
            _ => false,
        })
}

/// How the call controls name a trigger: "Right Ctrl", "Ctrl+Shift+M".
pub fn label(trigger: &str) -> String {
    trigger
        .split('+')
        .map(|part| match part.to_ascii_uppercase().as_str() {
            "CTRL" => "Ctrl".into(),
            "SHIFT" => "Shift".into(),
            "ALT" => "Alt".into(),
            "NUM" => "Num".into(),
            "LOGO" => "Super".into(),
            _ => NAMED
                .iter()
                .find(|(n, ..)| *n == part)
                .map(|(.., label)| (*label).to_owned())
                .unwrap_or_else(|| part.to_uppercase()),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[derive(Clone, Copy)]
pub enum Source {
    /// The push-to-talk key while Untethered has focus.
    Window,
    /// A desktop-wide backend.
    Global,
}

/// The talk key as both sources see it, for the call's talk gate.
#[derive(Default)]
pub struct State {
    /// The desktop-wide push-to-talk trigger, while a backend has one bound.
    pub talk: Option<String>,
    window_down: bool,
    global_down: bool,
}
impl State {
    fn down(&self) -> bool {
        self.window_down || self.global_down
    }
    /// Push-to-talk was turned on or off: the subscription restarts and
    /// reports again. Forget a hold neither source will now end.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// The call panel's note, `window` being the in-window key's label.
    pub fn hint(&self, held: bool, window: &str) -> String {
        match (&self.talk, held) {
            (Some(key), true) if self.global_down => {
                format!("{key} held · release to stop talking.")
            }
            (_, true) => format!("{window} held · release to stop talking."),
            (Some(key), false) => format!("Hold {key} to talk."),
            (None, false) => format!("Hold {window} to talk while Untethered is focused."),
        }
    }
}

impl crate::App {
    pub(crate) fn hotkey(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Ready(talk) => {
                self.hotkeys.talk = talk;
                Task::none()
            }
            Event::Unavailable(_) => {
                self.hotkeys.talk = None;
                self.talk_key(Source::Global, false)
            }
            Event::Talk(down) => self.talk_key(Source::Global, down),
            Event::ToggleMute => self.update(Msg::ToggleMic),
            Event::ToggleDeafen => self.update(Msg::ToggleDeafen),
        }
    }

    /// One source pressed or released the talk key. The gate only sees edges
    /// of both sources together, so a key both report (Right Ctrl on X11
    /// while focused) counts once, and a repeat can't reopen a gate that a
    /// manual mute closed during the hold.
    pub(crate) fn talk_key(&mut self, source: Source, down: bool) -> Task<Msg> {
        let keys = &mut self.hotkeys;
        let before = keys.down();
        match source {
            Source::Window => keys.window_down = down,
            Source::Global => keys.global_down = down,
        }
        let down = keys.down();
        let Some(call) = &self.call else {
            return Task::none();
        };
        if down == before || call.cancel.is_closed() {
            return Task::none();
        }
        call.audio.send_if_modified(|s| s.hold(down));
        match call.audio.borrow().release_due() {
            // The timer starts on the executor, not in `update`.
            Some(until) => Task::perform(
                async move { tokio::time::sleep_until(until).await },
                move |()| Msg::TalkRelease(until),
            ),
            None => Task::none(),
        }
    }

    pub(crate) fn talk_release(&mut self, until: tokio::time::Instant) {
        if let Some(call) = &self.call {
            call.audio.send_if_modified(|s| s.finish_release(until));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_call::TalkGate;

    fn voice_room_with_ptt() -> crate::App {
        let (commands, _backend) = async_channel::bounded(8);
        let mut app = crate::App::new(commands);
        app.voice.push_to_talk = true;
        app.start_voice("!room".into(), crate::desktop_call::Target::Channel);
        app
    }
    fn gate(app: &crate::App) -> TalkGate {
        app.call.as_ref().unwrap().audio.borrow().talk
    }
    fn release(app: &mut crate::App) {
        let until = app.call.as_ref().unwrap().audio.borrow().release_due();
        app.talk_release(until.expect("a pending release"));
    }

    #[test]
    fn a_global_hold_survives_focus_changes_and_releases_after_the_delay() {
        let mut app = voice_room_with_ptt();
        assert!(gate(&app) == TalkGate::Waiting);
        // X11 while focused: the window and XInput2 both see Right Ctrl.
        let _ = app.talk_key(Source::Global, true);
        let _ = app.talk_key(Source::Window, true);
        assert!(gate(&app) == TalkGate::Held);
        let _ = app.update(crate::Msg::WindowFocus(false));
        assert!(gate(&app) == TalkGate::Held);
        let _ = app.update(crate::Msg::WindowFocus(true));
        let _ = app.talk_key(Source::Global, false);
        // Capture stays open for the release delay, then closes.
        assert!(matches!(gate(&app), TalkGate::Releasing(_)));
        release(&mut app);
        assert!(gate(&app) == TalkGate::Waiting);
    }

    #[test]
    fn a_press_during_the_release_delay_keeps_talking() {
        let mut app = voice_room_with_ptt();
        let _ = app.talk_key(Source::Global, true);
        let _ = app.talk_key(Source::Global, false);
        let until = app
            .call
            .as_ref()
            .unwrap()
            .audio
            .borrow()
            .release_due()
            .unwrap();
        let _ = app.talk_key(Source::Global, true);
        app.talk_release(until);
        assert!(gate(&app) == TalkGate::Held);
    }

    #[test]
    fn focus_loss_ends_an_in_window_hold_even_with_a_global_backend() {
        // Wayland: the portal key is separate, so Right Ctrl's release in
        // another window would never arrive.
        let mut app = voice_room_with_ptt();
        let _ = app.hotkey(Event::Ready(Some("Pause".into())));
        let _ = app.talk_key(Source::Window, true);
        let _ = app.update(crate::Msg::WindowFocus(false));
        release(&mut app);
        assert!(gate(&app) == TalkGate::Waiting);
        // Refocusing never reopens capture.
        let _ = app.update(crate::Msg::WindowFocus(true));
        assert!(gate(&app) == TalkGate::Waiting);
    }

    #[test]
    fn manual_mute_and_deafen_override_the_key_until_a_fresh_press() {
        let mut app = voice_room_with_ptt();
        let _ = app.hotkey(Event::Talk(true));
        let _ = app.hotkey(Event::ToggleMute);
        assert!(gate(&app) == TalkGate::Waiting);
        let _ = app.hotkey(Event::ToggleMute);
        // Still held, but unmuting needs a fresh press, even if the window
        // reports the same key now.
        let _ = app.talk_key(Source::Window, true);
        assert!(gate(&app) == TalkGate::Waiting);
        let _ = app.talk_key(Source::Window, false);
        let _ = app.hotkey(Event::Talk(false));
        let _ = app.hotkey(Event::Talk(true));
        assert!(gate(&app) == TalkGate::Held);
        // Deafen closes capture at once, without the release delay.
        let _ = app.hotkey(Event::ToggleDeafen);
        assert!(gate(&app) == TalkGate::Waiting);
    }

    #[test]
    fn hints_name_the_key_that_works() {
        let mut keys = State::default();
        let window = "Right Ctrl";
        assert_eq!(
            keys.hint(false, window),
            "Hold Right Ctrl to talk while Untethered is focused."
        );
        keys.talk = Some("Pause".into());
        assert_eq!(keys.hint(false, window), "Hold Pause to talk.");
        keys.window_down = true;
        assert_eq!(
            keys.hint(true, window),
            "Right Ctrl held · release to stop talking."
        );
        keys.global_down = true;
        assert_eq!(
            keys.hint(true, window),
            "Pause held · release to stop talking."
        );
    }

    #[test]
    fn keys_are_named_as_triggers_and_matched_in_the_window() {
        use iced::keyboard::{
            Key, Modifiers,
            key::{Code, Named, Physical},
        };
        let code = Physical::Code;
        assert_eq!(
            name(&Key::Named(Named::Control), code(Code::ControlRight)).as_deref(),
            Some("Control_R")
        );
        assert_eq!(
            name(&Key::Named(Named::F9), code(Code::F9)).as_deref(),
            Some("F9")
        );
        assert_eq!(
            name(&Key::Character("T".into()), code(Code::KeyT)).as_deref(),
            Some("t")
        );
        assert_eq!(
            name(&Key::Named(Named::Escape), code(Code::Escape)),
            None,
            "Escape cancels the dialog instead"
        );
        assert_eq!(
            combine("t", Modifiers::CTRL | Modifiers::SHIFT),
            "CTRL+SHIFT+t"
        );
        assert_eq!(label("CTRL+SHIFT+t"), "Ctrl+Shift+T");
        // A combination presses with its modifiers and releases with its key.
        assert!(hits("CTRL+t", "t", Modifiers::CTRL, true));
        assert!(!hits("CTRL+t", "t", Modifiers::empty(), true));
        assert!(hits("CTRL+t", "t", Modifiers::empty(), false));
        assert!(!hits("CTRL+t", "u", Modifiers::CTRL, true));
        // A modifier alone: holding it sets its own modifier.
        assert!(hits("Control_R", "Control_R", Modifiers::CTRL, true));
        assert!(!hits("Control_R", "Control_L", Modifiers::CTRL, true));
        assert_eq!(window_key(Some("F9")), "F9");
    }
}
