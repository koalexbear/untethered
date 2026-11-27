//! Search (ROADMAP N7) and the mentions inbox.
//!
//! Unencrypted rooms are searched on the homeserver, so their whole history
//! counts, also what this device never loaded: its full-text search for
//! words, and its history with a sender and attachment filter for a search
//! by filters alone. Encrypted rooms are searched on this device, over what
//! the event cache keeps (every event it synced or paginated, decrypted);
//! `index` loads their older history into it.
use super::{Failure, Found, messaging::http};
use matrix_sdk::cross_process_lock::MappedCrossProcessLockState;
use matrix_sdk::{
    Client, Room, RoomMemberships,
    room::MessagesOptions,
    ruma::{
        OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UInt, UserId,
        api::client::{
            filter::{RoomEventFilter, UrlFilter},
            search::search_events::v3::{Categories, Criteria, OrderBy, Request},
        },
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
            room::message::{MessageType, Relation},
        },
        serde::Raw,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What `has:` asks for, as on Discord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Has {
    /// Any attachment.
    File,
    Image,
    Video,
    Sound,
    Link,
}

impl Has {
    fn matches(self, kind: &str, body: &str) -> bool {
        match self {
            Has::File => matches!(
                kind,
                "m.file" | "m.image" | "m.video" | "m.audio" | "m.sticker"
            ),
            Has::Image => matches!(kind, "m.image" | "m.sticker"),
            Has::Video => kind == "m.video",
            Has::Sound => kind == "m.audio",
            Has::Link => body.contains("https://") || body.contains("http://"),
        }
    }
    /// Only events with a `url` (any attachment) can match.
    fn needs_url(self) -> bool {
        self != Has::Link
    }
}

/// A search over some rooms.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub rooms: Vec<String>,
    /// Text that must appear, as typed (any case).
    pub text: String,
    /// `from:` as typed: a Matrix ID, or part of a name.
    pub from: Vec<String>,
    pub has: Vec<Has>,
    /// Only messages sent before and after these times (milliseconds).
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// A search's matches, newest first, and what it covered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomsFound {
    pub found: Vec<Found>,
    pub reach: Vec<Reach>,
    /// `from:` values no member of the rooms answers to.
    pub nobody: Vec<String>,
    /// More matched than `found` holds.
    pub more: bool,
}

/// How a room was searched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reach {
    pub room: String,
    /// On the homeserver, all its history; otherwise on this device.
    pub server: bool,
    /// On this device: the oldest message there was to search.
    pub oldest: Option<i64>,
}

/// Matches a search returns.
const LIMIT: usize = 100;
/// Requests to the homeserver per room (or per search) at most.
const PAGES: usize = 10;
/// Events loaded into an encrypted room's cache per `index`.
const INDEX: u16 = 2000;

/// One message's fields that searches look at.
struct Stored {
    found: Found,
    /// Its `msgtype`, or `m.sticker`.
    kind: String,
    mentions: Vec<String>,
}

/// What one event says, if it's a message: an original, or an edit of one.
enum Parsed {
    Message(Stored),
    Edit {
        target: OwnedEventId,
        sender: String,
        time: i64,
        body: String,
        mentions: Vec<String>,
    },
}

fn parse(raw: &Raw<AnySyncTimelineEvent>, room: &str) -> Option<Parsed> {
    let any = raw.deserialize().ok()?;
    let (event_id, sender, time) = (
        any.event_id().to_owned(),
        any.sender().to_string(),
        u64::from(any.origin_server_ts().0) as i64,
    );
    let (body, kind, mentions) = match any {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(m),
        )) => {
            if let Some(Relation::Replacement(r)) = &m.content.relates_to {
                return Some(Parsed::Edit {
                    target: r.event_id.clone(),
                    sender,
                    time,
                    body: r.new_content.msgtype.body().to_owned(),
                    mentions: r
                        .new_content
                        .mentions
                        .as_ref()
                        .map(|m| m.user_ids.iter().map(ToString::to_string).collect())
                        .unwrap_or_default(),
                });
            }
            let mentions = m
                .content
                .mentions
                .as_ref()
                .map(|m| m.user_ids.iter().map(ToString::to_string).collect())
                .unwrap_or_default();
            let kind = match &m.content.msgtype {
                MessageType::Image(_) => "m.image",
                MessageType::File(_) => "m.file",
                MessageType::Video(_) => "m.video",
                MessageType::Audio(_) => "m.audio",
                other => other.msgtype(),
            }
            .to_owned();
            (m.content.body().to_owned(), kind, mentions)
        }
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Sticker(
            SyncMessageLikeEvent::Original(s),
        )) => (s.content.body, "m.sticker".to_owned(), Vec::new()),
        _ => return None,
    };
    (!body.is_empty()).then(|| {
        Parsed::Message(Stored {
            found: Found {
                room: room.to_owned(),
                event_id: event_id.to_string(),
                sender,
                body,
                time,
            },
            kind,
            mentions,
        })
    })
}

