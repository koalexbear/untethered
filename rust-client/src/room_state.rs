//! Room state the App reads, whichever engine fetched it: the documents
//! behind permissions and the membership list. gomuks answers are converted
//! here at the edge; the matrix-sdk engine builds this directly.
//!
//! The requests that read or change room state are here too, one method per
//! operation, so feature code doesn't care which engine runs.
use crate::{
    App, Request,
    moderation::{Action, Kind, Member},
    permissions::{Posting, StateDoc},
};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct RoomState {
    pub create: Option<StateDoc>,
    pub power: Option<StateDoc>,
    pub members: BTreeMap<String, Member>,
}

impl RoomState {
    pub fn posting(&self) -> Posting {
        Posting::new(self.create.clone(), self.power.clone())
    }

    pub fn membership(&self, user: &str) -> Option<Kind> {
        self.members.get(user).map(|m| m.kind)
    }
}

impl App {
    /// Fresh room state from the homeserver, members included.
    pub fn fetch_room_state(&mut self, room: &str, request: Request) {
        let room = room.to_owned();
        self.command(|id| crate::engine::Command::RoomState { id, room }, request);
    }

    pub fn send_membership(
        &mut self,
        room: &str,
        user: &str,
        action: Action,
        reason: &str,
        request: Request,
    ) {
        let (room, user, reason) = (room.to_owned(), user.to_owned(), reason.to_owned());
        self.command(
            |id| crate::engine::Command::Moderate {
                id,
                room,
                user,
                action,
                reason,
            },
            request,
        );
    }

    /// Replace the room's power levels document; answered with the event ID.
    pub fn send_power_levels(&mut self, room: &str, content: Value, request: Request) {
        let room = room.to_owned();
        self.command(
            |id| crate::engine::Command::SetPowerLevels { id, room, content },
            request,
        );
    }

    /// Create a room from a Matrix `createRoom` request body; answered with
    /// the room ID.
    pub fn send_create_room(&mut self, body: Value, request: Request) {
        self.command(
            |id| crate::engine::Command::CreateRoom { id, body },
            request,
        );
    }

    /// Link `child` into `space`; answered with the event ID.
    pub fn send_space_child(&mut self, space: &str, child: &str, via: &str, request: Request) {
        let (space, child, via) = (space.to_owned(), child.to_owned(), vec![via.to_owned()]);
        self.command(
            |id| crate::engine::Command::AddToSpace {
                id,
                space,
                child,
                via,
            },
            request,
        );
    }
}
