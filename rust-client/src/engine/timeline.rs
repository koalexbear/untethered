//! matrix-sdk-ui timelines for opened rooms, mapped into [`Message`]s.
//!
//! The Timeline aggregates edits, redactions and local echoes itself. Each
//! batch of diffs produces a full snapshot of the room's loaded messages; the
//! UI keeps a bounded window of it, so this stays cheap at today's sizes.
use super::{Delivery, Event, Failure, Kind, Message, Reply, TimelineSnapshot, media::Sources};
use async_channel::Sender;
use futures_util::{FutureExt, StreamExt};
use matrix_sdk::{
    Client,
    room::edit::EditedContent,
    ruma::{
        EventId, OwnedEventId, OwnedRoomId, RoomId,
        events::room::message::{
            FormattedBody, MessageFormat, MessageType, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        },
    },
    send_queue::RoomSendQueueUpdate,
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
    sync::{Arc, Mutex as Shared},
    time::Duration,
};
use tokio::sync::{Mutex, Notify, broadcast};

/// How many events one "Earlier messages" request loads, like gomuks.
const PAGE: u16 = 50;

/// The Timeline's start-of-room item, once a back-pagination has shown the
/// events it loaded. The item goes away when the Timeline is cleared.
type Start = Arc<Shared<Option<String>>>;

