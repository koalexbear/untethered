//! Room search and the mentions inbox over what this device has stored: the
//! event cache keeps every event it synced or paginated, decrypted. Like
//! gomuks's local search, older history is found once it has been loaded.
use super::{Failure, Found};
use matrix_sdk::cross_process_lock::MappedCrossProcessLockState;
use matrix_sdk::{
    Client,
    ruma::{
        OwnedEventId, RoomId,
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
            room::message::Relation,
        },
    },
};
use std::collections::HashMap;

/// One stored message with its current text and mentions.
struct Stored {
    found: Found,
    mentions: Vec<String>,
}

/// A room's stored messages and stickers, newest first, with edits applied
/// (from the original sender only) and redacted or undecryptable ones left
/// out.
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
    let mut originals: HashMap<OwnedEventId, Stored> = HashMap::new();
    // (target, sender, time, text, mentions)
    let mut edits = Vec::new();
    for event in events {
        let Ok(any) = event.raw().deserialize() else {
            continue;
        };
        let (event_id, sender, time) = (
            any.event_id().to_owned(),
            any.sender().to_string(),
            u64::from(any.origin_server_ts().0) as i64,
        );
        let (body, mentions) = match any {
            AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
                SyncMessageLikeEvent::Original(m),
            )) => {
                if let Some(Relation::Replacement(r)) = &m.content.relates_to {
                    let mentions = r
                        .new_content
                        .mentions
                        .as_ref()
                        .map(|m| m.user_ids.iter().map(ToString::to_string).collect())
                        .unwrap_or_default();
                    edits.push((
                        r.event_id.clone(),
                        sender,
                        time,
                        r.new_content.msgtype.body().to_owned(),
                        mentions,
                    ));
                    continue;
                }
                let mentions = m
                    .content
                    .mentions
                    .as_ref()
                    .map(|m| m.user_ids.iter().map(ToString::to_string).collect())
                    .unwrap_or_default();
                (m.content.body().to_owned(), mentions)
            }
            AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Sticker(
                SyncMessageLikeEvent::Original(s),
            )) => (s.content.body, Vec::new()),
            _ => continue,
        };
        if body.is_empty() {
            continue;
        }
        let found = Found {
            room: room.to_owned(),
            event_id: event_id.to_string(),
            sender,
            body,
            time,
        };
        originals.insert(event_id, Stored { found, mentions });
    }
    edits.sort_by_key(|(_, _, time, ..)| *time);
    for (target, sender, _, body, mentions) in edits {
        if let Some(original) = originals.get_mut(&target)
            && original.found.sender == sender
        {
            original.found.body = body;
            original.mentions = mentions;
        }
    }
    let mut all: Vec<Stored> = originals.into_values().collect();
    all.sort_by(|a, b| {
        b.found
            .time
            .cmp(&a.found.time)
            .then_with(|| a.found.event_id.cmp(&b.found.event_id))
    });
    Ok(all)
}

const PAGE: usize = 32;

/// Messages whose current text contains `term` (case-insensitive), newest
/// first, a page at a time. `before` is the `next` of the previous page.
pub async fn search(
    client: &Client,
    room: &str,
    term: &str,
    before: Option<String>,
) -> Result<(Vec<Found>, Option<String>), Failure> {
    let term = term.to_lowercase();
    let mut hits = messages(client, room)
        .await?
        .into_iter()
        .map(|s| s.found)
        .filter(|f| f.body.to_lowercase().contains(&term));
    let mut page: Vec<Found> = match &before {
        Some(cursor) => hits
            .by_ref()
            .skip_while(|f| &cursor_of(f) != cursor)
            .skip(1)
            .take(PAGE + 1)
            .collect(),
        None => hits.take(PAGE + 1).collect(),
    };
    let next = (page.len() > PAGE).then(|| {
        page.truncate(PAGE);
        cursor_of(page.last().unwrap())
    });
    Ok((page, next))
}

fn cursor_of(found: &Found) -> String {
    format!("{}:{}", found.time, found.event_id)
}

/// How many messages of a room are stored, and the oldest one's time.
pub async fn coverage(client: &Client, room: &str) -> Result<(usize, Option<i64>), Failure> {
    let all = messages(client, room).await?;
    Ok((all.len(), all.last().map(|s| s.found.time)))
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
