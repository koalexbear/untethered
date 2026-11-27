//! matrix-sdk-ui timelines for opened rooms, mapped into [`Message`]s.
//!
//! The Timeline aggregates edits, redactions and local echoes itself. Each
//! batch of diffs produces a full snapshot of the room's loaded messages; the
//! UI keeps a bounded window of it, so this stays cheap at today's sizes.
//!
//! An opened room is a chain of segments, one Timeline each: the room itself,
//! then each older room it was upgraded from (upgrades.rs), added once
//! scrolling back reaches the start of the one before. One snapshot carries
//! them all, oldest first, with a seam where each room's history begins.
use super::{
    Delivery, Event, Failure, Kind, Message, Reply, TimelineSnapshot, Unreadable,
    media::Sources,
    upgrades::{self, Above, Seam},
};
use async_channel::Sender;
use futures_util::{FutureExt, Stream, StreamExt};
use matrix_sdk::{
    Client,
    room::edit::EditedContent,
    ruma::{
        EventId, OwnedEventId, OwnedRoomId, OwnedTransactionId, RoomId,
        events::{
            AnyMessageLikeEventContent, Mentions,
            room::message::{
                FormattedBody, MessageFormat, MessageType, RoomMessageEventContent,
                RoomMessageEventContentWithoutRelation,
            },
        },
    },
    send_queue::{RoomSendQueue, RoomSendQueueUpdate, SendHandle},
};
use matrix_sdk_crypto::types::events::UtdCause;
use matrix_sdk_ui::{
    Timeline,
    eyeball_im::{Vector, VectorDiff},
    timeline::{
        EventSendState, EventTimelineItem, MsgLikeKind, RoomExt, TimelineDetails,
        TimelineEventItemId, TimelineItem, TimelineItemContent,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as Shared, Weak},
    time::Duration,
};
use tokio::sync::{Mutex, Notify, broadcast};

/// How many events one "Earlier messages" request loads, like gomuks.
const PAGE: u16 = 50;

/// The Timeline's start-of-room item, once a back-pagination has shown the
/// events it loaded. The item goes away when the Timeline is cleared.
type Start = Arc<Shared<Option<String>>>;

/// One room's Timeline, mirrored for the opened rooms that show it: its own
/// and those of the rooms that were upgraded from it.
struct Segment {
    room: OwnedRoomId,
    timeline: Arc<Timeline>,
    start: Start,
    mirror: Arc<Mirror>,
    task: super::rt::JoinHandle<()>,
}
impl Drop for Segment {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Segment {
    fn listen(&self, changed: &Arc<Notify>) {
        self.mirror
            .listeners
            .lock()
            .unwrap()
            .push(Arc::downgrade(changed));
    }