/// Messages with edits applied (from the original sender only), newest
/// first.
fn assemble(events: impl IntoIterator<Item = Parsed>) -> Vec<Stored> {
    let mut originals: HashMap<String, Stored> = HashMap::new();
    let mut edits = Vec::new();
    for parsed in events {
        match parsed {
            Parsed::Message(s) => {
                originals.insert(s.found.event_id.clone(), s);
            }
            Parsed::Edit {
                target,
                sender,
                time,
                body,
                mentions,
            } => edits.push((target, sender, time, body, mentions)),
        }
    }
    edits.sort_by_key(|(_, _, time, ..)| *time);
    for (target, sender, _, body, mentions) in edits {
        if let Some(original) = originals.get_mut(target.as_str())
            && original.found.sender == sender
        {
            original.found.body = body;
            original.mentions = mentions;
        }
    }
    let mut all: Vec<Stored> = originals.into_values().collect();
    newest_first(&mut all);
    all
}

fn newest_first(all: &mut [Stored]) {
    all.sort_by(|a, b| {
        b.found
            .time
            .cmp(&a.found.time)
            .then_with(|| a.found.event_id.cmp(&b.found.event_id))
    });
}

/// A room's stored messages and stickers, newest first, with edits applied
/// and redacted or undecryptable ones left out.
async fn messages(client: &Client, room: &str) -> Result<Vec<Stored>, Failure> {
    let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
    let guard = match client
        .event_cache_store()
        .lock()
        .await
        .map_err(|_| Failure::Failed)?
    {
        MappedCrossProcessLockState::Clean(guard) | MappedCrossProcessLockState::Dirty(guard) => {
            guard
        }
    };
    let events = guard
        .get_room_events(&id, None, None)
        .await
        .map_err(|_| Failure::Failed)?;
    drop(guard);
    Ok(assemble(
        events.iter().filter_map(|event| parse(event.raw(), room)),
    ))
}

/// A room's stored messages and those of the older rooms it was upgraded
/// from (upgrades.rs), which the App shows as one channel, newest first.
async fn history(client: &Client, room: &str) -> Result<Vec<Stored>, Failure> {
    let mut all = Vec::new();
    for id in super::upgrades::lineage(client, room).await {
        all.extend(messages(client, &id).await?);
    }
    newest_first(&mut all);
    Ok(all)
}

/// A query ready to test messages against.
struct Test<'a> {
    query: &'a SearchQuery,
    /// The text, lowercased.
    phrase: String,
    senders: Option<HashSet<String>>,
}

impl Test<'_> {
    fn passes(&self, s: &Stored, words: bool) -> bool {
        let f = &s.found;
        (!words || f.body.to_lowercase().contains(&self.phrase))
            && self
                .senders
                .as_ref()
                .is_none_or(|senders| senders.contains(&f.sender))
            && self.query.has.iter().all(|h| h.matches(&s.kind, &f.body))
            && self.query.before.is_none_or(|t| f.time < t)
            && self.query.after.is_none_or(|t| f.time > t)
    }
    /// Older than anything the search wants.
    fn too_old(&self, time: i64) -> bool {
        self.query.after.is_some_and(|t| time <= t)
    }
    fn filter(&self) -> RoomEventFilter {
        let mut filter = RoomEventFilter::default();
        filter.types = Some(vec!["m.room.message".into(), "m.sticker".into()]);
        filter.senders = self.senders.as_ref().map(|senders| {
            senders
                .iter()
                .filter_map(|s| UserId::parse(s.as_str()).ok())
                .collect::<Vec<OwnedUserId>>()
        });
        if self.query.has.iter().any(|h| h.needs_url()) {
            filter.url_filter = Some(UrlFilter::EventsWithUrl);
        }
        filter
    }
}

