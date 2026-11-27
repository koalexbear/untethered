//! Upgraded rooms as one channel, the way a Discord channel keeps a single
//! history. Upgrading a room (Element's "Upgrade room") makes a new room
//! whose `m.room.create` names its `predecessor`, and ends the old one with
//! an `m.room.tombstone` naming the successor.
//!
//! - Once the successor is joined, the old room leaves the room list, and so
//!   the server rail, the channel lists, DMs, unread counts, the tray badge
//!   and `#` suggestions. Space children, links and jumps that name it lead
//!   to the newest room instead.
//! - Until then the old room stays, read-only, offering to join the new one.
//! - The newest room's timeline continues into the older rooms' history
//!   above a divider (`engine::Seam`, engine/upgrades.rs). Those messages are
//!   read-only: replies, reactions and edits belong to the live room.
use crate::{App, Request, engine::RoomSummary, rooms::Rooms};
use std::collections::{BTreeMap, BTreeSet};

/// More upgrades than any room goes through; it only stops a loop.
const LONGEST: usize = 32;

/// Old rooms and the joined room that replaced each, from the joined rooms'
/// summaries. A joined room's own tombstone says where it went; it counts
/// once that room, or a room that in turn replaced it, is joined. An old room
/// this account isn't in is known only from its successor's `predecessor`,
/// which leads space children and links on but hides nothing.
pub fn replaced(rooms: &[RoomSummary]) -> BTreeMap<String, String> {
    let joined: BTreeSet<&str> = rooms.iter().map(|r| r.id.as_str()).collect();
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();
    for r in rooms {
        if let Some(old) = r
            .predecessor
            .as_ref()
            .filter(|old| !joined.contains(old.as_str()))
        {
            claimed.entry(old.clone()).or_insert_with(|| r.id.clone());
        }
    }
    let mut replaced = claimed.clone();
    for r in rooms {
        let mut next = r.successor.as_deref();
        for _ in 0..LONGEST {
            match next {
                Some(id) if id == r.id => break,
                Some(id) if joined.contains(id) => {
                    replaced.insert(r.id.clone(), id.to_owned());
                    break;
                }
                Some(id) => next = claimed.get(id).map(String::as_str),
                None => break,
            }
        }
    }
    replaced
}

impl Rooms {
    /// The newest joined room of `id`'s upgrade chain: `id` itself unless it
    /// was replaced by a room this account has joined.
    pub fn newest<'a>(&'a self, id: &'a str) -> &'a str {
        let mut current = id;
        for _ in 0..LONGEST {
            match self.replaced.get(current) {
                Some(next) if next != id => current = next,
                _ => break,
            }
        }
        current
    }
}

/// The room and event a Matrix permalink names: `https://matrix.to/#/!room/$event`
/// or `matrix:roomid/room/e/event`. Only room IDs; an alias needs a lookup.
pub fn permalink(link: &str) -> Option<(String, Option<String>)> {
    let (room, event) = if let Some(rest) = link
        .strip_prefix("https://matrix.to/#/")
        .or_else(|| link.strip_prefix("http://matrix.to/#/"))
    {
        let path = rest.split('?').next().unwrap_or_default();
        let mut parts = path.split('/').map(decode);
        (parts.next()?, parts.next())
    } else {
        let rest = link.strip_prefix("matrix:roomid/")?;
        let path = rest.split('?').next().unwrap_or_default();
        let mut parts = path.split('/').map(decode);
        let room = format!("!{}", parts.next()?);
        let event = match (parts.next().as_deref(), parts.next()) {
            (Some("e"), Some(event)) => Some(format!("${event}")),
            _ => None,
        };
        (room, event)
    };
    let room = room.starts_with('!').then_some(room)?;
    Some((room, event.filter(|e| e.starts_with('$') && e.len() > 1)))
}

