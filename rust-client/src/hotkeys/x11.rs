//! X11: XInput2 raw key events on the root window.
//!
//! Raw events reach every client that selects them, whichever window has
//! focus, even during another client's keyboard grab (XI 2.1). A passive key
//! grab would take the key from the focused application and, while held,
//! divert the rest of the keyboard to us too, so a game would stop seeing its
//! movement keys while you talk. Listening takes nothing away.
use super::{DEAFEN, Event, MUTE, Shortcut, TALK};
use async_channel::Sender;
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use x11rb::{
    connection::Connection,
    protocol::{
        Event as XEvent,
        xinput::{self, ConnectionExt as _},
        xproto::{self, ConnectionExt as _},
    },
    rust_connection::RustConnection,
};

type Out = iced::futures::channel::mpsc::Sender<Event>;
const ALL_MASTER_DEVICES: xinput::DeviceId = 1;

pub(super) async fn run(shortcuts: &[Shortcut], out: &mut Out) -> Result<(), String> {
    use iced::futures::SinkExt;
    let bindings: Vec<_> = shortcuts
        .iter()
        .filter_map(|s| Some((s.id, s.x11.clone()?)))
        .collect();
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let conn = Arc::new(conn);
    let root = conn.setup().roots[screen].root;
    let version = conn
        .xinput_xi_query_version(2, 2)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|_| "the X server has no XInput2")?;
    if (version.major_version, version.minor_version) < (2, 1) {
        return Err("raw key events need XInput 2.1".into());
    }
    conn.xinput_xi_select_events(
        root,
        &[xinput::EventMask {
            deviceid: ALL_MASTER_DEVICES,
            mask: vec![xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE],
        }],
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| e.to_string())?;
    let keys = Keys::load(&conn, &bindings)?;
    // An unmapped window of our own: a message to it wakes the listener when
    // the subscription ends.
    let wake = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_window(
        0,
        wake,
        root,
        0,
        0,
        1,
        1,
        0,
        xproto::WindowClass::INPUT_ONLY,
        0,
        &Default::default(),
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| e.to_string())?;
    let label = bindings
        .iter()
        .find(|(id, _)| *id == TALK)
        .map(|(_, trigger)| label(trigger));
    let (events, received) = async_channel::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let _waker = Waker {
        conn: conn.clone(),
        window: wake,
        stop: stop.clone(),
    };
    let listener = conn.clone();
    std::thread::Builder::new()
        .name("x11-hotkeys".into())
        .spawn(move || listen(&listener, keys, bindings, &events, &stop))
        .map_err(|e| e.to_string())?;
    out.send(Event::Ready(label))
        .await
        .map_err(|e| e.to_string())?;
    while let Ok(event) = received.recv().await {
        if out.send(event).await.is_err() {
            return Ok(());
        }
    }
    Err("lost the X11 connection".into())
}

struct Waker {
    conn: Arc<RustConnection>,
    window: xproto::Window,
    stop: Arc<AtomicBool>,
}
impl Drop for Waker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // With an empty event mask the message goes to the window's creator.
        let message =
            xproto::ClientMessageEvent::new(32, self.window, xproto::AtomEnum::NONE, [0u32; 5]);
        let _ = self
            .conn
            .send_event(false, self.window, xproto::EventMask::NO_EVENT, message);
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

fn listen(
    conn: &RustConnection,
    mut keys: Keys,
    bindings: Vec<(&'static str, String)>,
    events: &Sender<Event>,
    stop: &AtomicBool,
) {
    let mut pressed = HashSet::new();
    let mut held = vec![false; keys.combos.len()];
    while let Ok(event) = conn.wait_for_event() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match event {
            XEvent::XinputRawKeyPress(e) => {
                pressed.insert(e.detail);
            }
            XEvent::XinputRawKeyRelease(e) => {
                pressed.remove(&e.detail);
            }
            XEvent::MappingNotify(_) => {
                // The layout changed. Keys already held keep their codes.
                if let Ok(next) = Keys::load(conn, &bindings) {
                    keys = next;
                }
                continue;
            }
            _ => continue,
        }
        for ((id, combo), was) in keys.combos.iter().zip(&mut held) {
            let now = combo.held(&pressed);
            if now == *was {
                continue;
            }
            *was = now;
            let event = match *id {
                TALK => Event::Talk(now),
                MUTE if now => Event::ToggleMute,
                DEAFEN if now => Event::ToggleDeafen,
                _ => continue,
            };
            if events.send_blocking(event).is_err() {
                return;
            }
        }
    }
}

