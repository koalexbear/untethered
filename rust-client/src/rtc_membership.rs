//! Renewable MatrixRTC session membership for desktop calls and interoperability tests,
//! and the signaling a call needs from the Matrix engine. No keys or tokens belong in
//! room state.
//!
//! [`CallSignal`] is the typed boundary between the call layer and an engine (gomuks or
//! matrix-rust-sdk). Room state crosses it as Matrix client-server API state events
//! (`type`, `state_key`, `sender`, `event_id`, `origin_server_ts`, `content`), and call
//! contents stay MatrixRTC JSON. [`Signal`] is gomuks's JSON RPC; anything implementing
//! it can run a [`Membership`] through [`StateSignal`].
use serde_json::{Value, json};
use std::ops::AsyncFnMut;

pub const EVENT_TYPE: &str = "org.matrix.msc3401.call.member";

/// gomuks's JSON RPC, for callers that only hold a request function.
pub trait Signal {
    fn request(
        &mut self,
        cmd: &'static str,
        body: Value,
    ) -> impl Future<Output = Result<Value, String>>;
}
impl<F: AsyncFnMut(&'static str, Value) -> Result<Value, String>> Signal for F {
    async fn request(&mut self, cmd: &'static str, body: Value) -> Result<Value, String> {
        self(cmd, body).await
    }
}

/// What to do with a scheduled (MSC4140 delayed) event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delayed {
    Cancel,
    Restart,
    Send,
}
impl Delayed {
    fn wire(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Restart => "restart",
            Self::Send => "send",
        }
    }
}

/// Room state writes, which are all a [`Membership`] needs.
pub trait StateSignal {
    /// Send a state event now; returns its event ID.
    fn set_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
    ) -> impl Future<Output = Result<String, String>>;
    /// Schedule a state event for `delay_ms` from now; returns the delay ID.
    fn set_delayed_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
        delay_ms: u32,
    ) -> impl Future<Output = Result<String, String>>;
    fn update_delayed(
        &mut self,
        delay_id: &str,
        action: Delayed,
    ) -> impl Future<Output = Result<(), String>>;
}
impl<T: Signal> StateSignal for T {
    async fn set_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
    ) -> Result<String, String> {
        let body = json!({"room_id":room,"type":kind,"state_key":key,"content":content});
        self.request("set_state", body)
            .await?
            .as_str()
            .filter(|id| id.starts_with('$'))
            .map(str::to_owned)
            .ok_or_else(|| "Missing state event ID".into())
    }
    async fn set_delayed_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
        delay_ms: u32,
    ) -> Result<String, String> {
        let body = json!({"room_id":room,"type":kind,"state_key":key,"content":content,
            "delay_ms":delay_ms});
        // gomuks answers a delayed send with the delay ID, not an event ID.
        self.request("set_state", body)
            .await?
            .as_str()
            .filter(|id| !id.is_empty() && !id.starts_with('$'))
            .map(str::to_owned)
            .ok_or_else(|| "Missing delay ID".into())
    }
    async fn update_delayed(&mut self, delay_id: &str, action: Delayed) -> Result<(), String> {
        self.request(
            "update_delayed_event",
            json!({"delay_id":delay_id,"action":action.wire()}),
        )
        .await
        .map(drop)
    }
}

/// The account and device a call runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub user: String,
    pub device: String,
    pub homeserver: String,
    /// The device is cross-signed; unverified devices don't call.
    pub verified: bool,
}

/// An Olm-encrypted `io.element.call.encryption_keys` to-device message.
#[derive(Clone, Debug, PartialEq)]
pub struct CallKey {
    /// The Olm-authenticated sender.
    pub sender: String,
    pub content: Value,
}

/// Something a running call watches for.
#[derive(Clone, Debug, PartialEq)]
pub enum CallUpdate {
    /// Call memberships or room membership changed, or we left the room.
    Roster,
    Key(CallKey),
    /// `sender` declined the ring `refers_to`, in an encrypted, server-confirmed
    /// reference.
    Decline {
        sender: String,
        refers_to: String,
    },
}

