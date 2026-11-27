//! Sounds (ROADMAP N9), as on Discord: a chime for messages and a brighter
//! one for mentions, and short tones when you or others join or leave your
//! call and when you mute, unmute, deafen or undeafen. Each has a switch in
//! User Settings, kept on this computer (`sounds.json` in the profile
//! directory), like the tray's settings.
//!
//! The tones are generated here, so there are no sound files to license.
//! They play through `pacat` like the ringtone (incoming.rs), on the call's
//! speaker while in a call.
//!
//! Message and mention sounds follow the alerts' rules (notifications.rs):
//! never for your own messages, the room you're reading with the window
//! focused, or while Do Not Disturb is on. A mention always has its sound;
//! another message has one when the account's push rules give it a sound
//! (direct messages by default), which is what Matrix clients ring for.
//! Call sounds are feedback for the call you're in and play under Do Not
//! Disturb too, as on Discord; deafened, you don't hear people come and go.
use crate::{App, Msg};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    Message,
    Mention,
    Join,
    Leave,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
}

/// The switches in User Settings. Mute and Deafen each cover both ways.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Switch {
    Messages,
    Mentions,
    Join,
    Leave,
    Mute,
    Deafen,
}
impl Switch {
    pub const ALL: [Self; 6] = [
        Self::Messages,
        Self::Mentions,
        Self::Join,
        Self::Leave,
        Self::Mute,
        Self::Deafen,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Messages => "Message",
            Self::Mentions => "Mention",
            Self::Join => "Voice join",
            Self::Leave => "Voice leave",
            Self::Mute => "Mute and unmute",
            Self::Deafen => "Deafen and undeafen",
        }
    }
    pub fn about(self) -> &'static str {
        match self {
            Self::Messages => {
                "A message your notification settings ring for (direct messages, by default), unless you’re reading that room."
            }
            Self::Mentions => "Someone mentions you, unless you’re reading that room.",
            Self::Join => "You join a call, or someone joins yours.",
            Self::Leave => "You leave a call, or someone leaves yours.",
            Self::Mute => "You mute or unmute your microphone in a call.",
            Self::Deafen => "You deafen or undeafen yourself in a call.",
        }
    }
}

impl Sound {
    pub fn switch(self) -> Switch {
        match self {
            Self::Message => Switch::Messages,
            Self::Mention => Switch::Mentions,
            Self::Join => Switch::Join,
            Self::Leave => Switch::Leave,
            Self::Mute | Self::Unmute => Switch::Mute,
            Self::Deafen | Self::Undeafen => Switch::Deafen,
        }
    }
    /// For the audio stream's name and the QA log.
    pub fn name(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Mention => "mention",
            Self::Join => "join",
            Self::Leave => "leave",
            Self::Mute => "mute",
            Self::Unmute => "unmute",
            Self::Deafen => "deafen",
            Self::Undeafen => "undeafen",
        }
    }
    /// The notes, as (Hz, start ms, length ms). Each sound has notes of its
    /// own (or the same ones in the other order), away from the ringtone's
    /// 440/620 Hz, so the live fixture can tell them apart.
    fn notes(self) -> &'static [(f32, u32, u32)] {
        match self {
            Self::Message => &[(1175., 0, 90), (1568., 70, 170)],
            Self::Mention => &[(1397., 0, 80), (1865., 70, 80), (2349., 140, 210)],
            Self::Join => &[(587., 0, 110), (784., 100, 230)],
            Self::Leave => &[(698., 0, 110), (523., 100, 250)],
            Self::Mute => &[(880., 0, 70), (659., 60, 130)],
            Self::Unmute => &[(659., 0, 70), (880., 60, 130)],
            Self::Deafen => &[(554., 0, 90), (415., 80, 190)],
            Self::Undeafen => &[(415., 0, 90), (554., 80, 190)],
        }
    }
    /// Mono 16-bit samples at [`RATE`]: soft sine notes that fade out.
    pub fn pcm(self) -> Vec<i16> {
        let notes = self.notes();
        let ms = |ms: u32| (ms as usize) * RATE as usize / 1000;
        let len = notes.iter().map(|(_, at, n)| ms(at + n)).max().unwrap_or(0);
        let mut mix = vec![0f32; len];
        for &(hz, at, length) in notes {
            let (start, n) = (ms(at), ms(length));
            let attack = ms(4).max(1) as f32;
            for i in 0..n {
                let t = i as f32 / RATE as f32;
                // A quick rise, then a fall to silence by the note's end.
                let rise = (i as f32 / attack).min(1.);
                let fall = (1. - i as f32 / n as f32).powi(2);
                mix[start + i] += (t * hz * std::f32::consts::TAU).sin() * rise * fall;
            }
        }
        // Overlapping notes sum; keep the peak where one note's is.
        let peak = mix.iter().fold(1f32, |m, v| m.max(v.abs()));
        mix.into_iter().map(|v| (v / peak * LEVEL) as i16).collect()
    }
}

