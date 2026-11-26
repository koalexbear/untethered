//! Messaging features beyond sending: reactions and edits (through the
//! room's matrix-sdk-ui Timeline), reports, read receipts and the unread
//! flag, events around a message outside the loaded timeline, and the
//! account's read receipt preference.
use super::{Delivery, Event, Failure, Kind, Message, Reply, media::Sources};
use async_channel::Sender;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::TimelineEvent,
    room::Receipts,
    ruma::{
        EventId, OwnedEventId, RoomId, UInt,
        events::{
            AnyGlobalAccountDataEvent, AnySyncMessageLikeEvent, AnySyncTimelineEvent,
            GlobalAccountDataEventType, SyncMessageLikeEvent,
            room::message::{MessageType, Relation},
        },
        serde::Raw,
    },
};
use serde_json::{Value, json};

/// Shared with `reading.rs`, which reads it on the gomuks engine.
const READ_SHARING: &str = "com.untethered.read_receipts.v1";

pub fn room(client: &Client, room: &str) -> Result<Room, Failure> {
    let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
    client.get_room(&id).ok_or(Failure::Unavailable)
}

pub fn event_id(event: &str) -> Result<OwnedEventId, Failure> {
    EventId::parse(event).map_err(|_| Failure::Unavailable)
}

pub(super) fn http(e: &matrix_sdk::Error) -> Failure {
    if e.client_api_error_kind().is_some() {
        Failure::Failed
    } else {
        Failure::Unreachable
    }
}

pub async fn report(client: &Client, room: &str, event: &str, reason: &str) -> Result<(), Failure> {
    let reason = (!reason.trim().is_empty()).then(|| reason.to_owned());
    self::room(client, room)?
        .report_content(event_id(event)?, reason)
        .await
        .map(|_| ())
        .map_err(|e| http(&e))
}

pub async fn mark_read(
    client: &Client,
    room: &str,
    event: &str,
    public: bool,
) -> Result<(), Failure> {
    let room = self::room(client, room)?;
    let id = event_id(event)?;
    let receipts = Receipts::new().fully_read_marker(id.clone());
    let receipts = if public {
        receipts.public_read_receipt(id)
    } else {
        receipts.private_read_receipt(id)
    };
    room.send_multiple_receipts(receipts)
        .await
        .map_err(|e| http(&e))?;
    room.set_unread_flag(false).await.map_err(|e| http(&e))
}

pub async fn mark_unread(client: &Client, room: &str) -> Result<(), Failure> {
    self::room(client, room)?
        .set_unread_flag(true)
        .await
        .map_err(|e| http(&e))
}

/// The event and up to 12 events on each side, from `/context`.
pub async fn context(
    client: &Client,
    room: &str,
    event: &str,
    sources: &Sources,
) -> Result<Vec<Message>, Failure> {
    let me = client
        .user_id()
        .map(ToString::to_string)
        .unwrap_or_default();
    let room = self::room(client, room)?;
    let target = event_id(event)?;
    let context = room
        .event_with_context(&target, false, UInt::from(12u32), None)
        .await
        .map_err(|e| http(&e))?;
    let found = context.event.ok_or(Failure::Unavailable)?;
    let ordered = context
        .events_before
        .iter()
        .rev()
        .chain(std::iter::once(&found))
        .chain(context.events_after.iter());
    Ok(ordered
        .zip(1..)
        .filter_map(|(event, row)| message(event, row, &me, sources))
        .collect())
}