    /// A pagination confirmed the start, and the Timeline still shows it.
    async fn at_start(&self) -> bool {
        let Some(marker) = self.start.lock().unwrap().clone() else {
            return false;
        };
        self.timeline
            .items()
            .await
            .iter()
            .any(|i| i.is_timeline_start() && i.unique_id().0 == marker)
    }
}

/// A Timeline's items as of its latest batch of diffs.
#[derive(Default)]
struct Mirror {
    items: Shared<Vector<Arc<TimelineItem>>>,
    /// Take everything the Timeline has emitted so far, now.
    refresh: Notify,
    /// The emitters of the opened rooms that show these items.
    listeners: Shared<Vec<Weak<Notify>>>,
}
impl Mirror {
    fn wake(&self) {
        self.listeners
            .lock()
            .unwrap()
            .retain(|listener| match listener.upgrade() {
                Some(changed) => {
                    changed.notify_one();
                    true
                }
                None => false,
            });
    }
}

/// An opened room, which emits the timeline snapshots for its chain.
struct Open {
    chain: Arc<Shared<Chain>>,
    changed: Arc<Notify>,
    emitter: super::rt::JoinHandle<()>,
}
impl Drop for Open {
    fn drop(&mut self) {
        self.emitter.abort();
    }
}
impl Open {
    fn segments(&self) -> Vec<Arc<Segment>> {
        self.chain.lock().unwrap().segments.clone()
    }
    /// The opened room's own Timeline, where new messages go.
    fn timeline(&self) -> Arc<Timeline> {
        self.chain.lock().unwrap().segments[0].timeline.clone()
    }
}

/// Segments newest first; the first is the opened room.
#[derive(Default)]
struct Chain {
    segments: Vec<Arc<Segment>>,
    /// `seams[i]` joins `segments[i]` to the older `segments[i + 1]`, and
    /// one more sits above the oldest when its predecessor can't be read.
    seams: Vec<Seam>,
    /// Nothing older remains to load.
    complete: bool,
}

pub struct Timelines {
    client: Client,
    events: Sender<Event>,
    sources: Arc<Sources>,
    open: Mutex<HashMap<OwnedRoomId, Arc<Open>>>,
    segments: Mutex<HashMap<OwnedRoomId, Arc<Segment>>>,
    keys: Arc<Keys>,
}

/// History older than this device has its keys in key backup only. The SDK
/// fetches them one session at a time as each message fails to decrypt;
/// far back in a room (an older room an upgrade replaced, especially) that
/// is a long wait behind every page. So the first time a room shows
/// messages waiting for keys, ask the backup for all of that room's keys at
/// once; the SDK then decrypts what they open on its own.
#[derive(Default)]
struct Keys {
    /// Rooms whose backup keys were asked for this session.
    asked: Shared<std::collections::HashSet<String>>,
}
impl Keys {
    fn fetch(self: &Arc<Self>, client: &Client, rooms: impl IntoIterator<Item = String>) {
        for room in rooms {
            if !self.asked.lock().unwrap().insert(room.clone()) {
                continue;
            }
            let client = client.clone();
            super::rt::spawn(async move {
                let backups = client.encryption().backups();
                if let Ok(id) = RoomId::parse(&room)
                    && backups.are_enabled().await
                {
                    let _ = backups.download_room_keys_for_room(&id).await;
                }
            });
        }
    }
}

impl Timelines {
    pub fn new(client: Client, events: Sender<Event>, sources: Arc<Sources>) -> Self {
        Self {
            client,
            events,
            sources,
            open: Default::default(),
            segments: Default::default(),
            keys: Default::default(),
        }
    }

    async fn segment(&self, id: &RoomId) -> Result<Arc<Segment>, Failure> {
        let mut segments = self.segments.lock().await;
        if let Some(existing) = segments.get(id) {
            return Ok(existing.clone());
        }
        let room = self.client.get_room(id).ok_or(Failure::Unavailable)?;
        let timeline = Arc::new(room.timeline().await.map_err(|_| Failure::Failed)?);
        let (items, stream) = timeline.subscribe().await;
        let shared = Arc::new(Mirror {
            items: Shared::new(items),
            ..Default::default()
        });
        let task = super::rt::spawn(mirror(stream, shared.clone()));
        let segment = Arc::new(Segment {
            room: id.to_owned(),
            timeline,
            start: Start::default(),
            mirror: shared,
            task,
        });
        segments.insert(id.to_owned(), segment.clone());
        Ok(segment)
    }

    async fn get(&self, room: &str) -> Result<Arc<Open>, Failure> {
        let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
        let mut open = self.open.lock().await;
        if let Some(existing) = open.get(&id) {
            return Ok(existing.clone());
        }
        let segment = self.segment(&id).await?;
        let changed = Arc::new(Notify::new());
        segment.listen(&changed);
        let chain = Arc::new(Shared::new(Chain {
            segments: vec![segment],
            ..Default::default()
        }));
        let emitter = super::rt::spawn(emit(
            room.to_owned(),
            self.client.clone(),
            chain.clone(),
            changed.clone(),
            self.events.clone(),
            self.sources.clone(),
            self.keys.clone(),
        ));
        let entry = Arc::new(Open {
            chain,
            changed,
            emitter,
        });
        open.insert(id, entry.clone());
        Ok(entry)
    }

