//! Renewable MatrixRTC session membership for desktop calls and interoperability tests.
//! The caller supplies gomuks's RPC adapter. No keys or tokens belong in room state.
use serde_json::{Value, json};
use std::ops::AsyncFnMut;

pub const EVENT_TYPE: &str = "org.matrix.msc3401.call.member";
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
        rpc: impl Signal,
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
        rpc: impl Signal,
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
    pub async fn renew(&mut self, mut rpc: impl Signal) -> Result<(), String> {
        if !self.delay_id.is_empty() {
            rpc.request(
                "update_delayed_event",
                json!({"delay_id":self.delay_id,"action":"cancel"}),
            )
            .await?;
            self.delay_id.clear();
        }
        // Keep the same member identity and creation time, but advance expiry.
        // Identical content can be deduplicated to the original state event.
        self.join_content["expires"] = (60_000 + self.started.elapsed().as_millis() as u64).into();
        self.event_id = rpc
            .request("set_state", self.state(self.join_content.clone()))
            .await?
            .as_str()
            .filter(|id| id.starts_with('$'))
            .ok_or("Missing call membership event ID")?
            .into();
        let mut delayed = self.state(json!({}));
        delayed["delay_ms"] = self.delay_ms.into();
        let result = rpc.request("set_state", delayed).await;
        match result.as_ref().ok().and_then(Value::as_str) {
            Some(id) if !id.is_empty() && !id.starts_with('$') => self.delay_id = id.into(),
            _ => {
                rpc.request("set_state", self.state(json!({}))).await?;
                return Err("Could not arm delayed call leave".into());
            }
        }
        Ok(())
    }

    fn state(&self, content: Value) -> Value {
        json!({"room_id":self.room,"type":EVENT_TYPE,"state_key":self.state_key,"content":content})
    }

    pub async fn heartbeat(&self, mut rpc: impl Signal) -> Result<(), String> {
        rpc.request(
            "update_delayed_event",
            json!({"delay_id": self.delay_id, "action": "restart"}),
        )
        .await?;
        Ok(())
    }

    pub async fn leave(self, mut rpc: impl Signal) -> Result<(), String> {
        if rpc
            .request(
                "update_delayed_event",
                json!({"delay_id":self.delay_id,"action":"send"}),
            )
            .await
            .is_err()
        {
            rpc.request("set_state", self.state(json!({}))).await?;
        }
        Ok(())
    }
}