/// A single event outside a Timeline, mapped like a timeline item.
fn message(event: &TimelineEvent, row: i64, me: &str, sources: &Sources) -> Option<Message> {
    let encrypted = event.encryption_info().is_some();
    let parsed = event.raw().deserialize().ok();
    let (id, sender, time) = match &parsed {
        Some(any) => (
            any.event_id().to_string(),
            any.sender().to_string(),
            u64::from(any.origin_server_ts().0) as i64,
        ),
        None => {
            let raw: Value = event.raw().deserialize_as().ok()?;
            (
                raw["event_id"].as_str()?.to_owned(),
                raw["sender"].as_str()?.to_owned(),
                raw["origin_server_ts"].as_i64().unwrap_or(0),
            )
        }
    };
    let base = Message {
        row,
        event_id: Some(id),
        sender,
        sender_name: None,
        sender_avatar: None,
        time,
        body: String::new(),
        formatted: None,
        kind: Kind::Undecryptable,
        encrypted,
        delivery: Delivery::Sent,
        edited: false,
        reply_to: None,
        reactions: Vec::new(),
        text: false,
        editable: false,
        mentions: None,
        edit_id: None,
        edit_time: None,
        receipts: Vec::new(),
        media: None,
    };
    match parsed {
        Some(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(m))) => {
            let SyncMessageLikeEvent::Original(m) = m else {
                return Some(Message {
                    kind: Kind::Redacted,
                    ..base
                });
            };
            if matches!(m.content.relates_to, Some(Relation::Replacement(_))) {
                return None;
            }
            let reply_to = match &m.content.relates_to {
                Some(Relation::Reply(r)) => Some(Reply {
                    event_id: r.in_reply_to.event_id.to_string(),
                    quoted: None,
                }),
                _ => None,
            };
            let (kind, media) = match &m.content.msgtype {
                MessageType::Image(_)
                | MessageType::File(_)
                | MessageType::Audio(_)
                | MessageType::Video(_) => {
                    sources.remember(&m.content.msgtype);
                    (
                        Kind::Attachment,
                        serde_json::to_value(&m.content.msgtype).ok(),
                    )
                }
                _ => (Kind::Text, None),
            };
            Some(Message {
                body: m.content.body().to_owned(),
                formatted: super::timeline::html_body(&m.content.msgtype),
                kind,
                media,
                text: matches!(m.content.msgtype, MessageType::Text(_)),
                editable: m.sender == me,
                reply_to,
                ..base
            })
        }
        Some(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Sticker(s))) => {
            let SyncMessageLikeEvent::Original(s) = s else {
                return Some(Message {
                    kind: Kind::Redacted,
                    ..base
                });
            };
            Some(Message {
                body: s.content.body,
                kind: Kind::Sticker,
                ..base
            })
        }
        Some(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(_))) => {
            Some(base)
        }
        _ => None,
    }
}

pub async fn share_reads(client: &Client, public: bool) -> Result<(), Failure> {
    let content = serde_json::value::to_raw_value(&json!({ "public": public }))
        .map_err(|_| Failure::Failed)?;
    client
        .account()
        .set_account_data_raw(
            GlobalAccountDataEventType::from(READ_SHARING),
            Raw::from_json(content),
        )
        .await
        .map(|_| ())
        .map_err(|e| http(&e))
}

/// Report the read-sharing preference and the push rules now and whenever
/// they sync.
pub async fn account_data(client: Client, events: Sender<Event>) {
    let current = |kind: GlobalAccountDataEventType| {
        let client = client.clone();
        async move {
            client
                .account()
                .account_data_raw(kind)
                .await
                .ok()
                .flatten()
                .and_then(|raw| raw.deserialize_as::<Value>().ok())
        }
    };
    let public = current(GlobalAccountDataEventType::from(READ_SHARING))
        .await
        .is_some_and(|v| v["public"] == true);
    let _ = events.send(Event::ReadSharing(public)).await;
    if let Some(rules) = current(GlobalAccountDataEventType::PushRules).await {
        let _ = events.send(Event::PushRules(rules)).await;
    }
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
        let events = events.clone();
        async move {
            let Ok(value) = raw.deserialize_as::<Value>() else {
                return;
            };
            let event = match value["type"].as_str() {
                Some(READ_SHARING) => Event::ReadSharing(value["content"]["public"] == true),
                Some("m.push_rules") => Event::PushRules(value["content"].clone()),
                _ => return,
            };
            let _ = events.send(event).await;
        }
    });
}