    /// Stop showing `room`: drop its Timeline, and those of the older rooms
    /// its chain reached that no other open room shows. Their events stay
    /// in the event cache on disk, and with no Timeline listening the SDK
    /// lets go of all but the newest in memory; opening the room again
    /// reads them back from disk.
    pub async fn close(&self, room: &str) {
        let Ok(id) = RoomId::parse(room) else {
            return;
        };
        let mut open = self.open.lock().await;
        if open.remove(&id).is_none() {
            return;
        }
        let shown: std::collections::HashSet<OwnedRoomId> = open
            .values()
            .flat_map(|o| o.segments())
            .map(|s| s.room.clone())
            .collect();
        drop(open);
        self.segments
            .lock()
            .await
            .retain(|room, _| shown.contains(room));
    }

    /// Load older messages: from the newest segment whose start isn't shown
    /// yet. Reaching the start of the oldest continues into the room it was
    /// upgraded from, which loads a page at once so the seam isn't left at
    /// the top of what's shown.
    pub async fn paginate(&self, room: &str) -> Result<(), Failure> {
        let open = self.get(room).await?;
        loop {
            let segments = open.segments();
            let mut target = None;
            for (i, segment) in segments.iter().enumerate() {
                if !segment.at_start().await {
                    target = Some(i);
                    break;
                }
            }
            let Some(i) = target else {
                // Every start is shown already (an older room opened on its
                // own reached it): continue above the oldest.
                let oldest = segments.last().expect("a chain starts with its room");
                if open.chain.lock().unwrap().complete || !self.extend(&open, oldest).await {
                    break;
                }
                continue;
            };
            let segment = &segments[i];
            let start = segment
                .timeline
                .paginate_backwards(PAGE)
                .await
                .map_err(|_| Failure::Unreachable)?;
            if start {
                self.settle(segment).await;
                let marker = segment
                    .timeline
                    .items()
                    .await
                    .iter()
                    .find(|i| i.is_timeline_start())
                    .map(|i| i.unique_id().0.clone());
                *segment.start.lock().unwrap() = marker;
            }
            segment.mirror.refresh.notify_one();
            // A segment cleared and paged back to its start already has the
            // older ones after it.
            if !start || i + 1 < segments.len() || !self.extend(&open, segment).await {
                break;
            }
        }
        open.changed.notify_one();
        Ok(())
    }

    /// Continue `open`'s chain above `oldest`, whose start is now shown.
    /// Whether an older room's segment was added.
    async fn extend(&self, open: &Open, oldest: &Segment) -> bool {
        let above = match self.client.get_room(&oldest.room) {
            Some(room) => upgrades::above(&self.client, &room).await,
            None => Above::Start,
        };
        let (older, seam) = match above {
            Above::Start => {
                open.chain.lock().unwrap().complete = true;
                return false;
            }
            Above::Unreadable(seam) => (None, seam),
            Above::Older(room, seam) => {
                let looped = open
                    .segments()
                    .iter()
                    .any(|s| s.room.as_str() == room.room_id().as_str());
                let segment = match looped {
                    true => None,
                    false => self.segment(room.room_id()).await.ok(),
                };
                (segment, seam)
            }
        };
        let mut chain = open.chain.lock().unwrap();
        match older {
            Some(segment) => {
                segment.listen(&open.changed);
                chain.seams.push(seam);
                chain.segments.push(segment);
                true
            }
            None => {
                chain.seams.push(Seam {
                    readable: false,
                    ..seam
                });
                chain.complete = true;
                false
            }
        }
    }

