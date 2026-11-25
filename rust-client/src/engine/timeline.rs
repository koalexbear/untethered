//! matrix-sdk-ui timelines for opened rooms, mapped into [`Message`]s.
//!
//! The Timeline aggregates edits, redactions and local echoes itself. Each
//! batch of diffs produces a full snapshot of the room's loaded messages; the
//! UI keeps a bounded window of it, so this stays cheap at today's sizes.
use super::{Delivery, Event, Failure, Kind, Message, Reply, TimelineSnapshot};
use async_channel::Sender;
use futures_util::StreamExt;
use matrix_sdk::{
    Client,
    room::edit::EditedContent,
    ruma::{
        EventId, OwnedRoomId, RoomId,
        events::room::message::{
            MessageType, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
        },
    },
};
use matrix_sdk_ui::{
    Timeline,
    eyeball_im::Vector,
    timeline::{
        EventSendState, EventTimelineItem, MsgLikeKind, RoomExt, TimelineDetails,
        TimelineEventItemId, TimelineItem, TimelineItemContent,
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, Notify};

/// How many events one "Earlier messages" request loads, like gomuks.
const PAGE: u16 = 50;

struct Open {
    timeline: Arc<Timeline>,
    start: Arc<AtomicBool>,
    refresh: Arc<Notify>,
    forwarder: tokio::task::JoinHandle<()>,
}
impl Drop for Open {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

pub struct Timelines {
    client: Client,
    events: Sender<Event>,
    open: Mutex<HashMap<OwnedRoomId, Arc<Open>>>,
}

impl Timelines {
    pub fn new(client: Client, events: Sender<Event>) -> Self {
        Self {
            client,
            events,
            open: Default::default(),
        }
    }

    async fn get(&self, room: &str) -> Result<Arc<Open>, Failure> {
        let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
        let mut open = self.open.lock().await;
        if let Some(existing) = open.get(&id) {
            return Ok(existing.clone());
        }
        let joined = self.client.get_room(&id).ok_or(Failure::Unavailable)?;
        let timeline = Arc::new(joined.timeline().await.map_err(|_| Failure::Failed)?);
        let start = Arc::new(AtomicBool::new(false));
        let refresh = Arc::new(Notify::new());
        let forwarder = tokio::spawn(forward(
            room.to_owned(),
            self.client
                .user_id()
                .map(ToString::to_string)
                .unwrap_or_default(),
            timeline.clone(),
            start.clone(),
            refresh.clone(),
            self.events.clone(),
        ));
        let entry = Arc::new(Open {
            timeline,
            start,
            refresh,
            forwarder,
        });
        open.insert(id, entry.clone());
        Ok(entry)
    }

    pub async fn paginate(&self, room: &str) -> Result<(), Failure> {
        let open = self.get(room).await?;
        let start = open
            .timeline
            .paginate_backwards(PAGE)
            .await
            .map_err(|_| Failure::Unreachable)?;
        if start {
            open.start.store(true, Ordering::Release);
        }
        open.refresh.notify_one();
        Ok(())
    }

    pub async fn send(
        &self,
        room: &str,
        body: String,
        reply_to: Option<String>,
    ) -> Result<(), Failure> {
        let open = self.get(room).await?;
        match reply_to {
            // Replies stay literal text, as with gomuks's base_content: a
            // slash or Markdown in a reply is never a command.
            Some(target) => {
                let target = EventId::parse(target).map_err(|_| Failure::Unavailable)?;
                open.timeline
                    .send_reply(
                        RoomMessageEventContentWithoutRelation::text_plain(body),
                        target,
                    )
                    .await
                    .map_err(|_| Failure::Failed)?;
            }
            // Messages render Markdown like gomuks's send_message.
            None => {
                open.timeline
                    .send(RoomMessageEventContent::text_markdown(body).into())
                    .await
                    .map_err(|_| Failure::Failed)?;
            }
        }
        Ok(())
    }

    fn item(event: &str) -> Result<TimelineEventItemId, Failure> {
        Ok(TimelineEventItemId::EventId(
            EventId::parse(event).map_err(|_| Failure::Unavailable)?,
        ))
    }

    pub async fn react(&self, room: &str, event: &str, key: &str) -> Result<(), Failure> {
        let open = self.get(room).await?;
        open.timeline
            .toggle_reaction(&Self::item(event)?, key)
            .await
            .map(|_| ())
            .map_err(|_| Failure::Failed)
    }

    pub async fn edit(&self, room: &str, event: &str, body: String) -> Result<(), Failure> {
        let open = self.get(room).await?;
        open.timeline
            .edit(
                &Self::item(event)?,
                EditedContent::RoomMessage(RoomMessageEventContentWithoutRelation::text_markdown(
                    body,
                )),
            )
            .await
            .map_err(|_| Failure::Failed)
    }
}

async fn forward(
    room: String,
    me: String,
    timeline: Arc<Timeline>,
    start: Arc<AtomicBool>,
    refresh: Arc<Notify>,
    events: Sender<Event>,
) {
    let (mut items, mut stream) = timeline.subscribe().await;
    let mut rows = Rows::default();
    loop {
        let snapshot = snapshot(&room, &me, &items, &mut rows, start.load(Ordering::Acquire));
        if events.send(Event::Timeline(snapshot)).await.is_err() {
            return;
        }
        tokio::select! {
            diffs = stream.next() => match diffs {
                Some(diffs) => {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                }
                None => return,
            },
            _ = refresh.notified() => {}
        }
    }
}

/// Stable positive row numbers per timeline item, so the UI's scroll anchors
/// survive updates and local echoes keep their place when they are sent.
#[derive(Default)]
struct Rows {
    ids: HashMap<String, i64>,
    next: i64,
}
impl Rows {
    fn get(&mut self, key: &str) -> i64 {
        if let Some(row) = self.ids.get(key) {
            return *row;
        }
        self.next += 1;
        self.ids.insert(key.to_owned(), self.next);
        self.next
    }
}

fn snapshot(
    room: &str,
    me: &str,
    items: &Vector<Arc<TimelineItem>>,
    rows: &mut Rows,
    start: bool,
) -> TimelineSnapshot {
    TimelineSnapshot {
        room: room.to_owned(),
        start_reached: start || items.iter().any(|i| i.is_timeline_start()),
        messages: items
            .iter()
            .filter_map(|item| {
                let event = item.as_event()?;
                message(event, rows.get(&item.unique_id().0), me)
            })
            .collect(),
    }
}

fn message(item: &EventTimelineItem, row: i64, me: &str) -> Option<Message> {
    let TimelineItemContent::MsgLike(content) = item.content() else {
        return None;
    };
    let mut text = false;
    let mut mentions = None;
    let (kind, body, edited) = match &content.kind {
        MsgLikeKind::Message(m) => {
            let kind = match m.msgtype() {
                MessageType::Image(_)
                | MessageType::File(_)
                | MessageType::Audio(_)
                | MessageType::Video(_) => Kind::Attachment,
                _ => Kind::Text,
            };
            text = matches!(m.msgtype(), MessageType::Text(_));
            mentions = m
                .mentions()
                .map(|m| (m.user_ids.iter().map(ToString::to_string).collect(), m.room));
            (kind, m.body().to_owned(), m.is_edited())
        }
        MsgLikeKind::Sticker(s) => (Kind::Sticker, s.content().body.clone(), false),
        MsgLikeKind::Redacted => (Kind::Redacted, String::new(), false),
        MsgLikeKind::UnableToDecrypt(_) => (Kind::Undecryptable, String::new(), false),
        // Polls, live locations and custom events have no Post form yet.
        _ => return None,
    };
    let reactions = content
        .reactions
        .iter()
        .map(|(key, senders)| {
            (
                key.clone(),
                senders.len() as u64,
                senders.keys().any(|u| u.as_str() == me),
            )
        })
        .collect();
    let delivery = match item.send_state() {
        None | Some(EventSendState::Sent { .. }) => Delivery::Sent,
        Some(EventSendState::NotSentYet { .. }) => Delivery::Sending,
        Some(EventSendState::SendingFailed { .. }) => Delivery::Failed,
    };
    let reply_to = content.in_reply_to.as_ref().map(|r| Reply {
        event_id: r.event_id.to_string(),
        quoted: match &r.event {
            TimelineDetails::Ready(e) => e
                .content
                .as_message()
                .map(|m| (e.sender.to_string(), m.body().to_owned())),
            _ => None,
        },
    });
    Some(Message {
        row,
        event_id: item.event_id().map(ToString::to_string),
        sender: item.sender().to_string(),
        sender_name: match item.sender_profile() {
            TimelineDetails::Ready(p) => p.display_name.clone(),
            _ => None,
        },
        time: u64::from(item.timestamp().0) as i64,
        body,
        kind,
        encrypted: item.encryption_info().is_some() || kind == Kind::Undecryptable,
        delivery,
        edited,
        reply_to,
        reactions,
        text,
        editable: item.is_editable(),
        mentions,
        edit_id: item
            .latest_edit_json()
            .and_then(|raw| raw.get_field::<String>("event_id").ok().flatten()),
        receipts: item
            .read_receipts()
            .iter()
            .map(|(user, receipt)| {
                (
                    user.to_string(),
                    receipt.ts.map_or(0, |t| u64::from(t.0) as i64),
                )
            })
            .collect(),
    })
}
