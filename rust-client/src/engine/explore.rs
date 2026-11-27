//! Rooms this account can find and join (ROADMAP J4): the homeserver's
//! public room directory (`/publicRooms`), searched and filtered by room
//! type, for Explore; and what a space contains (`/hierarchy`), for the
//! channels of a joined space that this account hasn't joined yet. Both are
//! spec: the directory since v1.0, its room type filter since v1.4, the
//! hierarchy since v1.2. A homeserver that ignores the room type filter
//! still gets only the kind asked for: the answer is filtered here too.
use super::{Failure, JoinRule, messaging::http};
use matrix_sdk::{
    Client,
    ruma::{
        OwnedRoomId, RoomAliasId, RoomId, UInt,
        api::client::{directory::get_public_rooms_filtered, space::get_hierarchy},
        directory::{PublicRoomsChunk, RoomTypeFilter},
        room::{JoinRuleKind, JoinRuleSummary, RoomSummary, RoomType},
    },
};
use std::collections::BTreeMap;

/// Rooms one page of the directory holds.
const PAGE: u32 = 30;
/// Rooms one page of a space's hierarchy holds, and the pages read: a
/// space with more lists its first few hundred rooms.
const HIERARCHY: u32 = 100;
const HIERARCHY_PAGES: usize = 3;
/// MSC3417 voice rooms, which Element calls video rooms.
const CALL: &str = "org.matrix.msc3417.call";

/// What Explore lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    #[default]
    All,
    Spaces,
    Rooms,
}

impl Kind {
    /// The directory's room type filter for this kind: `None` is a room
    /// without a type; voice rooms count as rooms.
    fn filter(self) -> Vec<RoomTypeFilter> {
        match self {
            Kind::All => Vec::new(),
            Kind::Spaces => vec![RoomTypeFilter::Space],
            Kind::Rooms => vec![RoomTypeFilter::Default, RoomTypeFilter::from(Some(CALL))],
        }
    }

    pub fn admits(self, room: &PublicRoom) -> bool {
        match self {
            Kind::All => true,
            Kind::Spaces => room.space,
            Kind::Rooms => !room.space,
        }
    }
}

/// A search of the directory: `term` (empty for everything) among `kind`,
/// from the page after `since` (a `next_batch`) or the first.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirectoryQuery {
    pub term: String,
    pub kind: Kind,
    pub since: Option<String>,
}

/// A room or space the directory lists, or one a space contains.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PublicRoom {
    pub room_id: String,
    pub alias: Option<String>,
    pub name: Option<String>,
    pub topic: Option<String>,
    /// Its picture (mxc URI).
    pub avatar: Option<String>,
    pub members: u64,
    pub space: bool,
    /// An MSC3417 voice room.
    pub voice: bool,
    pub join: JoinRule,
    /// Servers to join it through.
    pub via: Vec<String>,
}

impl PublicRoom {
    /// Its name, else its address, else its ID.
    pub fn title(&self) -> &str {
        self.name
            .as_deref()
            .or(self.alias.as_deref())
            .unwrap_or(&self.room_id)
    }

    fn new(chunk: Chunk, join: JoinRule, via: Vec<String>) -> Self {
        let Chunk {
            room_id,
            alias,
            name,
            topic,
            avatar,
            members,
            room_type,
        } = chunk;
        PublicRoom {
            room_id: room_id.to_string(),
            alias: alias.map(|a| a.to_string()),
            name: name.filter(|n| !n.trim().is_empty()),
            topic: topic.filter(|t| !t.trim().is_empty()),
            avatar,
            members,
            space: room_type == Some(RoomType::Space),
            voice: room_type.as_ref().is_some_and(|t| t.as_str() == CALL),
            join,
            via,
        }
    }
}

/// The fields the directory and the hierarchy share.
struct Chunk {
    room_id: OwnedRoomId,
    alias: Option<matrix_sdk::ruma::OwnedRoomAliasId>,
    name: Option<String>,
    topic: Option<String>,
    avatar: Option<String>,
    members: u64,
    room_type: Option<RoomType>,
}

/// One page of the directory.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Directory {
    pub rooms: Vec<PublicRoom>,
    /// The token for the next page, while there is one.
    pub next: Option<String>,
    /// The homeserver's estimate of how many rooms match.
    pub total: Option<u64>,
}

/// A room in a space's hierarchy, with the space that lists it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpaceChild {
    pub parent: String,
    pub room: PublicRoom,
}

/// The room's own server and its address's: the servers that surely know
/// a room the homeserver lists (room version 12's IDs name no server).
fn via(room: &RoomId, alias: Option<&RoomAliasId>) -> Vec<String> {
    let mut via: Vec<String> = room
        .server_name()
        .map(|s| s.to_string())
        .into_iter()
        .collect();
    if let Some(server) = alias.map(|a| a.server_name().to_string())
        && !via.contains(&server)
    {
        via.push(server);
    }
    via
}

fn join_rule(kind: &JoinRuleKind) -> JoinRule {
    match kind {
        JoinRuleKind::Public => JoinRule::Open,
        JoinRuleKind::Restricted => JoinRule::Members,
        JoinRuleKind::Knock | JoinRuleKind::KnockRestricted => JoinRule::Knock,
        _ => JoinRule::Invite,
    }
}