pub const RATE: u32 = 48_000;
/// About a quarter of full scale: noticeable, not startling.
const LEVEL: f32 = 8_000.;
/// Message and mention sounds wait this long after the last one, so a
/// burst of messages chimes once.
const MESSAGE_GAP: Duration = Duration::from_millis(1_500);

/// Which sounds play, on this computer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub messages: bool,
    pub mentions: bool,
    pub join: bool,
    pub leave: bool,
    pub mute: bool,
    pub deafen: bool,
}
impl Default for Settings {
    fn default() -> Self {
        // All on, as on Discord.
        Self {
            messages: true,
            mentions: true,
            join: true,
            leave: true,
            mute: true,
            deafen: true,
        }
    }
}
impl Settings {
    pub fn on(&self, switch: Switch) -> bool {
        match switch {
            Switch::Messages => self.messages,
            Switch::Mentions => self.mentions,
            Switch::Join => self.join,
            Switch::Leave => self.leave,
            Switch::Mute => self.mute,
            Switch::Deafen => self.deafen,
        }
    }
    fn set(&mut self, switch: Switch, on: bool) {
        *match switch {
            Switch::Messages => &mut self.messages,
            Switch::Mentions => &mut self.mentions,
            Switch::Join => &mut self.join,
            Switch::Leave => &mut self.leave,
            Switch::Mute => &mut self.mute,
            Switch::Deafen => &mut self.deafen,
        } = on;
    }
    fn path() -> Option<PathBuf> {
        Some(crate::engine::profile_root().ok()?.join("sounds.json"))
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn save(self) -> Result<(), &'static str> {
        const FAILED: &str = "Couldn’t save the setting in the profile directory.";
        let path = Self::path().ok_or(FAILED)?;
        let dir = path.parent().ok_or(FAILED)?;
        std::fs::create_dir_all(dir).map_err(|_| FAILED)?;
        // Replace the file whole, so a crash never leaves half a file.
        let next = path.with_extension("json.new");
        std::fs::write(&next, serde_json::to_vec_pretty(&self).map_err(|_| FAILED)?)
            .map_err(|_| FAILED)?;
        std::fs::rename(&next, &path).map_err(|_| FAILED)
    }
}

/// The sound for a notification the alert rules let through: `highlight`
/// and `sound` are the push rules' tweaks. A mention with its sound off
/// still chimes as the message it is.
pub fn for_message(highlight: bool, sound: bool, settings: Settings) -> Option<Sound> {
    if highlight && settings.mentions {
        Some(Sound::Mention)
    } else if (highlight || sound) && settings.messages {
        Some(Sound::Message)
    } else {
        None
    }
}

/// The call as the sounds last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub id: u128,
    /// People connected, you included, once connected; kept through a
    /// reconnect.
    pub people: Option<usize>,
    pub muted: bool,
    pub deafened: bool,
    /// The call's speaker, which the sounds use too.
    pub device: Option<String>,
}

/// How many people a call's status says are connected, you included.
pub fn people(label: &str) -> Option<usize> {
    if label == "Connected" {
        return Some(2);
    }
    label
        .strip_prefix("In voice · ")?
        .strip_suffix(" connected")?
        .parse()
        .ok()
}

