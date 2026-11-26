//! Desktop-wide call shortcuts: push-to-talk, toggle mute and toggle deafen
//! while another application has focus.
//!
//! Every backend turns key presses into the same [`Event`]s: the XDG
//! GlobalShortcuts portal on Wayland ([`portal`]) and XInput2 raw key events on
//! X11 ([`x11`]). Windows' low-level keyboard hook (plan step P1) will be a third.
//! The in-window Right Ctrl handler in `main.rs` stays as the fallback. Both
//! sources feed one talk key here, so a focus change only ends an in-window hold.
mod portal;
mod x11;

use crate::Msg;
use iced::{Subscription, Task, futures::Stream};

/// Shortcut ids. The portal stores the user's bindings under them.
const TALK: &str = "push-to-talk";
const MUTE: &str = "toggle-mute";
const DEAFEN: &str = "toggle-deafen";

/// A shortcut and its default triggers in the XDG shortcuts format
/// ("CTRL+SHIFT+m", "Control_R"). The Voice & Video settings page (U2) will
/// make them editable.
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

fn shortcuts() -> [Shortcut; 3] {
    // Until U2, UNTETHERED_PTT_KEY replaces the push-to-talk trigger, in the
    // same format, for both backends.
    let talk = std::env::var("UNTETHERED_PTT_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
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

/// Desktop-wide shortcuts while `enabled` (push-to-talk is armed). On Wayland,
/// starting them can show the compositor's binding dialog, so they don't run
/// for people who never use push-to-talk. Setting
/// `UNTETHERED_GLOBAL_SHORTCUTS=0` keeps only the in-window key.
pub fn subscription(enabled: bool) -> Subscription<Event> {
    let off = std::env::var("UNTETHERED_GLOBAL_SHORTCUTS").is_ok_and(|v| v == "0");
    if enabled && !off {
        Subscription::run(listen)
    } else {
        Subscription::none()
    }
}

fn listen() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async |mut out| {
        use iced::futures::SinkExt;
        let shortcuts = shortcuts();
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

#[derive(Clone, Copy)]
pub enum Source {
    /// Right Ctrl while Untethered has focus.
    Window,
    /// A desktop-wide backend.
    Global,
}

/// The talk key as both sources see it, for the call's talk gate.
#[derive(Default)]
pub struct State {
    /// The desktop-wide push-to-talk trigger, while a backend has one bound.
    talk: Option<String>,
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
    pub fn hint(&self, held: bool) -> String {
        match (&self.talk, held) {
            (Some(key), true) if self.global_down => {
                format!("{key} held · release to stop talking.")
            }
            (_, true) => "Right Ctrl held · release to stop talking.".into(),
            (Some(key), false) => format!("Hold {key} to talk."),
            (None, false) => "Hold Right Ctrl to talk while Untethered is focused.".into(),
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
        if down == before || !call.group || call.cancel.is_closed() {
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
        app.room_ptt = true;
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
        assert_eq!(
            keys.hint(false),
            "Hold Right Ctrl to talk while Untethered is focused."
        );
        keys.talk = Some("Pause".into());
        assert_eq!(keys.hint(false), "Hold Pause to talk.");
        keys.window_down = true;
        assert_eq!(
            keys.hint(true),
            "Right Ctrl held · release to stop talking."
        );
        keys.global_down = true;
        assert_eq!(keys.hint(true), "Pause held · release to stop talking.");
    }
}