    /// `paginate_backwards` returns once the event cache has the events; the
    /// Timeline adds them on its own task, which can be later (it is when
    /// they come from the store after a restart). Wait until the oldest ones
    /// show, so the start is never reported above a hole.
    async fn settle(&self, segment: &Segment) {
        let Some(joined) = self.client.get_room(&segment.room) else {
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
                if segment.timeline.item_by_event_id(id).await.is_some() {
                    return;
                }
            }
            super::rt::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The Timeline of the room that owns `event`, among `room`'s segments:
    /// reactions and edits go where the event is. The room's own otherwise.
    async fn owner(
        &self,
        room: &str,
        event: &TimelineEventItemId,
    ) -> Result<Arc<Timeline>, Failure> {
        let segments = self.get(room).await?.segments();
        if let TimelineEventItemId::EventId(id) = event {
            for segment in &segments[1..] {
                if segment.timeline.item_by_event_id(id).await.is_some() {
                    return Ok(segment.timeline.clone());
                }
            }
        }
        Ok(segments[0].timeline.clone())
    }

    pub async fn send(
        &self,
        room: &str,
        body: String,
        pills: &[crate::pills::Pill],
        reply_to: Option<String>,
    ) -> Result<(), Failure> {
        let timeline = self.get(room).await?.timeline();
        match reply_to {
            // Replies stay literal text, as with gomuks's base_content: a
            // slash or Markdown in a reply is never a command.
            Some(target) => {
                let target = EventId::parse(target).map_err(|_| Failure::Unavailable)?;
                timeline
                    .send_reply(super::compose::literal(body, pills), target)
                    .await
                    .map_err(|_| Failure::Failed)?;
            }
            // Messages render Markdown like gomuks's send_message.
            None => {
                timeline
                    .send(
                        RoomMessageEventContent::from(super::compose::markdown(body, pills)).into(),
                    )
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
        let timeline = self.get(room).await?.timeline();
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
                timeline.send_reply(reply, target).await
            }
            None => timeline.send(content.into()).await,
        }
        .map_err(|_| Failure::Failed)?;
        sent(&queue, &mut updates, vec![handle]).await
    }

    /// Forward `event`, a message `room` shows, to the room `to` (ROADMAP
    /// M8), as Element does: its content sent again with no reply or
    /// thread, so an attachment reuses its file, and its key, rather than
    /// uploading it again. Nobody it mentions is notified again, as on
    /// Discord. `note` follows as a message of its own.
    pub async fn forward(
        &self,
        room: &str,
        event: &str,
        to: &str,
        note: Option<String>,
    ) -> Result<(), Failure> {
        let item = Self::item(event)?;
        let TimelineEventItemId::EventId(id) = &item else {
            return Err(Failure::Unavailable);
        };
        let found = self
            .owner(room, &item)
            .await?
            .item_by_event_id(id)
            .await
            .ok_or(Failure::Unavailable)?;
        let content = forwarded(found.content()).ok_or(Failure::Failed)?;
        let target = RoomId::parse(to)
            .ok()
            .and_then(|id| self.client.get_room(&id))
            .ok_or(Failure::Unavailable)?;
        // Through the room's send queue rather than a Timeline, so a room
        // nobody has open doesn't start showing; one that's open shows it
        // as a local echo meanwhile.
        let queue = target.send_queue();
        let (_, mut updates) = queue.subscribe().await.map_err(|_| Failure::Failed)?;
        let mut handles = vec![queue.send(content).await.map_err(|_| Failure::Failed)?];
        if let Some(note) = note.filter(|n| !n.trim().is_empty()) {
            let note = RoomMessageEventContent::from(super::compose::markdown(note, &[]));
            match queue.send(note.into()).await {
                Ok(handle) => handles.push(handle),
                Err(_) => {
                    let _ = handles[0].abort().await;
                    return Err(Failure::Failed);
                }
            }
        }
        sent(&queue, &mut updates, handles).await
    }

    fn item(event: &str) -> Result<TimelineEventItemId, Failure> {
        Ok(TimelineEventItemId::EventId(
            EventId::parse(event).map_err(|_| Failure::Unavailable)?,
        ))
    }

    pub async fn react(&self, room: &str, event: &str, key: &str) -> Result<(), Failure> {
        let item = Self::item(event)?;
        self.owner(room, &item)
            .await?
            .toggle_reaction(&item, key)
            .await
            .map(|_| ())
            .map_err(|_| Failure::Failed)
    }

    pub async fn edit(
        &self,
        room: &str,
        event: &str,
        body: String,
        pills: &[crate::pills::Pill],
    ) -> Result<(), Failure> {
        let item = Self::item(event)?;
        self.owner(room, &item)
            .await?
            .edit(
                &item,
                EditedContent::RoomMessage(super::compose::markdown(body, pills)),
            )
            .await
            .map_err(|_| Failure::Failed)
    }
}

/// What the SDK guesses about an undecryptable message.
fn unreadable(cause: Option<UtdCause>) -> Unreadable {
    match cause.unwrap_or_default() {
        UtdCause::Unknown => Unreadable::Waiting,
        UtdCause::SentBeforeWeJoined => Unreadable::BeforeJoined,
        UtdCause::HistoricalMessageAndBackupIsDisabled => Unreadable::NoBackup,
        UtdCause::HistoricalMessageAndDeviceIsUnverified => Unreadable::Unverified,
        UtdCause::WithheldForUnverifiedOrInsecureDevice | UtdCause::WithheldBySender => {
            Unreadable::Withheld
        }
        UtdCause::VerificationViolation | UtdCause::UnsignedDevice | UtdCause::UnknownDevice => {
            Unreadable::Untrusted
        }
        UtdCause::Malformed => Unreadable::Malformed,
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

/// Wait for the homeserver to accept what `handles` queued, in one room. On
/// a failure, what's still queued leaves the queue and the queue starts
/// again: the App offers the retry.
async fn sent(
    queue: &RoomSendQueue,
    updates: &mut broadcast::Receiver<RoomSendQueueUpdate>,
    handles: Vec<SendHandle>,
) -> Result<(), Failure> {
    let mut waiting: Vec<OwnedTransactionId> = handles
        .iter()
        .map(|h| h.transaction_id().to_owned())
        .collect();
    while !waiting.is_empty() {
        let update = match updates.recv().await {
            Ok(update) => update,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return Err(Failure::Failed),
        };
        match update {
            RoomSendQueueUpdate::SentEvent { transaction_id, .. } => {
                waiting.retain(|t| *t != transaction_id);
            }
            RoomSendQueueUpdate::SendError { transaction_id, .. }
                if waiting.contains(&transaction_id) =>
            {
                for handle in &handles {
                    let _ = handle.abort().await;
                }
                // Any error disables the room's queue; nothing of ours is
                // left in it now.
                queue.set_enabled(true);
                return Err(Failure::Unreachable);
            }
            RoomSendQueueUpdate::CancelledLocalEvent { transaction_id }
                if waiting.contains(&transaction_id) =>
            {
                return Err(Failure::Cancelled);
            }
            _ => {}
        }
    }
    Ok(())
}

/// What forwarding a timeline item sends: a message's content without its
/// reply, thread or mentions, or a sticker as it is. Polls, locations
/// shared live and the like aren't shown here, so they can't be forwarded.
fn forwarded(content: &TimelineItemContent) -> Option<AnyMessageLikeEventContent> {
    let TimelineItemContent::MsgLike(like) = content else {
        return None;
    };
    match &like.kind {
        MsgLikeKind::Message(m) => {
            let msgtype = m.msgtype();
            if !matches!(
                msgtype,
                MessageType::Text(_)
                    | MessageType::Notice(_)
                    | MessageType::Emote(_)
                    | MessageType::Location(_)
            ) && !is_attachment(msgtype)
            {
                return None;
            }
            let mut content = RoomMessageEventContent::new(msgtype.clone());
            content.mentions = Some(Mentions::new());
            Some(content.into())
        }
        MsgLikeKind::Sticker(s) => {
            let mut sticker = s.content().clone();
            sticker.relates_to = None;
            Some(sticker.into())
        }
        _ => None,
    }
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

/// Keep a segment's items current, and wake the emitters that show them.
async fn mirror(
    mut stream: impl Stream<Item = Vec<VectorDiff<Arc<TimelineItem>>>> + Unpin,
    mirror: Arc<Mirror>,
) {
    loop {
        tokio::select! {
            biased;
            diffs = stream.next() => match diffs {
                Some(diffs) => {
                    let mut items = mirror.items.lock().unwrap();
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                }
                None => return,
            },
            _ = mirror.refresh.notified() => {
                // A pagination that confirmed the start waited for its
                // events; take everything the Timeline has emitted so far.
                while let Some(next) = stream.next().now_or_never() {
                    let Some(diffs) = next else { return };
                    let mut items = mirror.items.lock().unwrap();
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                }
            }
        }
        mirror.wake();
    }
}

/// Send the opened room's snapshot now and whenever one of its segments
/// changes.
async fn emit(
    room: String,
    client: Client,
    chain: Arc<Shared<Chain>>,
    changed: Arc<Notify>,
    events: Sender<Event>,
    sources: Arc<Sources>,
    keys: Arc<Keys>,
) {
    let me = client
        .user_id()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut rows = Rows::default();
    loop {
        let (segments, seams, complete) = {
            let chain = chain.lock().unwrap();
            (chain.segments.clone(), chain.seams.clone(), chain.complete)
        };
        let latest = segments[0]
            .timeline
            .latest_event_id()
            .await
            .map(|e| e.to_string());
        let mut snapshot = snapshot(&room, &me, &segments, &seams, complete, &mut rows, &sources);
        snapshot.latest_event = latest;
        let waiting: std::collections::BTreeSet<&str> = snapshot
            .messages
            .iter()
            .filter(|m| m.kind == Kind::Undecryptable(Unreadable::Waiting))
            .map(|m| m.from.as_deref().unwrap_or(&room))
            .collect();
        keys.fetch(&client, waiting.into_iter().map(str::to_owned));
        if events.send(Event::Timeline(snapshot)).await.is_err() {
            return;
        }
        changed.notified().await;
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
    segments: &[Arc<Segment>],
    seams: &[Seam],
    complete: bool,
    rows: &mut Rows,
    sources: &Sources,
) -> TimelineSnapshot {
    let parts = segments
        .iter()
        .map(|segment| {
            let items = segment.mirror.items.lock().unwrap().clone();
            let started = segment
                .start
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|marker| {
                    items
                        .iter()
                        .any(|i| i.is_timeline_start() && &i.unique_id().0 == marker)
                });
            let messages = items
                .iter()
                .filter_map(|item| {
                    let event = item.as_event()?;
                    let key = format!("{}\n{}", segment.room, item.unique_id().0);
                    message(event, rows.get(&key), me, sources)
                })
                .collect();
            Part {
                room: segment.room.to_string(),
                messages,
                started,
            }
        })
        .collect();
    let (messages, seams, start_reached) = assemble(parts, seams, complete);
    TimelineSnapshot {
        room: room.to_owned(),
        start_reached,
        messages,
        latest_event: None,
        seams,
    }
}

/// One segment's share of a snapshot.
struct Part {
    room: String,
    messages: Vec<Message>,
    /// Its start is shown: nothing of this room is missing above.
    started: bool,
}

/// The chain's messages oldest first, the seams between its rooms, and
/// whether this is the start of all the history there is. `parts` and
/// `seams` are as in [`Chain`].
fn assemble(
    mut parts: Vec<Part>,
    seams: &[Seam],
    complete: bool,
) -> (Vec<Message>, Vec<Seam>, bool) {
    // Older rooms show only while history is unbroken down to them: a
    // Timeline that was cleared no longer reaches its start.
    let shown = parts
        .iter()
        .position(|p| !p.started)
        .map_or(parts.len(), |i| i + 1);
    let whole = shown == parts.len() && parts.last().is_some_and(|p| p.started);
    parts.truncate(shown);
    let mut messages = Vec::new();
    let mut joins = Vec::new();
    // An older room that can't be read sits above everything.
    if whole && let Some(seam) = seams.get(shown - 1) {
        joins.push(seam.clone());
    }
    for (k, part) in parts.into_iter().enumerate().rev() {
        let from = (k > 0).then_some(part.room);
        messages.extend(part.messages.into_iter().map(|m| Message {
            from: from.clone(),
            ..m
        }));
        if k > 0 {
            joins.push(Seam {
                index: messages.len(),
                ..seams[k - 1].clone()
            });
        }
    }
    (messages, joins, whole && complete)
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
        // A sticker shows as its picture, animated or not (ROADMAP P1).
        MsgLikeKind::Sticker(s) => {
            let sticker = s.content();
            let mut image = matrix_sdk::ruma::events::room::message::ImageMessageEventContent::new(
                sticker.body.clone(),
                sticker.source.clone().into(),
            );
            image.info = Some(Box::new(sticker.info.clone()));
            let msgtype = MessageType::Image(image);
            sources.remember(&msgtype);
            media = serde_json::to_value(&msgtype).ok();
            (Kind::Attachment, sticker.body.clone(), false)
        }
        MsgLikeKind::Redacted => (Kind::Redacted, String::new(), false),
        MsgLikeKind::UnableToDecrypt(m) => (
            Kind::Undecryptable(unreadable(m.utd_cause())),
            String::new(),
            false,
        ),
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
        encrypted: item.encryption_info().is_some() || matches!(kind, Kind::Undecryptable(_)),
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
        from: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(event: &str) -> Message {
        Message {
            row: 0,
            event_id: Some(event.into()),
            sender: "@a:example".into(),
            sender_name: None,
            sender_avatar: None,
            time: 0,
            body: event.into(),
            formatted: None,
            kind: Kind::Text,
            encrypted: false,
            delivery: Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: false,
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
        }
    }

    fn part(room: &str, events: &[&str], started: bool) -> Part {
        Part {
            room: room.into(),
            messages: events.iter().map(|e| message(e)).collect(),
            started,
        }
    }

    fn seam(newer: &str, older: &str, readable: bool) -> Seam {
        Seam {
            index: 0,
            older: older.into(),
            newer: newer.into(),
            time: Some(1_700_000_000_000),
            readable,
        }
    }

    fn bodies(messages: &[Message]) -> Vec<(&str, Option<&str>)> {
        messages
            .iter()
            .map(|m| (m.body.as_str(), m.from.as_deref()))
            .collect()
    }

    #[test]
    fn several_upgrades_read_as_one_history_with_seams() {
        let (messages, seams, start) = assemble(
            vec![
                part("!c", &["c1", "c2"], true),
                part("!b", &["b1"], true),
                part("!a", &["a1", "a2"], true),
            ],
            &[seam("!c", "!b", true), seam("!b", "!a", true)],
            true,
        );
        assert_eq!(
            bodies(&messages),
            [
                ("a1", Some("!a")),
                ("a2", Some("!a")),
                ("b1", Some("!b")),
                ("c1", None),
                ("c2", None)
            ]
        );
        assert_eq!(
            seams
                .iter()
                .map(|s| (s.index, s.newer.as_str()))
                .collect::<Vec<_>>(),
            [(2, "!b"), (3, "!c")]
        );
        assert!(start);
    }

    #[test]
    fn an_unreadable_predecessor_ends_history_with_a_seam() {
        let (messages, seams, start) = assemble(
            vec![part("!r", &["r1"], true)],
            &[seam("!r", "!p", false)],
            true,
        );
        assert_eq!(bodies(&messages), [("r1", None)]);
        assert_eq!(seams.len(), 1);
        assert_eq!((seams[0].index, seams[0].readable), (0, false));
        assert!(start);
    }

    #[test]
    fn an_older_room_loading_keeps_the_start_unreached() {
        // The predecessor's latest messages show above the seam before its
        // start is reached.
        let (messages, seams, start) = assemble(
            vec![part("!r", &["r1"], true), part("!p", &["p9"], false)],
            &[seam("!r", "!p", true)],
            false,
        );
        assert_eq!(bodies(&messages), [("p9", Some("!p")), ("r1", None)]);
        assert_eq!(seams[0].index, 1);
        assert!(!start);
        // Before the room's own start: no seam yet.
        let (messages, seams, start) = assemble(vec![part("!r", &["r1"], false)], &[], false);
        assert_eq!(bodies(&messages), [("r1", None)]);
        assert!(seams.is_empty() && !start);
    }

    #[test]
    fn a_cleared_timeline_hides_the_older_rooms_behind_its_gap() {
        // The newest room's Timeline was reset (a gappy sync): its start is
        // no longer shown, so the older room would sit above a hole.
        let (messages, seams, start) = assemble(
            vec![part("!r", &["r7", "r8"], false), part("!p", &["p1"], true)],
            &[seam("!r", "!p", true)],
            true,
        );
        assert_eq!(bodies(&messages), [("r7", None), ("r8", None)]);
        assert!(seams.is_empty() && !start);
    }
}