/// What changed between two looks at the call (`None`: no call, or it's
/// ending), and the sounds that go with it.
pub fn call_step(before: Option<Seen>, now: Option<Seen>) -> (Option<Seen>, Vec<Sound>) {
    let mut sounds = Vec::new();
    let Some(mut now) = now else {
        if before.is_some_and(|b| b.people.is_some()) {
            sounds.push(Sound::Leave);
        }
        return (None, sounds);
    };
    match before {
        Some(b) if b.id != now.id => {
            if b.people.is_some() {
                sounds.push(Sound::Leave);
            }
            if now.people.is_some() {
                sounds.push(Sound::Join);
            }
        }
        None => {
            if now.people.is_some() {
                sounds.push(Sound::Join);
            }
        }
        Some(b) => {
            match (b.people, now.people) {
                (None, Some(_)) => sounds.push(Sound::Join),
                // Someone else came or went; deafened, you don't hear it.
                (Some(was), Some(is)) if is != was && !now.deafened => {
                    sounds.push(if is > was { Sound::Join } else { Sound::Leave })
                }
                // Reconnecting: still in the call.
                (Some(_), None) => now.people = b.people,
                _ => {}
            }
            if b.deafened != now.deafened {
                sounds.push(if now.deafened {
                    Sound::Deafen
                } else {
                    Sound::Undeafen
                });
            } else if b.muted != now.muted {
                sounds.push(if now.muted {
                    Sound::Mute
                } else {
                    Sound::Unmute
                });
            }
        }
    }
    (Some(now), sounds)
}

#[derive(Default)]
pub struct Sounds {
    pub settings: Settings,
    pub saving: bool,
    pub note: Option<&'static str>,
    queued: Vec<Sound>,
    call: Option<Seen>,
    last_message: Option<Instant>,
}

impl Sounds {
    #[cfg(test)]
    pub fn queued(&self) -> &[Sound] {
        &self.queued
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Set(Switch, bool),
    Saved(Settings, Result<(), &'static str>),
}

impl App {
    /// A notification the alert rules let through (notifications.rs).
    pub(crate) fn notice_sound(&mut self, highlight: bool, sound: bool) {
        if let Some(sound) = for_message(highlight, sound, self.sounds.settings) {
            self.sounds.queued.push(sound);
        }
    }
    /// After every update: the call's changes, and what notifications queued.
    pub(crate) fn play_sounds(&mut self) -> Task<Msg> {
        let now = self
            .call
            .as_ref()
            .filter(|c| !c.cancel.is_closed())
            .map(|c| {
                let mode = c.audio.borrow();
                Seen {
                    id: c.id.as_u128(),
                    people: people(&c.label),
                    muted: mode.muted,
                    deafened: mode.deafened,
                    device: mode.speaker.name.clone(),
                }
            });
        let device = now
            .as_ref()
            .or(self.sounds.call.as_ref())
            .and_then(|c| c.device.clone());
        let (seen, mut sounds) = call_step(self.sounds.call.take(), now);
        self.sounds.call = seen;
        let queued = std::mem::take(&mut self.sounds.queued);
        // One chime for a burst: a mention's if there is one.
        if let Some(chime) = queued
            .iter()
            .copied()
            .find(|s| *s == Sound::Mention)
            .or(queued.first().copied())
            && self
                .sounds
                .last_message
                .is_none_or(|at| at.elapsed() >= MESSAGE_GAP)
        {
            self.sounds.last_message = Some(Instant::now());
            sounds.push(chime);
        }
        sounds.retain(|s| self.sounds.settings.on(s.switch()));
        if self.closing || sounds.is_empty() {
            return Task::none();
        }
        Task::batch(
            sounds
                .into_iter()
                .map(|sound| Task::future(play(sound, device.clone())).discard()),
        )
    }
    pub(crate) fn sound_event(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Set(switch, on) => {
                if self.sounds.saving || self.sounds.settings.on(switch) == on {
                    return Task::none();
                }
                self.sounds.saving = true;
                let mut next = self.sounds.settings;
                next.set(switch, on);
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || next.save())
                            .await
                            .unwrap_or(Err("Couldn’t save the setting."))
                    },
                    move |result| Msg::Sound(Event::Saved(next, result)),
                );
            }
            Event::Saved(next, result) => {
                self.sounds.saving = false;
                self.sounds.note = result.err();
                if result.is_ok() {
                    self.sounds.settings = next;
                }
            }
        }
        Task::none()
    }
}

