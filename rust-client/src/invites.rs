//! Invites (ROADMAP J1): rooms, spaces and DMs this account is invited to,
//! from the engine's room list (engine/invites.rs). A space's invite sits in
//! the server rail, a room's under Invites in All Rooms, a DM's at the top of
//! Direct Messages; opening one shows its card with Accept and Decline.
//! Accepting joins and opens the room once it's listed; declining leaves it.
use crate::{App, Request, Section, engine::Command, engine::Invite};

impl App {
    pub fn invite(&self, id: &str) -> Option<&Invite> {
        self.live.invites.iter().find(|i| i.id == id)
    }

    /// Invites the channel list shows in `section`: DMs in Direct Messages,
    /// rooms in All Rooms; spaces are in the rail.
    pub fn section_invites(&self, section: &Section) -> Vec<&Invite> {
        if !self.logged_in {
            return Vec::new();
        }
        self.live
            .invites
            .iter()
            .filter(|i| match section {
                Section::Direct => i.dm,
                Section::All => !i.dm && !i.space,
                Section::Space(_) => false,
            })
            .collect()
    }

    pub fn open_invite(&mut self, id: String) {
        if self.invite(&id).is_some() {
            self.close_mentions();
            self.invite_open = Some(id);
        }
    }

    pub fn accept_invite(&mut self, id: String) {
        if self.invite_busy(&id) {
            return;
        }
        let room = id.clone();
        self.command(
            |seq| Command::AcceptInvite { id: seq, room },
            Request::AcceptInvite(id),
        );
    }

    pub fn decline_invite(&mut self, id: String) {
        if self.invite_busy(&id) {
            return;
        }
        let room = id.clone();
        self.command(
            |seq| Command::LeaveRoom { id: seq, room },
            Request::DeclineInvite(id),
        );
    }

    /// Accepting or declining `id` is under way.
    pub fn invite_busy(&self, id: &str) -> bool {
        self.pending.values().any(|r| {
            matches!(r, Request::AcceptInvite(room) | Request::DeclineInvite(room) if room == id)
        })
    }

    pub fn invite_answered(&mut self, id: String, accepted: bool, ok: bool) {
        if !ok {
            self.status = if accepted {
                "Couldn't join. The invite may have been withdrawn."
            } else {
                "Couldn't decline the invite. Try again."
            }
            .into();
            return;
        }
        if self.invite_open.as_ref() == Some(&id) {
            self.invite_open = None;
        }
        if accepted {
            self.joined_invite = Some(id);
            self.open_joined_invite();
        }
    }

    /// The room an accepted invite (or a link, joining.rs) joined, once the
    /// room list has it.
    pub fn open_joined_invite(&mut self) {
        let Some(id) = &self.joined_invite else {
            return;
        };
        let Some(room) = self.live.list.get(id).filter(|r| r.has_metadata) else {
            return;
        };
        let id = id.clone();
        self.joined_invite = None;
        self.section = if room.space {
            Section::Space(id.clone())
        } else if room.dm {
            Section::Direct
        } else {
            Section::All
        };
        if !room.space {
            self.open(id);
            if let Some(event) = self.joined_event.take() {
                self.jump(event);
            }
        }
        self.reconcile();
    }

    /// Unread marks for the rail: DM invites count on Direct Messages, room
    /// invites on All Rooms.
    pub fn invites_in(&self, section: &Section) -> u64 {
        self.section_invites(section).len() as u64
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{Event, Invite, RoomList};

    fn invite(id: &str, dm: bool, space: bool) -> Invite {
        Invite {
            id: id.into(),
            name: id.into(),
            dm,
            space,
            inviter: Some("@mira:example".into()),
            ..Default::default()
        }
    }

    #[test]
    fn each_invite_shows_where_discord_would() {
        let (commands, _) = async_channel::bounded(4);
        let mut app = crate::App::new(commands);
        app.logged_in = true;
        app.engine_event(Event::Rooms(RoomList {
            invites: vec![
                invite("!dm:example", true, false),
                invite("!room:example", false, false),
                invite("!space:example", false, true),
            ],
            ..Default::default()
        }));
        let ids = |s| -> Vec<String> {
            app.section_invites(&s)
                .iter()
                .map(|i| i.id.clone())
                .collect()
        };
        assert_eq!(ids(crate::Section::Direct), ["!dm:example"]);
        assert_eq!(ids(crate::Section::All), ["!room:example"]);
        assert!(ids(crate::Section::Space("!space:example".into())).is_empty());
        app.open_invite("!room:example".into());
        assert_eq!(app.invite_open.as_deref(), Some("!room:example"));
        app.open_invite("!gone:example".into());
        assert_eq!(app.invite_open.as_deref(), Some("!room:example"));
    }
}
