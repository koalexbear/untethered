//! Joining by link or address (ROADMAP J2): `matrix.to` links, `matrix:`
//! links and `#room:server` addresses open in the app. A room you're in
//! opens; any other shows its preview (engine/joining.rs) with Join, which
//! joins and opens it. Join a Server in the rail's **+** takes the same.
use crate::{
    App, Request,
    engine::{Command, Failure, RoomPreview},
};

/// A room a link or address points to, with the servers to reach it
/// through and the message it points at, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomLink {
    /// `#alias:server` or `!id:server`.
    pub target: String,
    pub via: Vec<String>,
    pub event: Option<String>,
}

pub enum PreviewState {
    Loading,
    Ready(RoomPreview),
    Failed,
}

pub const REFUSED: &str = "Couldn't join. The room may be invite-only, or the link may be wrong.";

/// The main area's room preview.
pub struct Preview {
    pub link: RoomLink,
    pub state: PreviewState,
    /// Joining it failed.
    pub refused: bool,
}

impl Preview {
    /// The room or address Join joins: by ID once the preview has one.
    pub fn join_target(&self) -> &str {
        match &self.state {
            PreviewState::Ready(p) => &p.room_id,
            _ => &self.link.target,
        }
    }

    fn joins(&self, target: &str) -> bool {
        self.join_target() == target || self.link.target == target
    }
}

/// The room in a `matrix.to` or `matrix:` link, or a bare address.
pub fn room_link(text: &str) -> Option<RoomLink> {
    let text = text.trim().trim_start_matches('<').trim_end_matches('>');
    let (path, query) = text.split_once('?').unwrap_or((text, ""));
    let via = query
        .split('&')
        .filter_map(|p| p.strip_prefix("via="))
        .map(crate::upgrades::decode)
        .filter(|s| !s.is_empty())
        .collect();
    let (target, event) = if let Some(rest) = path
        .strip_prefix("https://matrix.to/#/")
        .or_else(|| path.strip_prefix("http://matrix.to/#/"))
    {
        let mut parts = rest.split('/').map(crate::upgrades::decode);
        (parts.next()?, parts.next())
    } else if let Some(rest) = path.strip_prefix("matrix:r/") {
        (format!("#{}", crate::upgrades::decode(rest)), None)
    } else if let Some(rest) = path.strip_prefix("matrix:roomid/") {
        let mut parts = rest.split('/').map(crate::upgrades::decode);
        let room = format!("!{}", parts.next()?);
        let event = match (parts.next().as_deref(), parts.next()) {
            (Some("e"), Some(event)) => Some(format!("${event}")),
            _ => None,
        };
        (room, event)
    } else {
        (path.to_owned(), None)
    };
    // An address names its server; a room ID needn't (room version 12).
    let named = match (target.strip_prefix('#'), target.strip_prefix('!')) {
        (Some(alias), _) => alias
            .split_once(':')
            .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty()),
        (_, Some(id)) => !id.is_empty() && !id.starts_with(':') && !id.ends_with(':'),
        _ => false,
    };
    let valid = named && !target.chars().any(|c| c.is_whitespace() || c.is_control());
    valid.then(|| RoomLink {
        target,
        via,
        event: event.filter(|e| e.starts_with('$') && e.len() > 1),
    })
}

impl App {
    /// Open a link to a room in the app; false when it isn't one.
    pub fn open_room_link(&mut self, url: &str) -> bool {
        let Some(link) = room_link(url).filter(|_| self.logged_in && self.verified) else {
            return false;
        };
        self.show_preview(link);
        true
    }

    fn show_preview(&mut self, link: RoomLink) {
        if let Some(sdk) = &self.sdk {
            let _ = sdk.try_send(Command::PreviewRoom {
                target: link.target.clone(),
                via: link.via.clone(),
            });
        }
        self.show(link, PreviewState::Loading);
    }

    /// Preview a room Explore or a space's hierarchy lists, with the
    /// details they gave (explore.rs).
    pub fn preview_listed(&mut self, link: RoomLink, preview: RoomPreview) {
        self.show(link, PreviewState::Ready(preview));
    }

    fn show(&mut self, link: RoomLink, state: PreviewState) {
        self.close_mentions();
        self.account = false;
        self.invite_open = None;
        self.room_preview = Some(Preview {
            link,
            state,
            refused: false,
        });
    }