/// `%XX` escapes, which matrix.to links use for `!`, `$`, `:` and `#`.
fn decode(part: &str) -> String {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&h), Some(&l)) if hex(h).is_some() && hex(l).is_some() => {
                out.push((hex(h).unwrap() * 16 + hex(l).unwrap()) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl App {
    /// Join the room that replaced `room`; the next room list hides `room`
    /// and the open channel moves to the new one (`reconcile`).
    pub fn follow_upgrade(&mut self, room: String) {
        if !self.logged_in
            || !self.verified
            || self.following(&room)
            || !self
                .live
                .list
                .get(&room)
                .is_some_and(|r| r.successor.is_some())
        {
            return;
        }
        let target = room.clone();
        self.command(
            |id| crate::engine::Command::FollowUpgrade { id, room: target },
            Request::FollowUpgrade(room),
        );
    }
    /// A request to join `room`'s successor is in flight.
    pub fn following(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::FollowUpgrade(r) if r == room))
    }
    pub fn followed(&mut self, ok: bool) {
        if !ok {
            self.status = "Couldn’t join the new room. Check your connection and try again.".into();
        }
    }
    /// A Matrix link to a joined room (or one a joined room replaced) opens
    /// here, at its message if it names one; other links open in the browser.
    pub fn open_permalink(&mut self, link: &str) -> bool {
        let Some((room, event)) = permalink(link) else {
            return false;
        };
        let newest = self.live.newest(&room).to_owned();
        if !self.logged_in || !self.live.list.get(&newest).is_some_and(|r| !r.space) {
            return false;
        }
        self.account = false;
        self.close_mentions();
        self.clear_search();
        if !self.visible().any(|(id, _)| *id == newest) {
            self.section = crate::Section::All;
        }
        self.open(newest);
        if let Some(event) = event {
            self.jump(event);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RoomList;

    fn room(id: &str, successor: Option<&str>, predecessor: Option<&str>) -> RoomSummary {
        RoomSummary {
            id: id.into(),
            name: id.trim_start_matches('!').into(),
            successor: successor.map(Into::into),
            predecessor: predecessor.map(Into::into),
            ..Default::default()
        }
    }

    fn list(rooms: Vec<RoomSummary>, children: &[(&str, &[&str])]) -> RoomList {
        RoomList {
            invites: Vec::new(),
            rooms,
            children: children
                .iter()
                .map(|(space, kids)| {
                    (
                        space.to_string(),
                        kids.iter().map(|k| k.to_string()).collect(),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn a_chain_of_upgrades_shows_only_its_newest_room() {
        let mut rooms = Rooms::default();
        let left = rooms.apply_engine(list(
            vec![
                room("!a", Some("!b"), None),
                room("!b", Some("!c"), Some("!a")),
                room("!c", None, Some("!b")),
                room("!other", None, None),
            ],
            &[],
        ));
        assert!(left.is_empty());
        assert_eq!(rooms.ordered_rooms, ["!c", "!other"]);
        assert!(!rooms.list.contains_key("!a") && !rooms.list.contains_key("!b"));
        assert_eq!(rooms.newest("!a"), "!c");
        assert_eq!(rooms.newest("!b"), "!c");
        assert_eq!(rooms.newest("!c"), "!c");
        assert_eq!(rooms.newest("!other"), "!other");
        assert_eq!(rooms.newest("!unknown"), "!unknown");
    }

    #[test]
    fn an_unjoined_successor_keeps_the_old_room_with_its_offer() {
        let mut rooms = Rooms::default();
        rooms.apply_engine(list(
            vec![
                room("!a", Some("!b"), None),
                room("!b", Some("!c"), Some("!a")),
            ],
            &[],
        ));
        // !b's successor isn't joined: !b stays, and offers to join !c.
        assert_eq!(rooms.ordered_rooms, ["!b"]);
        assert_eq!(rooms.list["!b"].successor.as_deref(), Some("!c"));
        assert_eq!(rooms.newest("!a"), "!b");
        // Joining !c hides !b, which leaves the list like a room left.
        let left = rooms.apply_engine(list(
            vec![
                room("!a", Some("!b"), None),
                room("!b", Some("!c"), Some("!a")),
                room("!c", None, Some("!b")),
            ],
            &[],
        ));
        assert_eq!(left, ["!b"]);
        assert_eq!(rooms.ordered_rooms, ["!c"]);
        assert_eq!(rooms.list["!c"].successor, None);
    }

    #[test]
    fn a_skipped_version_still_leads_to_the_joined_one() {
        // Joined !a and !c; !b, between them, never was.
        let rooms = [room("!a", Some("!b"), None), room("!c", None, Some("!b"))];
        let replaced = replaced(&rooms);
        assert_eq!(replaced["!a"], "!c");
        assert_eq!(replaced["!b"], "!c");
    }

    #[test]
    fn a_predecessor_claim_never_hides_a_joined_room() {
        // !x names !a as its predecessor, but !a's tombstone doesn't point to
        // !x (or anywhere): !a stays.
        let mut rooms = Rooms::default();
        rooms.apply_engine(list(
            vec![room("!a", None, None), room("!x", None, Some("!a"))],
            &[],
        ));
        assert_eq!(rooms.ordered_rooms, ["!a", "!x"]);
        assert_eq!(rooms.newest("!a"), "!a");
        // A tombstone pointing at itself is no upgrade.
        assert!(replaced(&[room("!s", Some("!s"), None)]).is_empty());
    }

    #[test]
    fn unread_state_counts_only_the_newest_room() {
        let mut rooms = Rooms::default();
        rooms.apply_engine(list(
            vec![
                RoomSummary {
                    unread_messages: 4,
                    highlights: 2,
                    ..room("!a", Some("!b"), None)
                },
                room("!b", None, Some("!a")),
            ],
            &[],
        ));
        let badge = crate::tray::Badge::of(rooms.list.values());
        assert_eq!(badge, crate::tray::Badge::None);
        assert!(rooms.list.values().all(|r| !r.unread));
    }

    #[test]
    fn space_children_resolve_to_the_newest_room() {
        let mut rooms = Rooms::default();
        rooms.apply_engine(list(
            vec![
                RoomSummary {
                    space: true,
                    ..room("!space", None, None)
                },
                // A joined old child, upgraded twice.
                room("!a", Some("!b"), None),
                room("!b", Some("!c"), Some("!a")),
                room("!c", None, Some("!b")),
                // A child this account left before its upgrade.
                room("!new-d", None, Some("!d")),
                // Listed both as itself and as its predecessor: once.
                room("!f", None, Some("!e")),
            ],
            &[("!space", &["!a", "!d", "!e", "!f"])],
        ));
        assert_eq!(rooms.ordered_spaces["!space"], ["!c", "!f", "!new-d"]);
        assert_eq!(
            rooms.channels("!space").into_keys().collect::<Vec<_>>(),
            ["!c", "!f", "!new-d"]
        );
    }

    #[test]
    fn upgraded_spaces_leave_the_rail() {
        let mut rooms = Rooms::default();
        rooms.apply_engine(list(
            vec![
                RoomSummary {
                    space: true,
                    ..room("!old-space", Some("!space"), None)
                },
                RoomSummary {
                    space: true,
                    ..room("!space", None, Some("!old-space"))
                },
                room("!general", None, None),
            ],
            &[("!old-space", &["!general"]), ("!space", &["!general"])],
        ));
        let spaces: Vec<_> = rooms
            .list
            .iter()
            .filter(|(_, r)| r.space)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(spaces, ["!space"]);
        assert_eq!(rooms.ordered_spaces["!space"], ["!general"]);
    }

    #[test]
    fn permalinks_name_rooms_and_events() {
        assert_eq!(
            permalink("https://matrix.to/#/!abc:example.org/$ev1?via=example.org"),
            Some(("!abc:example.org".into(), Some("$ev1".into())))
        );
        assert_eq!(
            permalink("https://matrix.to/#/%21abc%3Aexample.org/%24ev1"),
            Some(("!abc:example.org".into(), Some("$ev1".into())))
        );
        assert_eq!(
            permalink("https://matrix.to/#/!abc:example.org?via=x.org"),
            Some(("!abc:example.org".into(), None))
        );
        assert_eq!(
            permalink("matrix:roomid/abc:example.org/e/ev1?via=example.org"),
            Some(("!abc:example.org".into(), Some("$ev1".into())))
        );
        assert_eq!(permalink("https://matrix.to/#/@mira:example.org"), None);
        assert_eq!(permalink("https://matrix.to/#/#general:example.org"), None);
        assert_eq!(permalink("https://example.org/#/!abc:example.org"), None);
    }
}