/// The signaling a call needs from the Matrix engine. Errors are engine-written
/// sentences for the call status, never server text.
pub trait CallSignal: StateSignal {
    fn identity(&mut self) -> impl Future<Output = Result<Identity, &'static str>>;
    /// The room's state events. `refetch` asks for the homeserver's current state
    /// rather than a cache.
    fn room_state(
        &mut self,
        room: &str,
        refetch: bool,
    ) -> impl Future<Output = Result<Vec<Value>, &'static str>>;
    /// Send a (possibly encrypted) room event; returns its event ID.
    fn send_event(
        &mut self,
        room: &str,
        kind: &str,
        content: Value,
    ) -> impl Future<Output = Result<String, &'static str>>;
    /// Send `content` Olm-encrypted to each (user, device).
    fn send_to_device(
        &mut self,
        kind: &str,
        content: Value,
        targets: &[(String, String)],
    ) -> impl Future<Output = Result<(), &'static str>>;
    /// An OpenID token, as the homeserver returns it, for the LiveKit JWT service.
    fn openid_token(&mut self) -> impl Future<Output = Result<Value, &'static str>>;
    /// The MSC4143 RTC transports the homeserver advertises (possibly none).
    fn rtc_transports(&mut self) -> impl Future<Output = Result<Vec<Value>, &'static str>>;
    /// Wait for what happened next in `room`. May return nothing, so callers
    /// recheck their timers.
    fn next(&mut self, room: &str) -> impl Future<Output = Result<Vec<CallUpdate>, &'static str>>;
    /// What has already arrived, without waiting (for example while another
    /// request was being answered).
    fn ready(&mut self) -> Vec<CallUpdate>;
    /// Before joining: deliver call keys from now on.
    fn start(&mut self) -> impl Future<Output = Result<(), &'static str>> {
        async { Ok(()) }
    }
    /// After leaving.
    fn stop(&mut self) -> impl Future<Output = ()> {
        async {}
    }
}

pub struct Membership {
    room: String,
    state_key: String,
    delay_id: String,
    pub event_id: String,
    join_content: Value,
    delay_ms: u32,
    started: std::time::Instant,
}

impl Membership {
    /// Advertise the room-wide voice slot for normal (numbered) room versions.
    /// The absolute expiry bounds stale membership even if the client dies between
    /// publishing the join and arming the homeserver's much shorter delayed leave.
    pub async fn join(
        room: &str,
        user: &str,
        device: &str,
        livekit_service: &url::Url,
        delay_ms: u32,
        rpc: &mut impl StateSignal,
    ) -> Result<Self, String> {
        Self::join_with_video(room, user, device, livekit_service, delay_ms, false, rpc).await
    }
    pub async fn join_with_video(
        room: &str,
        user: &str,
        device: &str,
        livekit_service: &url::Url,
        delay_ms: u32,
        video: bool,
        rpc: &mut impl StateSignal,
    ) -> Result<Self, String> {
        if device.is_empty() || !(1..60_000).contains(&delay_ms) {
            return Err("Invalid device or delayed-leave interval".into());
        }
        let join = json!({
            "application":"m.call", "scope":"m.room", "call_id":"", "m.call.intent":if video{"video"}else{"audio"},
            "device_id":device, "membershipID":format!("{user}:{device}"),
            "created_ts":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            "focus_active":{"type":"livekit","focus_selection":"multi_sfu"},
            "foci_preferred":[{"type":"livekit","livekit_service_url":livekit_service.as_str().trim_end_matches('/')}]
        });
        let mut this = Self {
            room: room.into(),
            state_key: format!("_{user}_{device}_m.call"),
            delay_id: String::new(),
            event_id: String::new(),
            join_content: join,
            delay_ms,
            started: std::time::Instant::now(),
        };
        this.renew(rpc).await?;
        Ok(this)
    }

    /// Retire the previous scheduled leave before replacing the bounded lease:
    /// a homeserver can otherwise deliver the old leave over the new membership.
    pub async fn renew(&mut self, rpc: &mut impl StateSignal) -> Result<(), String> {
        if !self.delay_id.is_empty() {
            rpc.update_delayed(&self.delay_id, Delayed::Cancel).await?;
            self.delay_id.clear();
        }
        // Keep the same member identity and creation time, but advance expiry.
        // Identical content can be deduplicated to the original state event.
        self.join_content["expires"] = (60_000 + self.started.elapsed().as_millis() as u64).into();
        self.event_id = rpc
            .set_state(
                &self.room,
                EVENT_TYPE,
                &self.state_key,
                self.join_content.clone(),
            )
            .await?;
        match rpc
            .set_delayed_state(
                &self.room,
                EVENT_TYPE,
                &self.state_key,
                json!({}),
                self.delay_ms,
            )
            .await
        {
            Ok(id) => self.delay_id = id,
            Err(_) => {
                rpc.set_state(&self.room, EVENT_TYPE, &self.state_key, json!({}))
                    .await?;
                return Err("Could not arm delayed call leave".into());
            }
        }
        Ok(())
    }

    pub async fn heartbeat(&self, rpc: &mut impl StateSignal) -> Result<(), String> {
        rpc.update_delayed(&self.delay_id, Delayed::Restart).await
    }

    pub async fn leave(self, rpc: &mut impl StateSignal) -> Result<(), String> {
        if rpc
            .update_delayed(&self.delay_id, Delayed::Send)
            .await
            .is_err()
        {
            rpc.set_state(&self.room, EVENT_TYPE, &self.state_key, json!({}))
                .await?;
        }
        Ok(())
    }
}
