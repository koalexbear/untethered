//! The call signaling on the gomuks engine: its own WebSocket to gomuks for the
//! lifetime of a call. gomuks's JSON (its RPCs, sync frames and event envelope) stops
//! here; the call layer sees [`CallSignal`]'s typed values and Matrix state events.
use super::{KEY, Result, SECOND};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tokio::{net::TcpStream, time};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use untethered_iced::rtc_membership::{CallKey, CallSignal, CallUpdate, EVENT_TYPE, Identity};

const DECLINE: &str = "org.matrix.msc4310.rtc.decline";

pub struct GomuksSignal {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    seq: u64,
    /// What `listen_to_device` was before the call turned it on.
    prior_listen: Option<Value>,
    /// Updates read while waiting for an RPC answer.
    queued: Vec<CallUpdate>,
    room: String,
}

impl GomuksSignal {
    pub async fn connect(addr: SocketAddr, cookie: &str, room: &str) -> Result<Self> {
        let mut req = format!("ws://{addr}/_gomuks/websocket")
            .into_client_request()
            .map_err(|_| "Cannot open call signaling.")?;
        req.headers_mut().insert(
            "Cookie",
            cookie.parse().map_err(|_| "Missing Matrix session.")?,
        );
        let ws = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|_| "Could not connect to gomuks.")?
            .0;
        Ok(Self {
            ws,
            seq: 0,
            prior_listen: None,
            queued: Vec::new(),
            room: room.into(),
        })
    }

    async fn frame(&mut self) -> Result<Value> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(raw))) => {
                    let f: Value =
                        serde_json::from_str(&raw).map_err(|_| "Invalid call signaling.")?;
                    if f["command"] == "client_state" && f["data"]["is_verified"] != true {
                        return Err("The verified call session ended.");
                    }
                    let room = self.room.clone();
                    self.queued.extend(updates(&f, &room));
                    return Ok(f);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return Err("Lost the Matrix call connection."),
            }
        }
    }

    async fn rpc(&mut self, name: &str, body: Value) -> Result<Value> {
        time::timeout(SECOND * 10, async {
            self.seq += 1;
            self.ws
                .send(Message::Text(
                    json!({"command":name,"request_id":self.seq,"data":body})
                        .to_string()
                        .into(),
                ))
                .await
                .map_err(|_| "Matrix call signaling failed.")?;
            loop {
                let f = self.frame().await?;
                if f["request_id"] == self.seq {
                    match f["command"].as_str() {
                        Some("response") => return Ok(f["data"].clone()),
                        Some("error") => return Err("Homeserver rejected the call request."),
                        _ => (),
                    }
                }
            }
        })
        .await
        .map_err(|_| "Matrix call request timed out.")?
    }
}

/// gomuks's RPC, so [`StateSignal`](untethered_iced::rtc_membership::StateSignal)
/// comes from the shared implementation.
impl untethered_iced::rtc_membership::Signal for GomuksSignal {
    async fn request(
        &mut self,
        cmd: &'static str,
        body: Value,
    ) -> std::result::Result<Value, String> {
        self.rpc(cmd, body).await.map_err(str::to_owned)
    }
}

impl CallSignal for GomuksSignal {
    async fn identity(&mut self) -> Result<Identity> {
        let state = self.rpc("get_state", json!({})).await?;
        Ok(Identity {
            user: state["user_id"].as_str().ok_or("No Matrix user.")?.into(),
            device: state["device_id"]
                .as_str()
                .ok_or("No Matrix device.")?
                .into(),
            homeserver: state["homeserver_url"].as_str().unwrap_or_default().into(),
            verified: state["is_verified"] == true,
        })
    }

    async fn room_state(&mut self, room: &str, refetch: bool) -> Result<Vec<Value>> {
        let data = self
            .rpc(
                "get_room_state",
                json!({"room_id":room,"fetch_members":refetch,"refetch":refetch,"include_members":true}),
            )
            .await?;
        Ok(data
            .as_array()
            .ok_or("Room state unavailable.")?
            .iter()
            .map(state_event)
            .collect())
    }

    async fn send_event(&mut self, room: &str, kind: &str, content: Value) -> Result<String> {
        let event = self
            .rpc(
                "send_event",
                json!({"room_id":room,"type":kind,"synchronous":true,"content":content}),
            )
            .await?;
        Ok(event["event_id"].as_str().unwrap_or_default().into())
    }

    async fn send_to_device(
        &mut self,
        kind: &str,
        content: Value,
        targets: &[(String, String)],
    ) -> Result<()> {
        let mut messages = json!({});
        for (user, device) in targets {
            messages[user][device] = content.clone();
        }
        self.rpc(
            "send_to_device",
            json!({"event_type":kind,"encrypted":true,"messages":messages}),
        )
        .await
        .map(drop)
    }

    async fn openid_token(&mut self) -> Result<Value> {
        self.rpc("request_openid_token", json!({})).await
    }

