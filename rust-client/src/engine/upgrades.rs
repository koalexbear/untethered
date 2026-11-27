//! Room upgrades. Upgrading a room (Element's "Upgrade room", or
//! `POST /rooms/{id}/upgrade`) makes a new room whose `m.room.create` names
//! its `predecessor`, and ends the old one with an `m.room.tombstone` that
//! names its successor. The App shows the newest room of such a chain as one
//! channel; here the engine finds the older rooms whose history continues
//! above a room's first message (`timeline.rs`, `search.rs`) and joins a
//! successor on request.
use super::{Failure, messaging};
use matrix_sdk::{
    Client, Room, RoomState,
    deserialized_responses::RawAnySyncOrStrippedState,
    ruma::{OwnedRoomId, OwnedServerName, RoomId, RoomOrAliasId, UserId, events::StateEventType},
};
use serde_json::Value;

/// Room versions a long-lived community has gone through stay well below
/// this; it only stops a chain that loops back on itself.
const LONGEST: usize = 32;

/// Where an upgraded room's history meets its predecessor's in a timeline
/// snapshot: a divider, not a separate room.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Seam {
    /// How many of the snapshot's messages come before it.
    pub index: usize,
    /// The room upgraded away from, whose messages are above.
    pub older: String,
    /// The room it was upgraded to.
    pub newer: String,
    /// When the newer room was created (ms since the epoch), if known.
    pub time: Option<i64>,
    /// This account can read the older room, so its history loads above.
    /// Otherwise older history isn't available.
    pub readable: bool,
}

/// What lies above a room's first message.
pub(super) enum Above {
    /// The start of history: the room wasn't upgraded from another.
    Start,
    /// An older room whose history continues above.
    Older(Room, Seam),
    /// An older room this account can't read.
    Unreadable(Seam),
}

/// The room `room` replaced, if this account is or was a member of it and
/// its tombstone points back here. Checking both directions keeps a room
/// from claiming another room's history by naming it as its predecessor.
pub(super) async fn above(client: &Client, room: &Room) -> Above {
    let Some(older) = room.predecessor_room().map(|p| p.room_id) else {
        return Above::Start;
    };
    let seam = Seam {
        index: 0,
        older: older.to_string(),
        newer: room.room_id().to_string(),
        time: created(room).await,
        readable: false,
    };
    match client
        .get_room(&older)
        .filter(|old| readable(old, room.room_id()))
    {
        Some(old) => Above::Older(
            old,
            Seam {
                readable: true,
                ..seam
            },
        ),
        None => Above::Unreadable(seam),
    }
}

/// Joined now, or left after reading it (a server returns history up to the
/// leave); a ban or an invite reads nothing. The tombstone must name `newer`.
fn readable(old: &Room, newer: &RoomId) -> bool {
    matches!(old.state(), RoomState::Joined | RoomState::Left)
        && old
            .successor_room()
            .is_some_and(|s| s.room_id.as_str() == newer.as_str())
}

/// `room` and the older rooms whose history it continues, newest first.
pub(super) async fn lineage(client: &Client, room: &str) -> Vec<String> {
    let mut rooms = vec![room.to_owned()];
    let Ok(mut current) = messaging::room(client, room) else {
        return rooms;
    };
    while rooms.len() < LONGEST {
        let Above::Older(old, _) = above(client, &current).await else {
            break;
        };
        let id = old.room_id().to_string();
        if rooms.contains(&id) {
            break;
        }
        rooms.push(id);
        current = old;
    }
    rooms
}

/// When the room was created: its `m.room.create` event's timestamp.
async fn created(room: &Room) -> Option<i64> {
    let RawAnySyncOrStrippedState::Sync(raw) = room
        .get_state_event(StateEventType::RoomCreate, "")
        .await
        .ok()??
    else {
        return None;
    };
    raw.get_field::<i64>("origin_server_ts").ok().flatten()
}

/// Join the room that replaced `room`. The tombstone doesn't say which
/// servers are in the new room; the one that upgraded it is (its sender's),
/// and so may the old room's own server and ours be.
pub async fn follow(client: &Client, room: &str) -> Result<(), Failure> {
    let old = messaging::room(client, room)?;
    let successor = old.successor_room().ok_or(Failure::Unavailable)?.room_id;
    if client
        .get_room(&successor)
        .is_some_and(|r| r.state() == RoomState::Joined)
    {
        return Ok(());
    }
    let via = servers(&old, &successor, client).await;
    client
        .join_room_by_id_or_alias(<&RoomOrAliasId>::from(&*successor), &via)
        .await
        .map(|_| ())
        .map_err(|e| messaging::http(&e))
}

async fn servers(old: &Room, successor: &OwnedRoomId, client: &Client) -> Vec<OwnedServerName> {
    let upgrader = match old.get_state_event(StateEventType::RoomTombstone, "").await {
        Ok(Some(RawAnySyncOrStrippedState::Sync(raw))) => {
            raw.deserialize_as::<Value>().ok().and_then(|e| {
                Some(
                    UserId::parse(e["sender"].as_str()?)
                        .ok()?
                        .server_name()
                        .to_owned(),
                )
            })
        }
        _ => None,
    };
    let mut via: Vec<OwnedServerName> = Vec::new();
    for server in [
        upgrader,
        successor.server_name().map(ToOwned::to_owned),
        old.room_id().server_name().map(ToOwned::to_owned),
        client.user_id().map(|u| u.server_name().to_owned()),
    ]
    .into_iter()
    .flatten()
    {
        if !via.contains(&server) {
            via.push(server);
        }
    }
    via
}