    pub fn room_previewed(&mut self, target: String, result: Result<RoomPreview, Failure>) {
        let Some(preview) = self
            .room_preview
            .as_mut()
            .filter(|p| p.link.target == target)
        else {
            return;
        };
        match result {
            // Already in it: open it, at the message linked to, once the
            // room list has it (invites.rs).
            Ok(p) if p.joined => {
                self.joined_event = preview.link.event.clone();
                self.room_preview = None;
                self.joined_invite = Some(p.room_id);
                self.open_joined_invite();
            }
            Ok(p) => preview.state = PreviewState::Ready(p),
            Err(_) => preview.state = PreviewState::Failed,
        }
    }

    pub fn close_preview(&mut self) {
        self.room_preview = None;
    }

    /// Join the previewed room: by ID once the preview has one, else by the
    /// link's address.
    pub fn join_previewed(&mut self) {
        let Some(preview) = &mut self.room_preview else {
            return;
        };
        preview.refused = false;
        let (target, via) = (preview.join_target().to_owned(), preview.link.via.clone());
        if self.joining(&target) {
            return;
        }
        let room = target.clone();
        self.command(
            |id| Command::JoinRoom {
                id,
                target: room,
                via,
            },
            Request::JoinRoom(target),
        );
    }

    pub fn joining(&self, target: &str) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::JoinRoom(t) if t == target))
    }

    pub fn room_joined(&mut self, target: String, ok: bool) {
        if !ok {
            // Said on the card when it's still open, else under the room.
            match self.room_preview.as_mut() {
                Some(p) if p.joins(&target) => p.refused = true,
                _ => self.status = REFUSED.into(),
            }
            return;
        }
        self.status.clear();
        let Some(link) = self.room_preview.take().map(|p| p.link) else {
            return;
        };
        // Joined from Explore: the room or space opens instead.
        self.explore.page = None;
        if target.starts_with('!') {
            // It opens once the room list has it (invites.rs).
            self.joined_invite = Some(target);
            self.joined_event = link.event;
            self.open_joined_invite();
        } else {
            // Joined by address: its preview now names the room it opens.
            self.show_preview(link);
        }
    }

    /// Join a Server: the link or address typed into the create window.
    pub fn join_address(&mut self) {
        match room_link(&self.join_address) {
            Some(link) => {
                self.join_address.clear();
                self.setup = None;
                self.show_preview(link);
            }
            None => {
                if let Some(f) = &mut self.setup {
                    f.note = "That isn't a Matrix room link or address. Try one like \
                        #room:example.org."
                        .into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_addresses_name_their_room() {
        assert_eq!(
            room_link("https://matrix.to/#/%23lounge:example.org?via=example.org&via=other.org"),
            Some(RoomLink {
                target: "#lounge:example.org".into(),
                via: vec!["example.org".into(), "other.org".into()],
                event: None,
            })
        );
        assert_eq!(
            room_link("https://matrix.to/#/!abc:example.org/$ev?via=example.org")
                .map(|l| (l.target, l.event)),
            Some(("!abc:example.org".into(), Some("$ev".into())))
        );
        assert_eq!(
            room_link("matrix:r/lounge:example.org").map(|l| l.target),
            Some("#lounge:example.org".into())
        );
        assert_eq!(
            room_link("matrix:roomid/abc:example.org/e/ev").map(|l| (l.target, l.event)),
            Some(("!abc:example.org".into(), Some("$ev".into())))
        );
        assert_eq!(
            room_link("  #lounge:example.org ").map(|l| l.target),
            Some("#lounge:example.org".into())
        );
        // Room version 12's IDs have no server part.
        assert_eq!(
            room_link(
                "https://matrix.to/#/!LkNSuHm8ZJ3kuVSlhEFNNBxlmjIQEFJUAxcV7DNYdt4?via=example.org"
            )
            .map(|l| (l.target, l.via)),
            Some((
                "!LkNSuHm8ZJ3kuVSlhEFNNBxlmjIQEFJUAxcV7DNYdt4".into(),
                vec!["example.org".into()]
            ))
        );
    }

    #[test]
    fn people_pages_and_typos_are_no_rooms() {
        for text in [
            "https://matrix.to/#/@mira:example.org",
            "https://example.org/#/#room:x",
            "#lounge",
            "#:example.org",
            "lounge:example.org",
            "#lo unge:example.org",
        ] {
            assert_eq!(room_link(text), None, "{text}");
        }
    }
}