/// Who `from:` names among the rooms' members: a full Matrix ID as it is,
/// anything else by part of a name or user ID. Also the values nobody
/// answers to.
async fn senders(client: &Client, query: &SearchQuery) -> (Option<HashSet<String>>, Vec<String>) {
    if query.from.is_empty() {
        return (None, Vec::new());
    }
    let mut members: Vec<(String, String)> = Vec::new();
    for room in query.rooms.iter().filter_map(|r| room(client, r)) {
        let memberships = RoomMemberships::JOIN | RoomMemberships::LEAVE | RoomMemberships::INVITE;
        if let Ok(list) = room.members(memberships).await {
            members.extend(list.into_iter().map(|m| {
                (
                    m.user_id().to_string(),
                    m.display_name().unwrap_or_default().to_lowercase(),
                )
            }));
        }
    }
    let mut found = HashSet::new();
    let mut nobody = Vec::new();
    for value in &query.from {
        if value.starts_with('@') && value.contains(':') {
            found.insert(value.clone());
            continue;
        }
        let wanted = value.trim_start_matches('@').to_lowercase();
        let named: Vec<&String> = members
            .iter()
            .filter(|(id, name)| {
                let local = id[1..].split(':').next().unwrap_or_default().to_lowercase();
                local.contains(&wanted) || name.contains(&wanted)
            })
            .map(|(id, _)| id)
            .collect();
        if named.is_empty() {
            nobody.push(value.clone());
        }
        found.extend(named.into_iter().cloned());
    }
    (Some(found), nobody)
}

fn room(client: &Client, id: &str) -> Option<Room> {
    client.get_room(&RoomId::parse(id).ok()?)
}

/// The homeserver's full-text search over `rooms`, newest first.
async fn server_words(
    client: &Client,
    rooms: &[OwnedRoomId],
    test: &Test<'_>,
) -> Result<(Vec<Stored>, bool), Failure> {
    let mut hits = Vec::new();
    let mut next = None;
    for _ in 0..PAGES {
        let mut criteria = Criteria::new(test.query.text.trim().to_owned());
        criteria.filter = test.filter();
        criteria.filter.rooms = Some(rooms.to_vec());
        criteria.order_by = Some(OrderBy::Recent);
        let mut categories = Categories::new();
        categories.room_events = Some(criteria);
        let mut request = Request::new(categories);
        request.next_batch = next.take();
        let response = client.send(request).await.map_err(|e| http(&e.into()))?;
        let events = response.search_categories.room_events;
        let mut oldest = None;
        for result in events.results {
            let Some(raw) = result.result else { continue };
            let Some(room) = raw.get_field::<String>("room_id").ok().flatten() else {
                continue;
            };
            let Ok(raw) = serde_json::from_str::<Raw<AnySyncTimelineEvent>>(raw.json().get())
            else {
                continue;
            };
            // The homeserver matched the words its own way; the rest is ours.
            if let Some(Parsed::Message(s)) = parse(&raw, &room) {
                oldest = Some(s.found.time);
                if test.passes(&s, false) {
                    hits.push(s);
                }
            }
        }
        next = events.next_batch;
        if next.is_none() || hits.len() > LIMIT || oldest.is_some_and(|t| test.too_old(t)) {
            break;
        }
    }
    Ok((hits, next.is_some()))
}

/// A room's history from the homeserver, newest first, filtered there by
/// sender and attachment: a search by filters alone.
async fn server_history(room: &Room, test: &Test<'_>) -> Result<(Vec<Stored>, bool), Failure> {
    let id = room.room_id().to_string();
    let mut parsed = Vec::new();
    let mut from = None;
    let mut matched = 0;
    let mut more = false;
    for _ in 0..PAGES {
        let mut options = MessagesOptions::backward();
        options.from = from.take();
        options.limit = UInt::from(100u32);
        options.filter = test.filter();
        let page = room.messages(options).await.map_err(|e| http(&e))?;
        let mut oldest = None;
        for event in &page.chunk {
            if let Some(p) = parse(event.raw(), &id) {
                if let Parsed::Message(s) = &p {
                    oldest = Some(s.found.time);
                    if test.passes(s, true) {
                        matched += 1;
                    }
                }
                parsed.push(p);
            }
        }
        from = page.end;
        more = from.is_some();
        if !more || matched > LIMIT || oldest.is_some_and(|t| test.too_old(t)) {
            break;
        }
    }
    let hits = assemble(parsed)
        .into_iter()
        .filter(|s| test.passes(s, true))
        .collect();
    Ok((hits, more))
}

