//! MatrixRTC call signaling on the matrix-sdk engine: the [`CallSignal`] that
//! the call layer (`desktop_call`) runs on, for one room.
//!
//! Call keys arrive as Olm-encrypted custom to-device messages; only those whose
//! Olm sender is known count. Roster changes come from the room's call
//! membership and member state; declines from the room's (decrypted) timeline.
use async_channel::{Receiver, Sender};
use futures_util::StreamExt;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::EncryptionInfo,
    event_handler::EventHandlerDropGuard,
    ruma::{
        DeviceId, RoomId, UserId,
        api::client::{
            account::request_openid_token, delayed_events::update_delayed_event::UpdateAction,
            state::get_state_events,
        },
        events::{AnySyncTimelineEvent, StateEventType, ToDeviceEventType},
        serde::Raw,
    },
};
use serde_json::{Value, json};
use std::time::Duration;
use untethered_iced::{
    rtc_keys::EVENT as KEY,
    rtc_membership::{CallKey, CallSignal, CallUpdate, Delayed, EVENT_TYPE, Identity, StateSignal},
};

const DECLINE: &str = "org.matrix.msc4310.rtc.decline";
const RING: &str = "org.matrix.msc4075.rtc.notification";

/// Pass call-related room events to the App as Matrix events (decrypted):
/// rings, declines and redactions from any room, and call memberships and
/// member changes in DMs, for incoming calls.
pub fn watch(client: &Client, events: async_channel::Sender<super::Event>) {
    let state = events.clone();
    client.add_event_handler(move |raw: Raw<AnySyncTimelineEvent>, room: Room| {
        let events = events.clone();
        async move { forward(raw.cast(), &room, &events).await }
    });
    client.add_event_handler(
        move |raw: Raw<matrix_sdk::ruma::events::AnySyncStateEvent>, room: Room| {
            let events = state.clone();
            async move { forward(raw.cast(), &room, &events).await }
        },
    );
}

async fn forward(raw: Raw<Value>, room: &Room, events: &async_channel::Sender<super::Event>) {
    let Ok(mut event) = raw.deserialize() else {
        return;
    };
    let dm = !room.direct_targets().is_empty();
    let wanted = match event["type"].as_str() {
        Some(RING | DECLINE | "m.room.redaction") => true,
        Some("m.room.member") => dm,
        Some(kind) => dm && kind == EVENT_TYPE,
        None => false,
    };
    if !wanted {
        return;
    }
    let id = room.room_id().to_string();
    event["room_id"] = id.clone().into();
    #[cfg(debug_assertions)]
    qa_log(&event);
    let _ = events
        .send(super::Event::CallActivity { room: id, event })
        .await;
}