    async fn rtc_transports(&mut self) -> Result<Vec<Value>> {
        Ok(self
            .rpc("get_rtc_transports", json!({}))
            .await?
            .get("rtc_transports")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn next(&mut self, _room: &str) -> Result<Vec<CallUpdate>> {
        if self.queued.is_empty() {
            self.frame().await?;
        }
        Ok(std::mem::take(&mut self.queued))
    }

    /// Frames read while waiting for an RPC answer.
    fn ready(&mut self) -> Vec<CallUpdate> {
        std::mem::take(&mut self.queued)
    }

    async fn start(&mut self) -> Result<()> {
        // Element may send its key as soon as it sees our membership.
        self.prior_listen = Some(self.rpc("listen_to_device", json!(true)).await?);
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(prior) = self.prior_listen.take() {
            let _ = self.rpc("listen_to_device", prior).await;
        }
    }
}

/// gomuks's `get_room_state` answer as Matrix state events.
pub fn room_state(data: &Value) -> Value {
    Value::Array(crate::rooms::items(data).map(state_event).collect())
}

/// A gomuks event as a Matrix state event.
pub fn state_event(e: &Value) -> Value {
    let mut event = json!({
        "type": e["type"], "state_key": e["state_key"], "sender": e["sender"],
        "event_id": e["event_id"], "content": e["content"],
    });
    if let Some(ts) = e["timestamp"].as_u64() {
        event["origin_server_ts"] = ts.into();
    }
    event
}

/// What a gomuks frame means for a call in `room`.
fn updates(f: &Value, room: &str) -> Vec<CallUpdate> {
    let mut out = Vec::new();
    if f["command"] == "sync_complete" {
        let state = &f["data"]["rooms"][room]["state"];
        if state.get(EVENT_TYPE).is_some()
            || state.get("m.room.member").is_some()
            || crate::rooms::items(&f["data"]["left_rooms"]).any(|r| r == room)
        {
            out.push(CallUpdate::Roster);
        }
        for e in crate::rooms::items(&f["data"]["to_device"]) {
            if e["type"] == KEY
                && e["encrypted"] == true
                && e["content"]["room_id"] == room
                && let Some(sender) = e["sender"].as_str()
            {
                out.push(CallUpdate::Key(CallKey {
                    sender: sender.into(),
                    content: e["content"].clone(),
                }));
            }
        }
    }
    // Room keys may arrive after the invite response's timeline sync. Use both
    // gomuks event paths, and require an encrypted, server-confirmed reference.
    let delta = match f["command"].as_str() {
        Some("sync_complete") => &f["data"]["rooms"][room],
        Some("events_decrypted") if f["data"]["room_id"] == room => &f["data"],
        _ => return out,
    };
    for e in crate::rooms::items(&delta["events"]) {
        let rel = &e["decrypted"]["m.relates_to"];
        if e["room_id"] == room
            && e["type"] == "m.room.encrypted"
            && e["decrypted_type"] == DECLINE
            && crate::rooms::server_confirmed(e)
            && !e["redacted_by"].is_string()
            && rel["rel_type"] == "m.reference"
            && let Some(sender) = e["sender"].as_str()
            && let Some(id) = rel["event_id"]
                .as_str()
                .filter(|id| id.starts_with('$') && id.len() > 1)
        {
            out.push(CallUpdate::Decline {
                sender: sender.into(),
                refers_to: id.into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_decrypted_declines_keep_matching_references_but_ignore_wrong_room_or_relation() {
        let event = |eid: &str, u: &str, rel: &str| {
            json!({"room_id":"!dm","event_id":eid,"sender":u,
            "type":"m.room.encrypted","decrypted_type":DECLINE,
            "decrypted":{"m.relates_to":{"rel_type":rel,"event_id":eid}}})
        };
        let late = json!({"command":"events_decrypted","data":{"room_id":"!dm","events":[
            event("$ours","@alice","m.reference"),event("$old","@alice","m.reference"),
            event("$wrong","@bob","m.reference"),event("$bad","@alice","m.annotation")]}});
        assert!(updates(&late, "!different").is_empty());
        let declines: Vec<_> = updates(&late, "!dm")
            .into_iter()
            .map(|u| match u {
                CallUpdate::Decline { sender, refers_to } => (sender, refers_to),
                _ => panic!("only declines"),
            })
            .collect();
        assert_eq!(
            declines,
            [
                ("@alice".into(), "$ours".into()),
                ("@alice".into(), "$old".into()),
                ("@bob".into(), "$wrong".into()),
            ]
        );
    }

    #[test]
    fn sync_frames_report_roster_changes_and_only_encrypted_keys_for_the_room() {
        let key = |room: &str, encrypted: bool| json!({"type":KEY,"sender":"@b:h","encrypted":encrypted,"content":{"room_id":room}});
        let f = json!({"command":"sync_complete","data":{
            "rooms":{"!r":{"state":{EVENT_TYPE:{}}}},
            "to_device":[key("!r", true), key("!r", false), key("!other", true)]}});
        assert_eq!(
            updates(&f, "!r"),
            [
                CallUpdate::Roster,
                CallUpdate::Key(CallKey {
                    sender: "@b:h".into(),
                    content: json!({"room_id":"!r"})
                })
            ]
        );
        let state = state_event(&json!({"type":EVENT_TYPE,"state_key":"_k","sender":"@b:h",
            "event_id":"$e","timestamp":5,"rowid":9,"content":{}}));
        assert_eq!(state["origin_server_ts"], 5);
        assert!(state.get("rowid").is_none());
    }
}
