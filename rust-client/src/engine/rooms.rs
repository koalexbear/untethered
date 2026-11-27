//! Joined rooms and space edges from the SDK's store, re-read whenever a sync
//! response or room-info update arrives. Simplified Sliding Sync's default
//! `required_state` includes `m.space.child`, so space edges are local reads.
use super::{ChildEdge, Event, Failure, RoomList, RoomSummary, messaging};
use crate::permissions::StateDoc;
use async_channel::Sender;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::{RawAnySyncOrStrippedState, SyncOrStrippedState},
    ruma::events::{
        RoomAccountDataEventType, StateEventType, SyncStateEvent,
        fully_read::FullyReadEventContent, space::child::SpaceChildEventContent,
    },
};
use serde_json::Value;

/// `notifications::spaces::PREF`; that module is Linux and web only.
const SPACE_ALERTS: &str = "com.untethered.space_alerts.v1";
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

pub async fn updates(client: Client, events: Sender<Event>) {
    let mut rooms = client.subscribe_to_all_room_updates();
    let mut info = client.room_info_notable_update_receiver();
    loop {
        if events
            .send(Event::Rooms(snapshot(&client).await))
            .await
            .is_err()
        {
            return;
        }
        let closed = tokio::select! {
            r = rooms.recv() => matches!(r, Err(RecvError::Closed)),
            r = info.recv() => matches!(r, Err(RecvError::Closed)),
        };
        if closed {
            return;
        }
        // Coalesce a burst (initial sync, many rooms) into one snapshot.
        super::rt::sleep(Duration::from_millis(80)).await;
        while rooms.try_recv().is_ok() {}
        while info.try_recv().is_ok() {}
    }
}

/// Mark a room read at the newest event sync delivered to its event cache:
/// the App knows a room's newest event only once its timeline is open.
pub async fn mark_latest_read(client: &Client, room: &str, public: bool) -> Result<(), Failure> {
    let joined = messaging::room(client, room)?;
    let caches = client.event_cache().ok_or(Failure::Unavailable)?;
    let (cache, _handles) = caches
        .room(joined.room_id())
        .await
        .map_err(|_| Failure::Unavailable)?;
    let events = cache.events().await.map_err(|_| Failure::Unavailable)?;
    let latest = events
        .iter()
        .rev()
        .find_map(|e| e.event_id())
        .ok_or(Failure::Unavailable)?;
    messaging::mark_read(client, room, latest.as_str(), public).await
}

/// Leave a joined room (or decline an invite).
pub async fn leave(client: &Client, room: &str) -> Result<(), Failure> {
    messaging::room(client, room)?
        .leave()
        .await
        .map_err(|e| messaging::http(&e))
}

async fn snapshot(client: &Client) -> RoomList {
    let mut list = RoomList::default();
    list.invites = super::invites::list(client).await;
    for room in client.joined_rooms() {
        let mut summary = summary(&room).await;
        if summary.space {
            summary.child_order = children(&room).await;
            list.children.insert(
                summary.id.clone(),
                summary
                    .child_order
                    .iter()
                    .map(|e| e.child.clone())
                    .collect(),
            );
        }
        list.rooms.push(summary);
    }
    list
}

/// A room's notification and highlight counts. The homeserver's can be
/// wrong in encrypted rooms, and some homeservers don't reset them for a
/// private read receipt, so a room read here stayed marked unread. The
/// SDK's own, from read receipts, are right once it knows where this
/// account read up to; before that (right after signing in, say) only the
/// homeserver's say anything.
fn alert_counts(room: &Room) -> (u64, u64) {
    let receipts = room.read_receipts();
    let server = room.unread_notification_counts();
    pick_counts(
        receipts.latest_active.is_some(),
        (receipts.num_notifications, receipts.num_mentions),
        (server.notification_count, server.highlight_count),
    )
}

fn pick_counts(known: bool, client: (u64, u64), server: (u64, u64)) -> (u64, u64) {
    if known { client } else { server }
}

