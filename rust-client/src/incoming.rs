//! Foreground encrypted DM invitations. An incoming notification never starts capture.
use crate::{
    App, Msg, Request,
    desktop_call::{active_member, check_dm, now},
    rooms::items,
};
use async_channel::{Receiver, Sender};
use iced::Task;
use serde_json::Value;
use std::{collections::BTreeMap, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, time};
use untethered_iced::rtc_membership::EVENT_TYPE;

const RING: &str = "org.matrix.msc4075.rtc.notification";
const DECLINE: &str = "org.matrix.msc4310.rtc.decline";
#[derive(Clone)]
pub struct Offer {
    pub video: bool,
    pub room: String,
    pub peer: String,
    pub event: String,
    member: String,
    // Bind prompt to the device and original join, including across lease renewal.
    caller_session: Option<(String, u64)>,
    pub expires: u64,
    room_mention: bool,
}
fn redacted(e: &Value) -> bool {
    e["unsigned"]["redacted_because"].is_object()
}
impl Offer {
    /// A ring in `room` from a Matrix event (decrypted).
    fn parse(e: &Value, room: &str, peer: &str, user: &str, at: u64) -> Option<Self> {
        let (ty, c) = (&e["type"], &e["content"]);
        if ty != RING
            || e.get("room_id").is_some_and(|r| r != room)
            || e["sender"] != peer
            || redacted(e)
            || c["notification_type"] != "ring"
            || !matches!(c["m.call.intent"].as_str(), Some("audio" | "video"))
            || c["slot_id"].as_str().is_some_and(|s| s != "m.call#ROOM")
            || c["m.relates_to"]["rel_type"] != "m.reference"
        {
            return None;
        }
        let direct = items(&c["m.mentions"]["user_ids"]).any(|u| u == user);
        if !direct && c["m.mentions"]["room"] != true {
            return None;
        }
        let server = e["origin_server_ts"].as_u64()?;
        let sent = c["sender_ts"].as_u64()?;
        let sent = if sent > server.saturating_add(20_000) {
            server
        } else {
            sent
        };
        let expires = sent.saturating_add(c["lifetime"].as_u64()?.min(90_000));
        if expires <= at || server > at.saturating_add(20_000) {
            return None;
        }
        let event_id = |v: &Value| {
            v.as_str()
                .filter(|id| id.starts_with('$') && id.len() > 1)
                .map(str::to_owned)
        };
        Some(Self {
            video: c["m.call.intent"] == "video",
            room: room.into(),
            peer: peer.into(),
            event: event_id(&e["event_id"])?,
            member: event_id(&c["m.relates_to"]["event_id"])?,
            caller_session: None,
            expires,
            room_mention: !direct,
        })
    }
    pub fn matches_member(&self, e: &Value) -> bool {
        e["sender"] == self.peer
            && active_member(e)
            && (e["event_id"] == self.member
                || self.caller_session.as_ref().is_some_and(|(key, ts)| {
                    e["state_key"] == *key && e["content"]["created_ts"] == *ts
                }))
    }
    pub fn valid_state(&self, state: &[Value], user: &str) -> bool {
        if self.expires <= now()
            || check_dm(state, user, &self.peer).is_err()
            || !state.iter().any(|e| self.matches_member(e))
            || state
                .iter()
                .any(|e| e["sender"] == user && active_member(e))
        {
            return false;
        }
        if !self.room_mention {
            return true;
        }
        // In room v12 and later creator power is implicit, not a `users` entry.
        if let Some(create) = state.iter().find(|e| e["type"] == "m.room.create")
            && create["content"]["room_version"]
                .as_str()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|v| v >= 12)
            && (create["sender"] == self.peer
                || items(&create["content"]["additional_creators"]).any(|u| *u == self.peer))
        {
            return true;
        }
        let power = state
            .iter()
            .find(|e| e["type"] == "m.room.power_levels")
            .map(|e| &e["content"])
            .unwrap_or(&Value::Null);
        power["users"][&self.peer]
            .as_i64()
            .or(power["users_default"].as_i64())
            .unwrap_or(0)
            >= power["notifications"]["room"].as_i64().unwrap_or(50)
    }
}
#[derive(Default)]
pub struct Inbox {
    pub ringing: Option<(Offer, Sender<()>)>,
    pub handled: Option<(String, &'static str)>,
    playback: Option<Receiver<()>>,
    mute: Option<Sender<()>>,
    seen: BTreeMap<(String, String), u64>,
    checking: Option<Offer>,
}
impl Inbox {
    pub(super) fn silence_chime(&mut self) {
        if let Some(mute) = self.mute.take() {
            mute.close();
        }
    }
}
impl App {
    pub(super) fn clear_ring(&mut self) {
        if let Some((_, stop)) = self.inbox.ringing.take() {
            stop.close();
        }
        self.inbox.playback = None;
        self.inbox.silence_chime();
        self.inbox.checking = None;
        self.inbox.handled = None;
    }
    /// Check a new ring against the room's current call state before
    /// presenting it.
    fn check_ring(&mut self, i: Offer) {
        let room = i.room.clone();
        self.inbox.checking = Some(i.clone());
        self.command(
            |id| crate::engine::Command::CallState { id, room },
            Request::RingState(i),
        );
        if !matches!(self.pending.get(&self.seq), Some(Request::RingState(_))) {
            self.inbox.checking = None;
        }
    }
    /// A call-related room event from the engine, as a Matrix event: a ring,
    /// a decline, a redaction, or a DM's call membership or member change.
    pub(super) fn call_activity(&mut self, room: &str, e: &Value) {
        let at = now();
        self.inbox.seen.retain(|_, t| *t > at);
        let Some(peer) = self
            .live
            .list
            .get(room)
            .filter(|r| r.encrypted)
            .and_then(|r| r.dm_peer.clone())
        else {
            return;
        };
        let pending = self
            .inbox
            .ringing
            .as_ref()
            .map(|(i, _)| i)
            .or(self.inbox.checking.as_ref())
            .filter(|i| i.room == room)
            .cloned();
        let ty = &e["type"];
        let c = &e["content"];
        if e["sender"] == self.matrix_id && !redacted(e) {
            let mut outcome = None;
            if ty == DECLINE
                && c["m.relates_to"]["rel_type"] == "m.reference"
                && let Some(id) = c["m.relates_to"]["event_id"]
                    .as_str()
                    .filter(|s| s.starts_with('$'))
                && let Some(ts) = e["origin_server_ts"].as_u64()
                && ts.saturating_add(90_000) > at
            {
                // A second device may decrypt the decline *before* the ring.
                self.inbox
                    .seen
                    .insert((room.into(), id.into()), at + 90_000);
                if pending.as_ref().is_some_and(|i| i.event == id) {
                    outcome = Some("Call declined on another device.");
                }
            } else if pending.is_some() && active_member(e) {
                outcome = Some("Call answered on another device.");
            }
            if let Some(outcome) = outcome {
                self.clear_ring();
                self.inbox.handled = Some((room.into(), outcome));
                return;
            }
        }
        if let Some(i) = &pending
            && ((ty == EVENT_TYPE
                && e["sender"] == peer
                && i.caller_session
                    .as_ref()
                    .map_or(!active_member(e), |(key, _)| e["state_key"] == *key)
                && !i.matches_member(e))
                || (ty == "m.room.member" && e["state_key"] == peer && c["membership"] != "join")
                || (ty == "m.room.redaction"
                    && [c.get("redacts"), e.get("redacts")]
                        .into_iter()
                        .flatten()
                        .any(|id| *id == i.event || *id == i.member)))
        {
            self.clear_ring();
        }
        if self.call.is_some()
            || !self.verified
            || self.closing
            || e["sender"] != peer
            || self.blocked.contains(&peer)
        {
            return;
        }
        let Some(id) = e["event_id"].as_str() else {
            return;
        };
        let key = (room.to_owned(), id.to_owned());
        if self.inbox.seen.contains_key(&key)
            || self.inbox.ringing.is_some()
            || self.inbox.checking.is_some()
        {
            return;
        }
        if let Some(i) = Offer::parse(e, room, &peer, &self.matrix_id, at) {
            self.inbox.seen.insert(key, i.expires);
            self.check_ring(i);
        }
    }
    pub(super) fn present_ring(&mut self, mut i: Offer, state: &Value) {
        if self.inbox.checking.as_ref().map(|p| &p.event) != Some(&i.event) {
            return;
        }
        self.inbox.checking = None;
        if !self.verified
            || self.closing
            || self.call.is_some()
            || !self
                .live
                .list
                .get(&i.room)
                .is_some_and(|r| r.encrypted && r.dm_peer.as_ref() == Some(&i.peer))
            || !state
                .as_array()
                .is_some_and(|s| i.valid_state(s, &self.matrix_id))
        {
            return;
        }
        if let Some(e) = items(state).find(|e| i.matches_member(e)) {
            i.caller_session = e["state_key"]
                .as_str()
                .zip(
                    e["content"]["created_ts"]
                        .as_u64()
                        .or(e["origin_server_ts"].as_u64()),
                )
                .map(|(k, t)| (k.to_owned(), t));
        }
        let (stop, recv) = async_channel::bounded::<()>(1);
        self.inbox.playback = Some(recv);
        self.inbox.handled = None;
        self.inbox.ringing = Some((i, stop));
    }
    pub(super) fn start_ringtone(&mut self) -> Task<Msg> {
        let quiet = self.quiet_alerts();
        let (Some((i, _)), Some(recv)) = (&self.inbox.ringing, self.inbox.playback.take()) else {
            return Task::none();
        };
        let id = i.event.clone();
        let duration = Duration::from_millis(i.expires.saturating_sub(now()));
        let (mute, muted) = async_channel::bounded(1);
        if quiet {
            mute.close();
        } else {
            self.inbox.mute = Some(mute);
        }
        Task::perform(ringtone(duration, recv, muted), move |()| {
            Msg::RingExpired(id.clone())
        })
    }
    pub(super) fn decide_ring(&mut self, id: &str, accept: bool) {
        if !self
            .inbox
            .ringing
            .as_ref()
            .is_some_and(|(i, _)| i.event == id)
        {
            return;
        }
        let (i, stop) = self.inbox.ringing.take().unwrap();
        stop.close();
        if i.expires <= now() || self.call.is_some() || !self.verified || self.closing {
            return;
        }
        if accept {
            self.start_voice(
                i.room.clone(),
                crate::desktop_call::Target::Dm {
                    video: i.video,
                    peer: i.peer.clone(),
                    incoming: Some(i),
                },
            );
        } else {
            let (room, ring) = (i.room.clone(), i.event.clone());
            self.command(
                |id| crate::engine::Command::DeclineRing { id, room, ring },
                Request::DeclineSent,
            );
        }
    }
}
async fn ringtone(duration: Duration, stop: Receiver<()>, muted: Receiver<()>) {
    let sound = async {
        let mut p = Command::new("pacat")
            .args([
                "--playback",
                "--raw",
                "--format=s16le",
                "--rate=16000",
                "--channels=1",
                "--latency-msec=50",
                "--client-name=Untethered",
                "--stream-name=Incoming call",
            ])
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut input = p.stdin.take()?;
        // Soft two-tone chime, followed by silence. There is no capture process here.
        let cycle: Vec<u8> = (0..32000)
            .flat_map(|n| {
                let i = n % 6400;
                let t = i as f32 / 16000.;
                let envelope = if n < 12800 && i < 4500 {
                    (i.min(4500 - i) as f32 / 250.).min(1.)
                } else {
                    0.
                };
                (((t * std::f32::consts::TAU * 620.)
                    .sin()
                    .mul_add(2400., (t * std::f32::consts::TAU * 440.).sin() * 1600.)
                    * envelope) as i16)
                    .to_le_bytes()
            })
            .collect();
        while input.write_all(&cycle).await.is_ok() {}
        Some(())
    };
    let timeout = time::sleep(duration);
    tokio::pin!(timeout);
    // Drop the sound future (and its pacat process) as soon as DND starts,
    // independently of the invite's accept/decline controls and expiry timer.
    let playback = async {
        tokio::select! {biased;_=muted.recv()=>(),_=sound=>()}
    };
    tokio::select! {biased;_=stop.recv()=>(),_= &mut timeout=>(),_=playback=>{
        tokio::select!{_=stop.recv()=>(),_=timeout=>()}
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ROOM: &str = "!dm:example";
    const ME: &str = "@me:example";
    const PEER: &str = "@caller:example";
    fn ring(t: u64) -> Value {
        json!({"room_id":ROOM,"event_id":"$ring","sender":PEER,"origin_server_ts":t,"type":RING,"content":{
            "m.call.intent":"audio","slot_id":"m.call#ROOM","notification_type":"ring",
            "m.relates_to":{"rel_type":"m.reference","event_id":"$join"},
            "m.mentions":{"room":true},"sender_ts":t,"lifetime":30_000}})
    }
    /// An App whose engine commands land in the returned channel.
    fn app(rooms: &[&str]) -> (App, async_channel::Receiver<crate::engine::Command>) {
        let (tx, _) = async_channel::bounded(8);
        let mut a = App::new(tx);
        let (sdk, commands) = async_channel::bounded(8);
        a.sdk = Some(sdk);
        a.connected = true;
        a.verified = true;
        a.matrix_id = ME.into();
        for room in rooms {
            a.live.list.insert(
                (*room).into(),
                crate::rooms::Room {
                    encrypted: true,
                    dm_peer: Some(PEER.into()),
                    ..Default::default()
                },
            );
        }
        (a, commands)
    }
    fn members(t: u64) -> Vec<Value> {
        vec![
            json!({"type":"m.room.encryption","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
            json!({"type":"m.room.member","state_key":ME,"content":{"membership":"join"}}),
            json!({"type":"m.room.member","state_key":PEER,"content":{"membership":"join"}}),
            json!({"type":EVENT_TYPE,"event_id":"$join","sender":PEER,"state_key":"caller_session","origin_server_ts":t,
                "content":{"application":"m.call","call_id":"","scope":"m.room","device_id":"D","created_ts":t,"expires":90000}}),
        ]
    }
    #[test]
    fn only_fresh_targeted_dm_rings_with_live_referenced_membership() {
        let t = now();
        let mut e = ring(t);
        assert!(Offer::parse(&e, ROOM, PEER, ME, t + 30_000).is_none());
        let mut s = members(t);
        let i = Offer::parse(&e, ROOM, PEER, ME, t).unwrap();
        assert!(!i.valid_state(&s, ME), "unauthorized room mention");
        s.push(json!({"type":"m.room.power_levels","content":{"notifications":{"room":0}}}));
        assert!(i.valid_state(&s, ME));
        e["content"]["m.mentions"] = json!({"user_ids":["@elsewhere:example"]});
        assert!(Offer::parse(&e, ROOM, PEER, ME, t).is_none());
        e["content"]["m.mentions"] = json!({"user_ids":[ME]});
        let direct = Offer::parse(&e, ROOM, PEER, ME, t).unwrap();
        s.pop();
        assert!(direct.valid_state(&s, ME));
        s[3]["event_id"] = json!("$new-session");
        assert!(!direct.valid_state(&s, ME));
    }
    #[test]
    fn sibling_decline_can_decrypt_before_the_invite_and_is_scoped_to_its_room_and_reference() {
        let (mut a, commands) = app(&[ROOM, "!other"]);
        let t = now();
        let mut invite = ring(t);
        invite["content"]["m.mentions"] = json!({"user_ids":[ME]});
        // Another of our devices declined, and its decline decrypted first.
        let decline = json!({"event_id":"$decline","room_id":ROOM,"sender":ME,"type":DECLINE,
            "origin_server_ts":t,"content":{"m.relates_to":{"rel_type":"m.reference","event_id":"$ring"}}});
        a.call_activity(ROOM, &decline);
        a.call_activity(ROOM, &invite);
        assert!(commands.is_empty() && a.inbox.checking.is_none() && a.inbox.ringing.is_none());

        let mut other = invite;
        other["room_id"] = "!other".into();
        a.call_activity("!other", &other);
        assert!(matches!(
            commands.try_recv(),
            Ok(crate::engine::Command::CallState { .. })
        ));
        let pending = a.inbox.checking.clone().unwrap();
        let mut wrong = decline;
        wrong["room_id"] = "!other".into();
        wrong["content"]["m.relates_to"]["rel_type"] = "m.annotation".into();
        a.call_activity("!other", &wrong);
        assert!(a.inbox.checking.is_some());
        wrong["content"]["m.relates_to"]["rel_type"] = "m.reference".into();
        a.call_activity("!other", &wrong);
        a.present_ring(pending, &json!(members(t)));
        assert!(
            a.inbox.checking.is_none() && a.inbox.ringing.is_none() && a.inbox.playback.is_none()
        );
    }
    #[test]
    fn caller_cancelling_stops_only_its_own_ring_and_replay_does_not_restart_it() {
        let (mut a, commands) = app(&[ROOM]);
        let t = now();
        let mut e = ring(t);
        e["content"]["m.mentions"] = json!({"user_ids":[ME]});
        a.call_activity(ROOM, &e);
        let invite = a.inbox.checking.clone().unwrap();
        assert!(matches!(
            commands.try_recv(),
            Ok(crate::engine::Command::CallState { .. })
        ));
        a.present_ring(invite.clone(), &json!(members(t)));
        let sound = a.inbox.playback.clone().unwrap();
        let leave = |key: &str| {
            json!({"type":EVENT_TYPE,"event_id":"$leave","state_key":key,"sender":PEER,
                "room_id":ROOM,"content":{}})
        };
        a.call_activity(ROOM, &leave("other_device"));
        assert!(a.inbox.ringing.is_some() && !sound.is_closed());
        a.call_activity(ROOM, &leave("caller_session"));
        assert!(a.inbox.ringing.is_none() && sound.is_closed());
        a.decide_ring(&invite.event, true);
        a.call_activity(ROOM, &e);
        assert!(a.inbox.ringing.is_none() && a.inbox.checking.is_none() && commands.is_empty());
    }
}
