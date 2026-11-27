//! Messaging features beyond sending: reactions and edits (through the
//! room's matrix-sdk-ui Timeline), reports, read receipts and the unread
//! flag, events around a message outside the loaded timeline, and the
//! account's read receipt and typing notice preferences.
use super::{Delivery, Event, Failure, Kind, Message, Reply, media::Sources, typing};
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

pub async fn redact(client: &Client, room: &str, event: &str) -> Result<(), Failure> {
    self::room(client, room)?
        .redact(&event_id(event)?, None, None)
        .await
        .map(|_| ())
        .map_err(|e| http(&e.into()))
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
    // The event may be in an older room this one was upgraded from, whose
    // history the App shows above this room's (upgrades.rs).
    let mut first = None;
    for (i, id) in super::upgrades::lineage(client, room)
        .await
        .iter()
        .enumerate()
    {
        match context_in(client, id, event, sources).await {
            Ok(mut messages) => {
                if i > 0 {
                    for m in &mut messages {
                        m.from = Some(id.clone());
                    }
                }
                return Ok(messages);
            }
            Err(failure) => {
                first.get_or_insert(failure);
            }
        }
    }
    Err(first.unwrap_or(Failure::Unavailable))
}

async fn context_in(
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
        kind: Kind::Undecryptable(super::Unreadable::Waiting),
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
        from: None,
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
    set_preference(client, READ_SHARING, json!({ "public": public })).await
}

/// Replace the account data of `kind` (one of our own preferences).
pub(super) async fn set_preference(
    client: &Client,
    kind: &str,
    content: Value,
) -> Result<(), Failure> {
    let content = serde_json::value::to_raw_value(&content).map_err(|_| Failure::Failed)?;
    client
        .account()
        .set_account_data_raw(
            GlobalAccountDataEventType::from(kind),
            Raw::from_json(content),
        )
        .await
        .map(|_| ())
        .map_err(|e| http(&e))
}

/// Report the read-sharing and typing notice preferences, the push rules
/// and who this account blocks, now and whenever they sync.
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
    let typing = current(GlobalAccountDataEventType::from(typing::SHARING))
        .await
        .is_none_or(|v| typing::sends(&v));
    let _ = events.send(Event::TypingSharing(typing)).await;
    if let Some(rules) = current(GlobalAccountDataEventType::PushRules).await {
        let _ = events.send(Event::PushRules(rules)).await;
    }
    let blocked = current(GlobalAccountDataEventType::IgnoredUserList)
        .await
        .map(|v| super::dms::blocked(&v))
        .unwrap_or_default();
    let _ = events.send(Event::Blocked(blocked)).await;
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
        let events = events.clone();
        async move {
            let Ok(value) = raw.deserialize_as::<Value>() else {
                return;
            };
            let event = match value["type"].as_str() {
                Some(READ_SHARING) => Event::ReadSharing(value["content"]["public"] == true),
                Some(typing::SHARING) => Event::TypingSharing(typing::sends(&value["content"])),
                Some("m.push_rules") => Event::PushRules(value["content"].clone()),
                Some("m.ignored_user_list") => {
                    Event::Blocked(super::dms::blocked(&value["content"]))
                }
                _ => return,
            };
            let _ = events.send(event).await;
        }
    });
}
