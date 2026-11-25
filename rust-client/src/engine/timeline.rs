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
    ruma::{
        EventId, OwnedRoomId, RoomId,
        events::room::message::{MessageType, RoomMessageEventContent},
    },
};
use matrix_sdk_ui::{
    Timeline,
    eyeball_im::Vector,
    timeline::{
        EventSendState, EventTimelineItem, MsgLikeKind, RoomExt, TimelineDetails, TimelineItem,
        TimelineItemContent,
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
        // Plain text, never slash commands: gomuks parity for replies and a
        // literal body for ordinary messages. Markdown rendering is a gap.
        let content = RoomMessageEventContent::text_plain(body);
        match reply_to {
            Some(target) => {
                let target = EventId::parse(target).map_err(|_| Failure::Unavailable)?;
                open.timeline
                    .send_reply(content.into(), target)
                    .await
                    .map_err(|_| Failure::Failed)?;
            }
            None => {
                open.timeline
                    .send(content.into())
                    .await
                    .map_err(|_| Failure::Failed)?;
            }
        }
        Ok(())
    }
}

async fn forward(
    room: String,
    timeline: Arc<Timeline>,
    start: Arc<AtomicBool>,
    refresh: Arc<Notify>,
    events: Sender<Event>,
) {
    let (mut items, mut stream) = timeline.subscribe().await;
    let mut rows = Rows::default();
    loop {
        let snapshot = snapshot(&room, &items, &mut rows, start.load(Ordering::Acquire));
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
                message(event, rows.get(&item.unique_id().0))
            })
            .collect(),
    }
}

fn message(item: &EventTimelineItem, row: i64) -> Option<Message> {
    let TimelineItemContent::MsgLike(content) = item.content() else {
        return None;
    };
    let (kind, body, edited) = match &content.kind {
        MsgLikeKind::Message(m) => {
            let kind = match m.msgtype() {
                MessageType::Image(_)
                | MessageType::File(_)
                | MessageType::Audio(_)
                | MessageType::Video(_) => Kind::Attachment,
                _ => Kind::Text,
            };
            (kind, m.body().to_owned(), m.is_edited())
        }
        MsgLikeKind::Sticker(s) => (Kind::Sticker, s.content().body.clone(), false),
        MsgLikeKind::Redacted => (Kind::Redacted, String::new(), false),
        MsgLikeKind::UnableToDecrypt(_) => (Kind::Undecryptable, String::new(), false),
        // Polls, live locations and custom events have no Post form yet.
        _ => return None,
    };
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
    })
}