/// Call fixtures (debug builds): every decrypted ring and decline this
/// device sees, sent or received, one JSON object per line in
/// `UNTETHERED_QA_CALL_EVENTS`. gomuks's database gave them the same.
#[cfg(debug_assertions)]
fn qa_log(event: &Value) {
    use std::io::Write;
    let Some(path) = std::env::var_os("UNTETHERED_QA_CALL_EVENTS") else {
        return;
    };
    if !matches!(event["type"].as_str(), Some(RING | DECLINE)) {
        return;
    }
    let line = json!({"room_id": event["room_id"], "event_id": event["event_id"],
        "sender": event["sender"], "type": event["type"], "content": event["content"]});
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// The room's state from the homeserver, as Matrix state events.
pub async fn call_state(
    client: &Client,
    room: &str,
) -> std::result::Result<Vec<Value>, super::Failure> {
    let id = RoomId::parse(room).map_err(|_| super::Failure::Unavailable)?;
    let response = client
        .send(get_state_events::v3::Request::new(id))
        .await
        .map_err(|_| super::Failure::Unreachable)?;
    Ok(response
        .room_state
        .iter()
        .filter_map(|raw| raw.deserialize_as::<Value>().ok())
        .collect())
}

/// Decline a ring, as Element Call does.
pub async fn decline_ring(
    client: &Client,
    room: &str,
    ring: &str,
) -> std::result::Result<(), super::Failure> {
    let room = super::messaging::room(client, room)?;
    room.send_raw(
        DECLINE,
        json!({"m.relates_to":{"rel_type":"m.reference","event_id":ring},"msc4354_sticky_key":ring}),
    )
    .await
    .map(drop)
    .map_err(|e| super::messaging::http(&e))
}

type Result<T> = std::result::Result<T, &'static str>;

pub struct SdkSignal {
    client: Client,
    room: Room,
    updates: Receiver<CallUpdate>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _declines: EventHandlerDropGuard,
}

impl Drop for SdkSignal {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl SdkSignal {
    pub fn new(client: Client, room: &str) -> Result<Self> {
        let id = RoomId::parse(room).map_err(|_| "Unknown call room.")?;
        let room = client
            .get_room(&id)
            .ok_or("Join the room before calling.")?;
        let (sender, updates) = async_channel::unbounded();
        let mut tasks = Vec::new();
        let keys =
            client.subscribe_to_custom_to_device_messages(vec![ToDeviceEventType::from(KEY)]);
        tasks.push(tokio::spawn(forward_keys(keys, sender.clone())));
        for kind in [StateEventType::from(EVENT_TYPE), StateEventType::RoomMember] {
            let changes = room.subscribe_to_state_events(kind);
            let sender = sender.clone();
            tasks.push(tokio::spawn(async move {
                let mut changes = std::pin::pin!(changes);
                // The first item is the state as it is now.
                let _ = changes.next().await;
                while changes.next().await.is_some() {
                    if sender.send(CallUpdate::Roster).await.is_err() {
                        return;
                    }
                }
            }));
        }
        let declines = client.add_room_event_handler(
            room.room_id(),
            move |raw: Raw<AnySyncTimelineEvent>, info: Option<EncryptionInfo>| {
                let sender = sender.clone();
                async move {
                    if let Some(decline) = decline(&raw, info.is_some()) {
                        let _ = sender.send(decline).await;
                    }
                }
            },
        );
        Ok(Self {
            _declines: client.event_handler_drop_guard(declines),
            client,
            room,
            updates,
            tasks,
        })
    }
}

async fn forward_keys(
    keys: impl futures_util::Stream<Item = matrix_sdk::ToDeviceMessage>,
    sender: Sender<CallUpdate>,
) {
    let mut keys = std::pin::pin!(keys);
    while let Some(message) = keys.next().await {
        // Plaintext keys are not trusted: anyone could have sent them.
        let Some(info) = &message.encryption_info else {
            continue;
        };
        let Ok(Some(content)) = message.raw.get_field::<Value>("content") else {
            continue;
        };
        let key = CallKey {
            sender: info.sender.to_string(),
            content,
        };
        if sender.send(CallUpdate::Key(key)).await.is_err() {
            return;
        }
    }
}

/// An encrypted decline referencing a ring.
fn decline(raw: &Raw<AnySyncTimelineEvent>, encrypted: bool) -> Option<CallUpdate> {
    let event: Value = raw.deserialize_as().ok()?;
    let rel = &event["content"]["m.relates_to"];
    (encrypted
        && event["type"] == DECLINE
        && rel["rel_type"] == "m.reference"
        && event["unsigned"]["redacted_because"].is_null())
    .then_some(())?;
    let refers_to = rel["event_id"]
        .as_str()
        .filter(|id| id.starts_with('$') && id.len() > 1)?;
    Some(CallUpdate::Decline {
        sender: event["sender"].as_str()?.into(),
        refers_to: refers_to.into(),
    })
}

fn failed(e: &matrix_sdk::Error) -> String {
    if e.client_api_error_kind().is_some() {
        "Homeserver rejected the call request.".into()
    } else {
        "Lost the Matrix call connection.".into()
    }
}

impl SdkSignal {
    fn check_room(&self, room: &str) -> std::result::Result<(), String> {
        (self.room.room_id().as_str() == room)
            .then_some(())
            .ok_or_else(|| "Unknown call room.".into())
    }
}

impl StateSignal for SdkSignal {
    async fn set_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
    ) -> std::result::Result<String, String> {
        self.check_room(room)?;
        let response = self
            .room
            .send_state_event_raw(kind, key, content)
            .await
            .map_err(|e| failed(&e))?;
        Ok(response.event_id.to_string())
    }

    async fn set_delayed_state(
        &mut self,
        room: &str,
        kind: &str,
        key: &str,
        content: Value,
        delay_ms: u32,
    ) -> std::result::Result<String, String> {
        self.check_room(room)?;
        let response = self
            .room
            .send_state_event_raw(kind, key, content)
            .into_delayed(Duration::from_millis(delay_ms.into()))
            .await
            .map_err(|e| failed(&e))?;
        Ok(response.delay_id)
    }

    async fn update_delayed(
        &mut self,
        delay_id: &str,
        action: Delayed,
    ) -> std::result::Result<(), String> {
        let action = match action {
            Delayed::Cancel => UpdateAction::Cancel,
            Delayed::Restart => UpdateAction::Restart,
            Delayed::Send => UpdateAction::Send,
        };
        self.client
            .update_delayed_event(delay_id.to_owned(), action, None)
            .await
            .map(drop)
            .map_err(|e| failed(&e))
    }
}

impl CallSignal for SdkSignal {
    async fn identity(&mut self) -> Result<Identity> {
        let user = self.client.user_id().ok_or("No Matrix user.")?;
        let device = self.client.device_id().ok_or("No Matrix device.")?;
        let verified = self
            .client
            .encryption()
            .get_own_device()
            .await
            .ok()
            .flatten()
            .is_some_and(|d| d.is_cross_signed_by_owner());
        Ok(Identity {
            user: user.to_string(),
            device: device.to_string(),
            homeserver: self.client.homeserver().to_string(),
            verified,
        })
    }

    /// Always the homeserver's state: the store may not hold every member or
    /// call membership the checks need.
    async fn room_state(&mut self, room: &str, _refetch: bool) -> Result<Vec<Value>> {
        self.check_room(room).map_err(|_| "Unknown call room.")?;
        let response = self
            .client
            .send(get_state_events::v3::Request::new(
                self.room.room_id().to_owned(),
            ))
            .await
            .map_err(|_| "Room state unavailable.")?;
        Ok(response
            .room_state
            .iter()
            .filter_map(|raw| raw.deserialize_as::<Value>().ok())
            .collect())
    }

    async fn send_event(&mut self, room: &str, kind: &str, content: Value) -> Result<String> {
        self.check_room(room).map_err(|_| "Unknown call room.")?;
        let sent = self
            .room
            .send_raw(kind, content)
            .await
            .map_err(|_| "Homeserver rejected the call request.")?;
        Ok(sent.response.event_id.to_string())
    }

    async fn send_to_device(
        &mut self,
        kind: &str,
        content: Value,
        targets: &[(String, String)],
    ) -> Result<()> {
        let encryption = self.client.encryption();
        let mut devices = Vec::new();
        for (user, device) in targets {
            let (Ok(user), Some(device)) = (
                UserId::parse(user),
                Some(<&DeviceId>::from(device.as_str())),
            ) else {
                continue;
            };
            let mut found = encryption.get_device(&user, device).await.ok().flatten();
            if found.is_none() {
                // A device this client hasn't heard of yet, such as a new
                // device of someone who just joined: ask the server for that
                // user's keys once, as gomuks did before encrypting.
                let _ = encryption.request_user_identity(&user).await;
                found = encryption.get_device(&user, device).await.ok().flatten();
            }
            devices.extend(found);
        }
        if devices.is_empty() {
            return Err("The other devices' encryption keys are unknown.");
        }
        let raw = Raw::from_json(
            serde_json::value::to_raw_value(&content).map_err(|_| "Invalid call key.")?,
        );
        let failures = encryption
            .encrypt_and_send_raw_to_device(
                devices.iter().collect(),
                kind,
                raw,
                matrix_sdk_crypto::CollectStrategy::AllDevices,
            )
            .await
            .map_err(|_| "Could not send the encrypted call key.")?;
        if failures.len() == devices.len() {
            return Err("Could not send the encrypted call key.");
        }
        Ok(())
    }

    async fn openid_token(&mut self) -> Result<Value> {
        let user = self.client.user_id().ok_or("No Matrix user.")?.to_owned();
        let token = self
            .client
            .send(request_openid_token::v3::Request::new(user))
            .await
            .map_err(|_| "Could not get an OpenID token.")?;
        Ok(json!({
            "access_token": token.access_token,
            "token_type": token.token_type.to_string(),
            "matrix_server_name": token.matrix_server_name.to_string(),
            "expires_in": token.expires_in.as_secs(),
        }))
    }

    async fn rtc_transports(&mut self) -> Result<Vec<Value>> {
        let transports = self
            .client
            .discover_rtc_transports()
            .await
            .map_err(|_| "RTC transport discovery failed.")?
            .unwrap_or_default();
        Ok(transports
            .iter()
            .filter_map(|t| serde_json::to_value(t).ok())
            .collect())
    }

    async fn next(&mut self, room: &str) -> Result<Vec<CallUpdate>> {
        self.check_room(room).map_err(|_| "Unknown call room.")?;
        let first = self
            .updates
            .recv()
            .await
            .map_err(|_| "Lost the Matrix call connection.")?;
        let mut all = vec![first];
        all.extend(self.ready());
        Ok(all)
    }

    fn ready(&mut self) -> Vec<CallUpdate> {
        std::iter::from_fn(|| self.updates.try_recv().ok()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_encrypted_references_count_as_declines() {
        let raw = |content: Value| {
            Raw::from_json(
                serde_json::value::to_raw_value(&json!({"type":DECLINE,"sender":"@b:h",
                    "event_id":"$d","origin_server_ts":1,"content":content}))
                .unwrap(),
            )
        };
        let reference = json!({"m.relates_to":{"rel_type":"m.reference","event_id":"$ring"}});
        assert_eq!(
            decline(&raw(reference.clone()), true),
            Some(CallUpdate::Decline {
                sender: "@b:h".into(),
                refers_to: "$ring".into()
            })
        );
        assert_eq!(decline(&raw(reference), false), None, "plaintext");
        let annotation = json!({"m.relates_to":{"rel_type":"m.annotation","event_id":"$ring"}});
        assert_eq!(decline(&raw(annotation), true), None);
    }
}

/// Opt-in: every [`CallSignal`] operation against a disposable Synapse, between
/// two SDK clients (`scripts/qa_matrix_sdk.py` provides the accounts).
#[cfg(test)]
mod live {
    use super::*;
    use matrix_sdk::config::SyncSettings;
    use std::path::Path;
    use untethered_iced::rtc_membership::Membership;

    async fn client(server: &str, user: &str, password: &str, dir: &Path) -> Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .homeserver_url(server)
            .sqlite_store(dir, None)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(user, password)
            .initial_device_display_name("Untethered call test")
            .send()
            .await
            .unwrap();
        client
    }

    fn syncing(client: &Client) -> tokio::task::JoinHandle<()> {
        let client = client.clone();
        tokio::spawn(async move {
            loop {
                let settings = SyncSettings::default().timeout(Duration::from_millis(500));
                if client.sync_once(settings).await.is_err() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        })
    }

    /// Wait for a matching update from `signal`.
    async fn expect(
        signal: &mut SdkSignal,
        room: &str,
        label: &str,
        mut wanted: impl FnMut(&CallUpdate) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if signal.next(room).await.unwrap().iter().any(&mut wanted) {
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "run scripts/qa_matrix_sdk.py (disposable Synapse)"]
    async fn matrix_sdk_call_signaling() {
        let fixture: Value = serde_json::from_slice(
            &std::fs::read(std::env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap(),
        )
        .unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("UNTETHERED_QA_PROFILES").unwrap());
        let text = |key: &str| fixture[key].as_str().unwrap().to_owned();
        let (server, room) = (text("server"), text("room"));
        let alice = client(
            &server,
            &text("username"),
            &text("password"),
            &base.join("call-a"),
        )
        .await;
        let bob = client(
            &server,
            &text("peer"),
            &text("peer_password"),
            &base.join("call-b"),
        )
        .await;
        let syncs = [syncing(&alice), syncing(&bob)];
        let id = RoomId::parse(&room).unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while alice.get_room(&id).is_none() || bob.get_room(&id).is_none() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("both clients have the room");
        let mut a = SdkSignal::new(alice.clone(), &room).unwrap();
        let mut b = SdkSignal::new(bob.clone(), &room).unwrap();
        let me = a.identity().await.unwrap();
        let peer = b.identity().await.unwrap();
        assert_eq!(me.user, alice.user_id().unwrap().as_str());
        let joined = |state: &[Value], user: &str| {
            state.iter().any(|e| {
                e["type"] == "m.room.member"
                    && e["state_key"] == user
                    && e["content"]["membership"] == "join"
            })
        };
        let state = a.room_state(&room, true).await.unwrap();
        assert!(joined(&state, &me.user) && joined(&state, &peer.user));

        // A call membership with a delayed leave: the peer sees it, a
        // heartbeat restarts the delay, and leaving sends it.
        let service = url::Url::parse("https://rtc.test.invalid/").unwrap();
        let member = Membership::join(&room, &me.user, &me.device, &service, 30_000, &mut a)
            .await
            .unwrap();
        expect(&mut b, &room, "the roster change", |u| {
            *u == CallUpdate::Roster
        })
        .await;
        let ours = |state: &[Value]| {
            state
                .iter()
                .find(|e| e["type"] == EVENT_TYPE && e["sender"] == me.user.as_str())
                .map(|e| e["content"].clone())
        };
        let content = ours(&b.room_state(&room, true).await.unwrap()).unwrap();
        assert_eq!(content["device_id"], me.device.as_str());
        member.heartbeat(&mut a).await.unwrap();
        member.leave(&mut a).await.unwrap();
        let left = ours(&b.room_state(&room, true).await.unwrap()).unwrap();
        assert_eq!(left, json!({}), "the delayed leave was sent");
        println!("matrix-sdk: call membership with a delayed leave: joined, heartbeat, left");

        // A ring, declined by the peer in an encrypted reference.
        let ring = a
            .send_event(
                &room,
                "org.matrix.msc4075.rtc.notification",
                json!({"notification_type":"ring","m.mentions":{"user_ids":[peer.user]}}),
            )
            .await
            .unwrap();
        b.send_event(
            &room,
            DECLINE,
            json!({"m.relates_to":{"rel_type":"m.reference","event_id":ring}}),
        )
        .await
        .unwrap();
        expect(&mut a, &room, "the decline", |u| {
            *u == CallUpdate::Decline {
                sender: peer.user.clone(),
                refers_to: ring.clone(),
            }
        })
        .await;
        println!("matrix-sdk: an encrypted ring and the peer's decline");

        // An Olm-encrypted call key. The peer's new device may take a sync or
        // two to become known.
        let key = json!({"room_id": room, "member": {"claimed_device_id": me.device}});
        let target = [(peer.user.clone(), peer.device.clone())];
        let sent = tokio::time::timeout(Duration::from_secs(30), async {
            while a.send_to_device(KEY, key.clone(), &target).await.is_err() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await;
        assert!(sent.is_ok(), "the peer's device became known");
        expect(&mut b, &room, "the call key", |u| {
            matches!(u, CallUpdate::Key(k) if k.sender == me.user && k.content["room_id"] == room.as_str())
        })
        .await;
        println!("matrix-sdk: an Olm-encrypted call key reached the peer's device");

        let token = a.openid_token().await.unwrap();
        assert!(token["access_token"].is_string() && token["expires_in"].is_u64());
        assert!(
            a.rtc_transports().await.unwrap().is_empty(),
            "none advertised"
        );
        println!("matrix-sdk: OpenID token for the call service; no RTC transports advertised");
        for sync in syncs {
            sync.abort();
        }
    }

    /// Wait for the Element observer in `scripts/qa_rtc_membership.py` to
    /// create `name` in the fixture directory.
    async fn step(dir: &Path, name: &str) {
        tokio::time::timeout(Duration::from_secs(25), async {
            while !dir.join(name).is_file() {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the Element observer did not write {name}"));
    }

    /// The seeded device in UNTETHERED_PROFILE (signed in and verified by the
    /// fixture's `seed()`, opened with UNTETHERED_KEYRING=off) joins a
    /// MatrixRTC call while a separate Element Web session watches the room's
    /// roster: heartbeat, explicit leave, then a join it abandons, which only
    /// the homeserver's delayed leave can end. UNTETHERED_RTC_QA holds
    /// `rtc.json` and the files both sides pace each other with.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "run scripts/test-media-interop.py --rtc-membership with local Element Web"]
    async fn rtc_membership_seen_by_element() {
        use super::super::session;
        let dir = std::path::PathBuf::from(std::env::var_os("UNTETHERED_RTC_QA").unwrap());
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(dir.join("rtc.json")).unwrap()).unwrap();
        let room = fixture["room"].as_str().unwrap().to_owned();
        let service = url::Url::parse(fixture["service"].as_str().unwrap()).unwrap();
        assert_eq!(service.host_str(), Some("127.0.0.1"));
        let _ = rustls::crypto::ring::default_provider().install_default();
        let profile = session::Profile::open(session::profile_root().unwrap()).unwrap();
        let client = session::restore(&profile)
            .await
            .unwrap()
            .expect("the fixture seeded this profile");
        let sync = syncing(&client);
        let id = RoomId::parse(&room).unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while client.get_room(&id).is_none() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the seeded device synced its room");
        let mut signal = SdkSignal::new(client.clone(), &room).unwrap();
        let me = signal.identity().await.unwrap();
        assert_eq!(me.user, client.user_id().unwrap().as_str());
        std::fs::write(dir.join("rtc-device"), &me.device).unwrap();
        let member = Membership::join(&room, &me.user, &me.device, &service, 8000, &mut signal)
            .await
            .unwrap();
        step(&dir, "restart").await;
        member.heartbeat(&mut signal).await.unwrap();
        std::fs::write(dir.join("restarted"), "").unwrap();
        step(&dir, "leave").await;
        member.leave(&mut signal).await.unwrap();
        step(&dir, "rejoin").await;
        let _abandoned = Membership::join(&room, &me.user, &me.device, &service, 8000, &mut signal)
            .await
            .unwrap();
        // Stop syncing and exit without a leave or any more heartbeats.
        sync.abort();
        println!("matrix-sdk: joined, renewed, left and abandoned a call membership");
    }
}