/// Bindings resolved to the keycodes that produce them on this keyboard.
struct Keys {
    combos: Vec<(&'static str, Combo)>,
}
struct Combo {
    /// Any of these keycodes is the key.
    key: Vec<u32>,
    /// For each required modifier, the keycodes that set it.
    modifiers: Vec<Vec<u32>>,
}
impl Combo {
    fn held(&self, pressed: &HashSet<u32>) -> bool {
        let any = |codes: &Vec<u32>| codes.iter().any(|c| pressed.contains(c));
        any(&self.key) && self.modifiers.iter().all(any)
    }
}
impl Keys {
    fn load(conn: &RustConnection, bindings: &[(&'static str, String)]) -> Result<Self, String> {
        let setup = conn.setup();
        let (first, last) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(first, last - first + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per = usize::from(map.keysyms_per_keycode).max(1);
        let codes = |keysym: u32| -> Vec<u32> {
            map.keysyms
                .chunks(per)
                .zip(u32::from(first)..)
                .filter(|(syms, _)| syms.contains(&keysym))
                .map(|(_, code)| code)
                .collect()
        };
        let mods = conn
            .get_modifier_mapping()
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per_mod = (mods.keycodes.len() / 8).max(1);
        let modifier = |index: usize| -> Vec<u32> {
            mods.keycodes
                .chunks(per_mod)
                .nth(index)
                .into_iter()
                .flatten()
                .filter(|&&code| code != 0)
                .map(|&code| u32::from(code))
                .collect()
        };
        let mut combos = Vec::new();
        for (id, trigger) in bindings {
            let (modifiers, keysym) = parse(trigger)?;
            let key = codes(keysym);
            if key.is_empty() {
                return Err(format!("no key on this keyboard produces {trigger}"));
            }
            combos.push((
                *id,
                Combo {
                    key,
                    modifiers: modifiers.into_iter().map(modifier).collect(),
                },
            ));
        }
        Ok(Self { combos })
    }
}

/// An XDG shortcuts-format trigger ("CTRL+SHIFT+m", "Control_R") as core
/// modifier indices and a keysym.
fn parse(trigger: &str) -> Result<(Vec<usize>, u32), String> {
    let mut parts: Vec<&str> = trigger.split('+').map(str::trim).collect();
    let key = parts.pop().filter(|k| !k.is_empty());
    let modifiers = parts
        .into_iter()
        .map(|m| match m.to_ascii_uppercase().as_str() {
            "SHIFT" => Ok(0),
            "CTRL" => Ok(2),
            "ALT" => Ok(3),
            "NUM" => Ok(4),
            "LOGO" => Ok(6),
            _ => Err(format!("unknown modifier {m} in {trigger}")),
        })
        .collect::<Result<_, _>>()?;
    let key = key.ok_or_else(|| format!("no key in {trigger}"))?;
    let keysym = keysym(key).ok_or_else(|| format!("unsupported key {key} in {trigger}"))?;
    Ok((modifiers, keysym))
}

/// The keysyms a push-to-talk key is likely to be. U2's key picker can widen
/// this when it records keys.
const NAMED: &[(&str, u32, &str)] = &[
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

fn keysym(name: &str) -> Option<u32> {
    if let Some(&(_, sym, _)) = NAMED.iter().find(|(n, ..)| *n == name) {
        return Some(sym);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=35).contains(&n)
    {
        return Some(0xffbe + n - 1);
    }
    // Printable ASCII keysyms are their own code points.
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => Some(u32::from(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// How the call controls name a trigger: "Right Ctrl", "Ctrl+Shift+M".
fn label(trigger: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_parse_to_modifiers_and_keysyms() {
        assert_eq!(parse("Control_R"), Ok((vec![], 0xffe4)));
        assert_eq!(parse("CTRL+SHIFT+m"), Ok((vec![2, 0], u32::from('m'))));
        assert_eq!(parse("LOGO+M"), Ok((vec![6], u32::from('m'))));
        assert_eq!(parse("F13"), Ok((vec![], 0xffca)));
        assert!(parse("HYPER+a").is_err());
        assert!(parse("CTRL+").is_err());
        assert!(parse("Multi_key").is_err());
    }

    #[test]
    fn labels_read_like_the_call_controls() {
        assert_eq!(label("Control_R"), "Right Ctrl");
        assert_eq!(label("CTRL+SHIFT+m"), "Ctrl+Shift+M");
        assert_eq!(label("F9"), "F9");
    }

    #[test]
    fn combos_need_the_key_and_every_modifier() {
        let combo = Combo {
            key: vec![58],
            modifiers: vec![vec![37, 105], vec![50, 62]],
        };
        let held = |codes: &[u32]| combo.held(&codes.iter().copied().collect());
        assert!(held(&[105, 62, 58]));
        assert!(held(&[37, 50, 58]));
        assert!(!held(&[37, 58]));
        assert!(!held(&[37, 50]));
    }
}
