//! Joined rooms and space edges from the SDK's store, re-read whenever a sync
//! response or room-info update arrives. Simplified Sliding Sync's default
//! `required_state` includes `m.space.child`, so space edges are local reads.
//!
//! The store can keep a room this account left elsewhere as joined: sliding
//! sync mentions a left room once, relative to the position the client syncs
//! from, so a fresh connection never does (the homeserver expired the old
//! one: Synapse does after a week unused, or an hour once more than 100 rooms
//! changed meanwhile).
//! And Synapse 1.166 can send a newly left room with the old `join` as the
//! account's member state, which is all the SDK reads. The room list leaves
//! out the rooms the homeserver says this account isn't in
//! ([`left_elsewhere`]).
use super::{ChildEdge, Event, Failure, RoomList, RoomSummary, messaging};
use crate::permissions::StateDoc;
use async_channel::Sender;
use futures_util::{FutureExt, Stream, StreamExt, future::FusedFuture};
use matrix_sdk::{
    Client, Room,
    config::RequestConfig,
    deserialized_responses::{RawAnySyncOrStrippedState, SyncOrStrippedState},
    ruma::{
        OwnedRoomId,
        api::client::membership::joined_rooms,
        events::{
            RoomAccountDataEventType, StateEventType, SyncStateEvent,
            fully_read::FullyReadEventContent, space::child::SpaceChildEventContent,
        },
    },
    sync::RoomUpdates,
};
use matrix_sdk_ui::sync_service::State as SyncState;
use serde_json::Value;

/// `notifications::spaces::PREF`; that module is Linux and web only.
const SPACE_ALERTS: &str = "com.untethered.space_alerts.v1";
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Send the room list now and after every change. `sync` is the sync
/// service's state: each time it's running again, the homeserver is asked
/// which rooms this account is in, as it is when sync brings back a room
/// left out as left (joined again, perhaps). The list doesn't wait for the
/// answer.
pub async fn updates(
    client: Client,
    mut sync: impl Stream<Item = SyncState> + Unpin,
    events: Sender<Event>,
) {
    let mut rooms = client.subscribe_to_all_room_updates();
    let mut info = client.room_info_notable_update_receiver();
    let mut left = BTreeSet::new();
    let mut asking = std::pin::pin!(left_elsewhere(&client).fuse());
    let mut check = false;
    let mut syncing = true;
    loop {
        if check && asking.is_terminated() {
            asking.set(left_elsewhere(&client).fuse());
            check = false;
        }
        if events
            .send(Event::Rooms(snapshot(&client, &left).await))
            .await
            .is_err()
        {
            return;
        }
        tokio::select! {
            r = rooms.recv() => match r {
                Ok(update) => check |= back(&update, &left),
                // Updates were missed: any room may have come back.
                Err(RecvError::Lagged(_)) => check = true,
                Err(RecvError::Closed) => return,
            },
            r = info.recv() => if matches!(r, Err(RecvError::Closed)) {
                return;
            },
            state = sync.next(), if syncing => match state {
                Some(SyncState::Running) => check = true,
                Some(_) => {}
                None => syncing = false,
            },
            answer = asking.as_mut(), if !asking.is_terminated() => {
                if let Some(now) = answer {
                    left = now;
                }
            },
        }
        // Coalesce a burst (initial sync, many rooms) into one snapshot.
        super::rt::sleep(Duration::from_millis(80)).await;
        loop {
            match rooms.try_recv() {
                Ok(update) => check |= back(&update, &left),
                Err(TryRecvError::Lagged(_)) => check = true,
                Err(_) => break,
            }
        }
        while info.try_recv().is_ok() {}
    }
}

/// Sync brings a room left out as left: it may have been joined again.
fn back(update: &RoomUpdates, left: &BTreeSet<OwnedRoomId>) -> bool {
    update.joined.keys().any(|id| left.contains(id))
}

/// The rooms the store has as joined that the homeserver says this account
/// isn't in, or `None` when it can't be asked. The store's are read first,
/// so a room joined while the answer is on its way isn't taken for left.
async fn left_elsewhere(client: &Client) -> Option<BTreeSet<OwnedRoomId>> {
    let stored: Vec<OwnedRoomId> = client
        .joined_rooms()
        .iter()
        .map(|r| r.room_id().to_owned())
        .collect();
    if stored.is_empty() {
        return Some(BTreeSet::new());
    }
    let response = client
        .send(joined_rooms::v3::Request::new())
        .with_request_config(RequestConfig::short_retry())
        .await
        .ok()?;
    Some(not_joined(stored, &response.joined_rooms))
}

fn not_joined(stored: Vec<OwnedRoomId>, server: &[OwnedRoomId]) -> BTreeSet<OwnedRoomId> {
    let server: BTreeSet<_> = server.iter().collect();
    stored
        .into_iter()
        .filter(|id| !server.contains(id))
        .collect()
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

async fn snapshot(client: &Client, left: &BTreeSet<OwnedRoomId>) -> RoomList {
    let mut list = RoomList::default();
    list.invites = super::invites::list(client).await;
    for room in client.joined_rooms() {
        if left.contains(room.room_id()) {
            continue;
        }
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
        space_order: if room.is_space() {
            space_order(room).await
        } else {
            None
        },
        me: super::names::own(room).await,
        id,
    }
}

/// A space's MSC3230 `order` (room account data), when it's one the MSC
/// allows; Element ignores any other (ROADMAP N8, rail_order.rs).
async fn space_order(room: &Room) -> Option<String> {
    let raw = room
        .account_data(RoomAccountDataEventType::from(super::rail::SPACE_ORDER))
        .await
        .ok()??;
    let e: Value = raw.deserialize_as().ok()?;
    e["content"]["order"]
        .as_str()
        .filter(|o| crate::channel_order::valid(o))
        .map(str::to_owned)
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
    use super::{not_joined, pick_counts};
    use matrix_sdk::ruma::{OwnedRoomId, owned_room_id};

    #[test]
    fn rooms_the_homeserver_no_longer_lists_are_left() {
        let stored: Vec<OwnedRoomId> =
            vec![owned_room_id!("!alpha:test"), owned_room_id!("!bravo:test")];
        // The homeserver may list rooms the store doesn't know yet.
        let server = [owned_room_id!("!bravo:test"), owned_room_id!("!delta:test")];
        assert_eq!(
            not_joined(stored.clone(), &server),
            [owned_room_id!("!alpha:test")].into()
        );
        assert!(not_joined(stored.clone(), &[stored[0].clone(), stored[1].clone()]).is_empty());
    }

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