/// Play one sound to the end, on `device` (or the default speaker).
async fn play(sound: Sound, device: Option<String>) {
    #[cfg(debug_assertions)]
    crate::engine::qa::sound(sound.name());
    let bytes: Vec<u8> = sound.pcm().iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut p = Command::new("pacat");
    p.args([
        "--playback",
        "--raw",
        "--format=s16le",
        "--rate=48000",
        "--channels=1",
        "--latency-msec=40",
        "--client-name=Untethered",
        "--property=media.role=event",
    ])
    .arg(format!("--stream-name=Untethered {} sound", sound.name()))
    .kill_on_drop(true)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    if let Some(device) = device {
        p.arg("--device").arg(device);
    }
    let Ok(mut child) = p.spawn() else {
        return;
    };
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(&bytes).await;
        // Closing the pipe lets pacat drain and exit.
        drop(input);
    }
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(people: Option<usize>, muted: bool, deafened: bool) -> Seen {
        Seen {
            id: 7,
            people,
            muted,
            deafened,
            device: None,
        }
    }

    #[test]
    fn mentions_chime_brighter_and_other_messages_follow_the_push_rules_sound() {
        let all = Settings::default();
        assert_eq!(for_message(true, true, all), Some(Sound::Mention));
        // A room mention notifies without a sound tweak; it still chimes.
        assert_eq!(for_message(true, false, all), Some(Sound::Mention));
        // A direct message (the default rules give it a sound).
        assert_eq!(for_message(false, true, all), Some(Sound::Message));
        // An ordinary group message notifies silently by default.
        assert_eq!(for_message(false, false, all), None);
        let no_mentions = Settings {
            mentions: false,
            ..all
        };
        assert_eq!(for_message(true, false, no_mentions), Some(Sound::Message));
        let silent = Settings {
            mentions: false,
            messages: false,
            ..all
        };
        assert_eq!(for_message(true, true, silent), None);
        assert_eq!(for_message(false, true, silent), None);
    }

    #[test]
    fn joining_leaving_and_others_coming_and_going_have_their_sounds() {
        // Ringing or connecting: nothing yet.
        let (s, sounds) = call_step(None, Some(seen(None, false, false)));
        assert!(sounds.is_empty());
        // Connected alone in a voice room.
        let (s, sounds) = call_step(s, Some(seen(Some(1), false, false)));
        assert_eq!(sounds, [Sound::Join]);
        let (s, sounds) = call_step(s, Some(seen(Some(2), false, false)));
        assert_eq!(sounds, [Sound::Join], "someone joined");
        // A reconnect keeps the count, and coming back is silent.
        let (s, sounds) = call_step(s, Some(seen(None, false, false)));
        assert!(sounds.is_empty());
        assert_eq!(s.as_ref().unwrap().people, Some(2));
        let (s, sounds) = call_step(s, Some(seen(Some(2), false, false)));
        assert!(sounds.is_empty());
        let (s, sounds) = call_step(s, Some(seen(Some(1), false, false)));
        assert_eq!(sounds, [Sound::Leave], "someone left");
        let (s, sounds) = call_step(s, None);
        assert_eq!(sounds, [Sound::Leave], "we left");
        assert!(s.is_none());
        // A call that never connected leaves silently.
        let (s, _) = call_step(None, Some(seen(None, false, false)));
        assert!(call_step(s, None).1.is_empty());
        // Straight into another call: leave this one, join that one.
        let (s, _) = call_step(None, Some(seen(Some(2), false, false)));
        let mut other = seen(Some(3), false, false);
        other.id = 8;
        assert_eq!(call_step(s, Some(other)).1, [Sound::Leave, Sound::Join]);
    }

    #[test]
    fn mute_and_deafen_sound_both_ways_and_deafen_hides_comings_and_goings() {
        let (s, _) = call_step(None, Some(seen(Some(2), false, false)));
        let (s, sounds) = call_step(s, Some(seen(Some(2), true, false)));
        assert_eq!(sounds, [Sound::Mute]);
        let (s, sounds) = call_step(s, Some(seen(Some(2), false, false)));
        assert_eq!(sounds, [Sound::Unmute]);
        let (s, sounds) = call_step(s, Some(seen(Some(2), false, true)));
        assert_eq!(sounds, [Sound::Deafen]);
        let (s, sounds) = call_step(s, Some(seen(Some(3), false, true)));
        assert!(sounds.is_empty(), "deafened: no join sound");
        assert_eq!(s.as_ref().unwrap().people, Some(3));
        let (_, sounds) = call_step(s, Some(seen(Some(3), false, false)));
        assert_eq!(sounds, [Sound::Undeafen]);
        // Joining muted plays only the join sound.
        let (_, sounds) = call_step(None, Some(seen(Some(1), true, false)));
        assert_eq!(sounds, [Sound::Join]);
    }

    #[test]
    fn call_status_labels_count_people() {
        assert_eq!(people("Connected"), Some(2));
        assert_eq!(people("In voice · 1 connected"), Some(1));
        assert_eq!(people("In voice · 12 connected"), Some(12));
        assert_eq!(people("Connecting"), None);
        assert_eq!(people("Reconnecting…"), None);
        assert_eq!(people("Ringing"), None);
    }

    /// An App signed in with a room, `!r`, open.
    fn app() -> App {
        let (tx, _) = async_channel::bounded(1);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.matrix_id = "@me:h".into();
        for room in ["!r", "!other"] {
            app.live.list.insert(room.into(), Default::default());
        }
        app.selected = "!r".into();
        app.sounds.settings = Settings::default();
        app
    }
    fn notice(app: &mut App, event: &str, room: &str, sender: &str, highlight: bool) {
        app.engine_notice(crate::engine::Alert {
            room: room.into(),
            event_id: event.into(),
            sender: sender.into(),
            body: "hi".into(),
            sound: !highlight,
            highlight,
        });
    }

    #[test]
    fn messages_chime_except_ours_the_room_being_read_and_under_dnd() {
        let mut app = app();
        notice(&mut app, "$own", "!other", "@me:h", false);
        assert!(app.sounds.queued().is_empty(), "never our own messages");
        app.focused = true;
        notice(&mut app, "$read", "!r", "@bob:h", true);
        assert!(app.sounds.queued().is_empty(), "the room we're reading");
        notice(&mut app, "$elsewhere", "!other", "@bob:h", false);
        assert_eq!(app.sounds.queued(), [Sound::Message], "another room");
        app.focused = false;
        notice(&mut app, "$away", "!r", "@bob:h", true);
        assert_eq!(
            app.sounds.queued(),
            [Sound::Message, Sound::Mention],
            "window not focused"
        );
        // A burst chimes once, with the mention's sound, then waits.
        let _ = app.play_sounds();
        assert!(app.sounds.queued().is_empty() && app.sounds.last_message.is_some());
        app.alerts.dnd = Some(true);
        notice(&mut app, "$dnd", "!other", "@bob:h", true);
        assert!(app.sounds.queued().is_empty(), "Do Not Disturb");
        app.alerts.dnd = Some(false);
        app.sounds.settings.messages = false;
        notice(&mut app, "$off", "!other", "@bob:h", false);
        assert!(app.sounds.queued().is_empty(), "switched off");
        app.sounds.settings.mentions = false;
        notice(&mut app, "$off2", "!other", "@bob:h", true);
        assert!(app.sounds.queued().is_empty(), "both switched off");
    }

    #[test]
    fn tones_are_short_quiet_enough_and_end_in_silence() {
        for switch in Switch::ALL {
            assert!(!switch.label().is_empty() && !switch.about().is_empty());
        }
        let all = [
            Sound::Message,
            Sound::Mention,
            Sound::Join,
            Sound::Leave,
            Sound::Mute,
            Sound::Unmute,
            Sound::Deafen,
            Sound::Undeafen,
        ];
        for sound in all {
            let pcm = sound.pcm();
            let ms = pcm.len() * 1000 / RATE as usize;
            assert!((150..=400).contains(&ms), "{sound:?} is {ms} ms");
            let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!((7_000..=8_000).contains(&peak), "{sound:?} peaks at {peak}");
            assert!(pcm[0].unsigned_abs() < 100, "{sound:?} starts with a click");
            assert!(
                pcm[pcm.len() - 1].unsigned_abs() < 100,
                "{sound:?} ends with a click"
            );
        }
        // Each sound's notes are its own, or another's reversed.
        assert_ne!(Sound::Mute.notes(), Sound::Unmute.notes());
        assert_eq!(Sound::Mute.notes()[0].0, Sound::Unmute.notes()[1].0);
    }
}