struct Open {
    timeline: Arc<Timeline>,
    start: Start,
    refresh: Arc<Notify>,
    forwarder: super::rt::JoinHandle<()>,
}
impl Drop for Open {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

pub struct Timelines {
    client: Client,
    events: Sender<Event>,
    sources: Arc<Sources>,
    open: Mutex<HashMap<OwnedRoomId, Arc<Open>>>,
}

impl Timelines {
    pub fn new(client: Client, events: Sender<Event>, sources: Arc<Sources>) -> Self {
        Self {
            client,
            events,
            sources,
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
        let start = Start::default();
        let refresh = Arc::new(Notify::new());
        let forwarder = super::rt::spawn(forward(
            room.to_owned(),
            self.client
                .user_id()
                .map(ToString::to_string)
                .unwrap_or_default(),
            timeline.clone(),
            start.clone(),
            refresh.clone(),
            self.events.clone(),
            self.sources.clone(),
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
            self.settle(room, &open.timeline).await;
            let marker = open
                .timeline
                .items()
                .await
                .iter()
                .find(|i| i.is_timeline_start())
                .map(|i| i.unique_id().0.clone());
            *open.start.lock().unwrap() = marker;
        }
        open.refresh.notify_one();
        Ok(())
    }

    /// `paginate_backwards` returns once the event cache has the events; the
    /// Timeline adds them on its own task, which can be later (it is when
    /// they come from the store after a restart). Wait until the oldest ones
    /// show, so the start is never reported above a hole.
    async fn settle(&self, room: &str, timeline: &Timeline) {
        let Some(joined) = RoomId::parse(room)
            .ok()
            .and_then(|id| self.client.get_room(&id))
        else {
            return;
        };
        let Some(Ok((cache, _handles))) = (match self.client.event_cache() {
            Some(caches) => Some(caches.room(joined.room_id()).await),
            None => None,
        }) else {
            return;
        };
        // Some of the oldest events may have no item of their own (an edit,
        // a reaction), so any of a few will do: the Timeline applies one
        // batch from the event cache at once.
        let oldest: Vec<OwnedEventId> = match cache.events().await {
            Ok(events) => events
                .iter()
                .filter_map(|e| e.event_id().map(ToOwned::to_owned))
                .take(10)
                .collect(),
            Err(_) => return,
        };
        for _ in 0..200 {
            for id in &oldest {
                if timeline.item_by_event_id(id).await.is_some() {
                    return;
                }
            }
            super::rt::sleep(Duration::from_millis(10)).await;
        }
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

    /// Send an attachment message and wait for the homeserver's answer. It
    /// goes through the room's send queue like text, so it shows as a local
    /// echo meanwhile. On failure it leaves the queue: the App's attachment
    /// tray offers the retry.
    pub async fn send_media(
        &self,
        room: &str,
        content: serde_json::Value,
        reply_to: Option<String>,
    ) -> Result<(), Failure> {
        let open = self.get(room).await?;
        let content: RoomMessageEventContent =
            serde_json::from_value(content).map_err(|_| Failure::Failed)?;
        if !is_attachment(&content.msgtype) {
            return Err(Failure::Failed);
        }
        let joined = RoomId::parse(room)
            .ok()
            .and_then(|id| self.client.get_room(&id))
            .ok_or(Failure::Unavailable)?;
        let queue = joined.send_queue();
        let (_, mut updates) = queue.subscribe().await.map_err(|_| Failure::Failed)?;
        let handle = match reply_to {
            Some(target) => {
                let target = EventId::parse(target).map_err(|_| Failure::Unavailable)?;
                let mut reply = RoomMessageEventContentWithoutRelation::new(content.msgtype);
                reply.mentions = content.mentions;
                open.timeline.send_reply(reply, target).await
            }
            None => open.timeline.send(content.into()).await,
        }
        .map_err(|_| Failure::Failed)?;
        let ours = handle.transaction_id().to_owned();
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Err(Failure::Failed),
            };
            match update {
                RoomSendQueueUpdate::SentEvent { transaction_id, .. } if transaction_id == ours => {
                    return Ok(());
                }
                RoomSendQueueUpdate::SendError { transaction_id, .. } if transaction_id == ours => {
                    let _ = handle.abort().await;
                    // Any error disables the room's queue; nothing of ours
                    // is left in it now.
                    queue.set_enabled(true);
                    return Err(Failure::Unreachable);
                }
                RoomSendQueueUpdate::CancelledLocalEvent { transaction_id }
                    if transaction_id == ours =>
                {
                    return Err(Failure::Cancelled);
                }
                _ => {}
            }
        }
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

/// The HTML body of a text, notice or emote (`org.matrix.custom.html`).
pub(super) fn html_body(msgtype: &MessageType) -> Option<String> {
    match msgtype {
        MessageType::Text(t) => t.formatted.as_ref(),
        MessageType::Notice(n) => n.formatted.as_ref(),
        MessageType::Emote(e) => e.formatted.as_ref(),
        _ => None,
    }
    .filter(|f: &&FormattedBody| f.format == MessageFormat::Html)
    .map(|f| f.body.clone())
}

fn is_attachment(msgtype: &MessageType) -> bool {
    matches!(
        msgtype,
        MessageType::Image(_)
            | MessageType::File(_)
            | MessageType::Audio(_)
            | MessageType::Video(_)
    )
}

/// Any send error disables the room's send queue until someone re-enables
/// it. After a network error, try again after a pause; the queue still holds
/// the request and resends it under the same transaction ID.
pub async fn resume_after_errors(client: Client) {
    let mut errors = client.send_queue().subscribe_errors();
    loop {
        match errors.recv().await {
            Ok(error) if error.is_recoverable => {
                let client = client.clone();
                super::rt::spawn(async move {
                    super::rt::sleep(Duration::from_secs(5)).await;
                    if let Some(room) = client.get_room(&error.room_id) {
                        room.send_queue().set_enabled(true);
                    }
                });
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn forward(
    room: String,
    me: String,
    timeline: Arc<Timeline>,
    start: Start,
    refresh: Arc<Notify>,
    events: Sender<Event>,
    sources: Arc<Sources>,
) {
    let (mut items, mut stream) = timeline.subscribe().await;
    let mut rows = Rows::default();
    loop {
        let marker = start.lock().unwrap().clone();
        let latest = timeline.latest_event_id().await.map(|e| e.to_string());
        let mut snapshot = snapshot(&room, &me, &items, &mut rows, marker.as_deref(), &sources);
        snapshot.latest_event = latest;
        if events.send(Event::Timeline(snapshot)).await.is_err() {
            return;
        }
        tokio::select! {
            biased;
            diffs = stream.next() => match diffs {
                Some(diffs) => {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                }
                None => return,
            },
            _ = refresh.notified() => {
                // A pagination that confirmed the start waited for its
                // events; take everything the Timeline has emitted so far.
                while let Some(next) = stream.next().now_or_never() {
                    let Some(diffs) = next else { return };
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                }
            }
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
    start: Option<&str>,
    sources: &Sources,
) -> TimelineSnapshot {
    TimelineSnapshot {
        room: room.to_owned(),
        start_reached: start.is_some_and(|marker| {
            items
                .iter()
                .any(|i| i.is_timeline_start() && i.unique_id().0 == marker)
        }),
        messages: items
            .iter()
            .filter_map(|item| {
                let event = item.as_event()?;
                message(event, rows.get(&item.unique_id().0), me, sources)
            })
            .collect(),
        latest_event: None,
    }
}

fn message(item: &EventTimelineItem, row: i64, me: &str, sources: &Sources) -> Option<Message> {
    let TimelineItemContent::MsgLike(content) = item.content() else {
        return None;
    };
    let mut text = false;
    let mut mentions = None;
    let mut media = None;
    let mut formatted = None;
    let (kind, body, edited) = match &content.kind {
        MsgLikeKind::Message(m) => {
            let kind = match m.msgtype() {
                MessageType::Image(_)
                | MessageType::File(_)
                | MessageType::Audio(_)
                | MessageType::Video(_) => {
                    sources.remember(m.msgtype());
                    media = serde_json::to_value(m.msgtype()).ok();
                    Kind::Attachment
                }
                _ => Kind::Text,
            };
            text = matches!(m.msgtype(), MessageType::Text(_));
            formatted = html_body(m.msgtype());
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
    let reactions = item
        .reactions()
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
        sender_avatar: match item.sender_profile() {
            TimelineDetails::Ready(p) => p.avatar_url.as_ref().map(ToString::to_string),
            _ => None,
        },
        time: u64::from(item.timestamp().0) as i64,
        body,
        formatted,
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
        edit_time: item
            .latest_edit_json()
            .and_then(|raw| raw.get_field::<i64>("origin_server_ts").ok().flatten()),
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
        media,
    })
}