async fn summary(room: &Room) -> RoomSummary {
    let id = room.room_id().to_string();
    let dm = room.is_direct().await.unwrap_or(false);
    let (notifications, highlights) = alert_counts(room);
    let voice = room.is_call();
    // A group DM has no one peer.
    let targets = room.direct_targets();
    let peer = (dm && targets.len() == 1)
        .then(|| {
            targets
                .iter()
                .find_map(|t| t.as_user_id().map(ToOwned::to_owned))
        })
        .flatten();
    let avatar = match (room.avatar_url(), &peer) {
        (Some(url), _) => Some(url.to_string()),
        (None, Some(peer)) => room
            .get_member_no_sync(peer)
            .await
            .ok()
            .flatten()
            .and_then(|m| m.avatar_url().map(ToString::to_string)),
        (None, None) => None,
    };
    RoomSummary {
        avatar,
        name: room
            .cached_display_name()
            .map(|n| n.to_string())
            .or_else(|| room.name())
            .or_else(|| room.canonical_alias().map(|a| a.to_string()))
            .unwrap_or_else(|| id.clone()),
        topic: room.topic().unwrap_or_default(),
        space: room.is_space(),
        dm,
        dm_peer: peer.map(|p| p.to_string()),
        voice,
        call_members: if voice {
            call_members(room).await
        } else {
            Vec::new()
        },
        encrypted: room.encryption_state().is_encrypted(),
        unread_messages: room.num_unread_messages(),
        unread_mentions: room.num_unread_mentions(),
        notifications,
        highlights,
        marked_unread: room.is_marked_unread(),
        fully_read: room
            .account_data_static::<FullyReadEventContent>()
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.deserialize().ok())
            .map(|e| e.content.event_id.to_string()),
        link_previews: super::previews::enabled(room).await,
        create: state_doc(room, StateEventType::RoomCreate).await,
        power: state_doc(room, StateEventType::RoomPowerLevels).await,
        space_alerts: room
            .account_data(RoomAccountDataEventType::from(SPACE_ALERTS))
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.deserialize_as::<Value>().ok())
            .map(|e| e["content"].clone()),
        successor: room.successor_room().map(|s| s.room_id.to_string()),
        predecessor: room.predecessor_room().map(|p| p.room_id.to_string()),
        // Filled in for spaces by `snapshot`.
        child_order: Vec::new(),
        id,
    }
}

/// A state event from the store as the App keeps it. Fields are read from
/// the JSON so power levels keep what ruma doesn't model.
pub(super) async fn state_doc(room: &Room, kind: StateEventType) -> Option<StateDoc> {
    let RawAnySyncOrStrippedState::Sync(raw) = room.get_state_event(kind, "").await.ok()?? else {
        return None;
    };
    let e: Value = raw.deserialize_as().ok()?;
    Some(StateDoc {
        event_id: e["event_id"].as_str()?.into(),
        sender: e["sender"].as_str().unwrap_or_default().into(),
        content: e["content"].clone(),
        redacted: e["unsigned"]["redacted_because"].is_object(),
    })
}

/// Who is in a voice room. Simplified Sliding Sync's default `required_state`
/// includes every `org.matrix.msc3401.call.member` event, so this is a local
/// read that follows sync; the App decides which memberships are still active.
async fn call_members(room: &Room) -> Vec<Value> {
    let Ok(events) = room.get_state_events(StateEventType::CallMember).await else {
        return Vec::new();
    };
    events
        .into_iter()
        .filter_map(|raw| match raw {
            RawAnySyncOrStrippedState::Sync(raw) => raw.deserialize_as::<Value>().ok(),
            RawAnySyncOrStrippedState::Stripped(_) => None,
        })
        .collect()
}

/// Children with a non-empty `via`, with what orders them among their
/// siblings (channel_order.rs); an empty or redacted edge is a removal.
async fn children(space: &Room) -> Vec<ChildEdge> {
    let Ok(edges) = space
        .get_state_events_static::<SpaceChildEventContent>()
        .await
    else {
        return Vec::new();
    };
    edges
        .into_iter()
        .filter_map(|raw| match raw.deserialize().ok()? {
            SyncOrStrippedState::Sync(SyncStateEvent::Original(e)) if !e.content.via.is_empty() => {
                Some(ChildEdge {
                    child: e.state_key.to_string(),
                    order: e.content.order.map(|o| o.to_string()),
                    ts: i64::from(e.origin_server_ts.0),
                })
            }
            SyncOrStrippedState::Stripped(e)
                if e.content.via.as_ref().is_some_and(|v| !v.is_empty()) =>
            {
                Some(ChildEdge {
                    child: e.state_key.to_string(),
                    ..ChildEdge::default()
                })
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::pick_counts;

    #[test]
    fn a_read_room_stays_read_whatever_the_homeserver_still_counts() {
        // Read here, privately: the homeserver still counts two highlights.
        assert_eq!(pick_counts(true, (0, 0), (2, 2)), (0, 0));
        // New messages after the receipt count as the SDK sees them.
        assert_eq!(pick_counts(true, (3, 1), (3, 0)), (3, 1));
        // Before the SDK knows where this account read up to.
        assert_eq!(pick_counts(false, (0, 0), (5, 1)), (5, 1));
    }
}