fn listed(chunk: PublicRoomsChunk) -> PublicRoom {
    let via = via(&chunk.room_id, chunk.canonical_alias.as_deref());
    let join = join_rule(&chunk.join_rule);
    PublicRoom::new(
        Chunk {
            room_id: chunk.room_id,
            alias: chunk.canonical_alias,
            name: chunk.name,
            topic: chunk.topic,
            avatar: chunk.avatar_url.map(|u| u.to_string()),
            members: chunk.num_joined_members.into(),
            room_type: chunk.room_type,
        },
        join,
        via,
    )
}

fn contained(summary: RoomSummary, via: Vec<String>) -> PublicRoom {
    let join = match &summary.join_rule {
        JoinRuleSummary::Public => JoinRule::Open,
        JoinRuleSummary::Restricted(_) => JoinRule::Members,
        JoinRuleSummary::Knock | JoinRuleSummary::KnockRestricted(_) => JoinRule::Knock,
        _ => JoinRule::Invite,
    };
    PublicRoom::new(
        Chunk {
            room_id: summary.room_id,
            alias: summary.canonical_alias,
            name: summary.name,
            topic: summary.topic,
            avatar: summary.avatar_url.map(|u| u.to_string()),
            members: summary.num_joined_members.into(),
            room_type: summary.room_type,
        },
        join,
        via,
    )
}

/// A page of the homeserver's public room directory.
pub async fn public_rooms(client: &Client, query: DirectoryQuery) -> Result<Directory, Failure> {
    let mut request = get_public_rooms_filtered::v3::Request::new();
    request.limit = Some(UInt::from(PAGE));
    request.since = query.since;
    let term = query.term.trim();
    request.filter.generic_search_term = (!term.is_empty()).then(|| term.to_owned());
    request.filter.room_types = query.kind.filter();
    let response = client
        .public_rooms_filtered(request)
        .await
        .map_err(|e| http(&e.into()))?;
    Ok(Directory {
        rooms: response
            .chunk
            .into_iter()
            .map(listed)
            .filter(|r| query.kind.admits(r))
            .collect(),
        next: response.next_batch.filter(|n| !n.is_empty()),
        total: response.total_room_count_estimate.map(u64::from),
    })
}

/// The rooms and spaces in `space`, at any depth, each with the space that
/// lists it and the servers its `m.space.child` event names. The homeserver
/// leaves out rooms this account may not see.
pub async fn space_rooms(client: &Client, space: &str) -> Result<Vec<SpaceChild>, Failure> {
    let root = RoomId::parse(space).map_err(|_| Failure::Unavailable)?;
    let mut chunks = Vec::new();
    let mut from = None;
    for _ in 0..HIERARCHY_PAGES {
        let mut request = get_hierarchy::v1::Request::new(root.clone());
        request.from = from.take();
        request.limit = Some(UInt::from(HIERARCHY));
        let response = client.send(request).await.map_err(|e| http(&e.into()))?;
        chunks.extend(response.rooms);
        match response.next_batch {
            Some(next) if !next.is_empty() => from = Some(next),
            _ => break,
        }
    }
    // Each space's `m.space.child` events name its children; an edge with
    // no servers is a removal.
    let mut parents: BTreeMap<OwnedRoomId, (OwnedRoomId, Vec<String>)> = BTreeMap::new();
    for chunk in &chunks {
        for edge in &chunk.children_state {
            let Ok(edge) = edge.deserialize() else {
                continue;
            };
            if edge.content.via.is_empty() {
                continue;
            }
            parents.entry(edge.state_key).or_insert_with(|| {
                (
                    chunk.summary.room_id.clone(),
                    edge.content.via.iter().map(ToString::to_string).collect(),
                )
            });
        }
    }
    Ok(chunks
        .into_iter()
        .filter(|c| c.summary.room_id != root)
        .filter_map(|c| {
            let (parent, via) = parents.remove(&c.summary.room_id)?;
            Some(SpaceChild {
                parent: parent.to_string(),
                room: contained(c.summary, via),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(space: bool) -> PublicRoom {
        PublicRoom {
            room_id: "!a:example.org".into(),
            alias: None,
            name: None,
            topic: None,
            avatar: None,
            members: 1,
            space,
            voice: false,
            join: JoinRule::Open,
            via: vec![],
        }
    }

    #[test]
    fn each_kind_admits_its_rooms() {
        assert!(Kind::All.admits(&room(true)) && Kind::All.admits(&room(false)));
        assert!(Kind::Spaces.admits(&room(true)) && !Kind::Spaces.admits(&room(false)));
        assert!(Kind::Rooms.admits(&room(false)) && !Kind::Rooms.admits(&room(true)));
        assert!(Kind::All.filter().is_empty());
        assert_eq!(Kind::Rooms.filter().len(), 2);
    }

    #[test]
    fn rooms_join_through_their_own_server_and_their_address() {
        let id = RoomId::parse("!a:one.org").unwrap();
        let alias = RoomAliasId::parse("#lounge:two.org").unwrap();
        assert_eq!(via(&id, Some(&alias)), ["one.org", "two.org"]);
        let same = RoomAliasId::parse("#lounge:one.org").unwrap();
        assert_eq!(via(&id, Some(&same)), ["one.org"]);
        // Room version 12: only the address names a server.
        let v12 = RoomId::parse("!LkNSuHm8ZJ3kuVSlhEFNNBxlmjIQEFJUAxcV7DNYdt4").unwrap();
        assert_eq!(via(&v12, Some(&alias)), ["two.org"]);
    }
}