/// Search `query.rooms`: unencrypted ones on the homeserver, encrypted ones
/// on this device.
pub async fn search_rooms(client: &Client, query: SearchQuery) -> Result<RoomsFound, Failure> {
    let (senders, nobody) = senders(client, &query).await;
    if senders.as_ref().is_some_and(HashSet::is_empty) {
        return Ok(RoomsFound {
            nobody,
            ..Default::default()
        });
    }
    let test = Test {
        query: &query,
        phrase: query.text.trim().to_lowercase(),
        senders,
    };
    let (mut plain, mut encrypted) = (Vec::new(), Vec::new());
    for id in &query.rooms {
        match room(client, id) {
            Some(r) if !r.encryption_state().is_encrypted() => plain.push(r),
            _ => encrypted.push(id.clone()),
        }
    }
    let mut found = Vec::new();
    let mut reach = Vec::new();
    let mut more = false;
    if !plain.is_empty() {
        if test.phrase.is_empty() {
            for r in &plain {
                let (hits, rest) = server_history(r, &test).await?;
                found.extend(hits);
                more |= rest;
            }
        } else {
            let ids: Vec<OwnedRoomId> = plain.iter().map(|r| r.room_id().to_owned()).collect();
            let (hits, rest) = server_words(client, &ids, &test).await?;
            found.extend(hits);
            more |= rest;
        }
        reach.extend(plain.iter().map(|r| Reach {
            room: r.room_id().to_string(),
            server: true,
            oldest: None,
        }));
    }
    for id in &encrypted {
        let all = history(client, id).await?;
        reach.push(Reach {
            room: id.clone(),
            server: false,
            oldest: all.last().map(|s| s.found.time),
        });
        found.extend(all.into_iter().filter(|s| test.passes(s, true)));
    }
    newest_first(&mut found);
    let mut seen = HashSet::new();
    let mut found: Vec<Found> = found
        .into_iter()
        .filter(|s| seen.insert(s.found.event_id.clone()))
        .map(|s| s.found)
        .collect();
    more |= found.len() > LIMIT;
    found.truncate(LIMIT);
    Ok(RoomsFound {
        found,
        reach,
        nobody,
        more,
    })
}

/// Load older history of the encrypted ones among `rooms` into this
/// device's event cache, so searches here reach further back.
pub async fn index(client: &Client, rooms: &[String]) -> Result<(), Failure> {
    let caches = client.event_cache().ok_or(Failure::Unavailable)?;
    for r in rooms.iter().filter_map(|id| room(client, id)) {
        if !r.encryption_state().is_encrypted() {
            continue;
        }
        let (cache, _handles) = caches
            .room(r.room_id())
            .await
            .map_err(|_| Failure::Unavailable)?;
        cache
            .pagination()
            .run_backwards_until(INDEX)
            .await
            .map_err(|_| Failure::Unreachable)?;
    }
    Ok(())
}

/// Messages from others that mention us (in their current `m.mentions`), in
/// these rooms, newest first.
pub async fn mentions(client: &Client, rooms: &[String]) -> Result<Vec<Found>, Failure> {
    let me = client
        .user_id()
        .map(ToString::to_string)
        .ok_or(Failure::Unavailable)?;
    let mut hits = Vec::new();
    for room in rooms {
        hits.extend(
            messages(client, room)
                .await?
                .into_iter()
                .filter(|s| s.found.sender != me && s.mentions.contains(&me))
                .map(|s| s.found),
        );
    }
    hits.sort_by_key(|f| std::cmp::Reverse(f.time));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(kind: &str, body: &str, sender: &str, time: i64) -> Stored {
        Stored {
            found: Found {
                room: "!r:h".into(),
                event_id: format!("${time}"),
                sender: sender.into(),
                body: body.into(),
                time,
            },
            kind: kind.into(),
            mentions: Vec::new(),
        }
    }

    #[test]
    fn filters_narrow_by_words_sender_attachment_and_time() {
        let query = SearchQuery {
            has: vec![Has::File],
            before: Some(100),
            after: Some(10),
            ..Default::default()
        };
        let test = Test {
            query: &query,
            phrase: "plan".into(),
            senders: Some(HashSet::from(["@mira:h".to_owned()])),
        };
        assert!(test.passes(&stored("m.file", "Plan.pdf", "@mira:h", 50), true));
        assert!(
            !test.passes(&stored("m.text", "plan", "@mira:h", 50), true),
            "no file"
        );
        assert!(
            !test.passes(&stored("m.file", "plan.pdf", "@bob:h", 50), true),
            "sender"
        );
        assert!(
            !test.passes(&stored("m.file", "plan.pdf", "@mira:h", 100), true),
            "before"
        );
        assert!(
            !test.passes(&stored("m.file", "plan.pdf", "@mira:h", 10), true),
            "after"
        );
        assert!(
            !test.passes(&stored("m.file", "notes.pdf", "@mira:h", 50), true),
            "words"
        );
        // The homeserver matched the words already.
        assert!(test.passes(&stored("m.file", "notes.pdf", "@mira:h", 50), false));
        assert!(test.too_old(10) && !test.too_old(11));
    }

    #[test]
    fn has_reads_attachments_and_links() {
        assert!(Has::File.matches("m.image", ""));
        assert!(Has::Image.matches("m.sticker", ""));
        assert!(!Has::Image.matches("m.file", ""));
        assert!(Has::Link.matches("m.text", "see https://example.org"));
        assert!(!Has::Link.matches("m.text", "no link"));
        assert!(!Has::Link.needs_url() && Has::Sound.needs_url());
    }
}
