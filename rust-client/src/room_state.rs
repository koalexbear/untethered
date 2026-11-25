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
    rooms::items,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Default, Debug)]
pub struct RoomState {
    pub create: Option<StateDoc>,
    pub power: Option<StateDoc>,
    pub members: BTreeMap<String, Member>,
}

impl RoomState {
    /// gomuks's `get_room_state` answer for `room`.
    pub fn from_gomuks(room: &str, data: &Value) -> Self {
        let mut state = Self::default();
        for e in items(data) {
            if let Some((user, member)) = Member::read(room, e) {
                state.members.insert(user.to_owned(), member);
                continue;
            }
            if e["state_key"] != "" {
                continue;
            }
            let slot = match e["type"].as_str() {
                Some("m.room.create") => &mut state.create,
                Some("m.room.power_levels") => &mut state.power,
                _ => continue,
            };
            if let Some(doc) = StateDoc::from_gomuks(room, e) {
                *slot = Some(doc);
            }
        }
        state
    }

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
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            let room = room.to_owned();
            return self.command(|id| crate::engine::Command::RoomState { id, room }, request);
        }
        // gomuks waits for the server refetch only with include_members=true.
        self.rpc(
            "get_room_state",
            json!({"room_id":room,"refetch":true,"include_members":true}),
            request,
        );
    }

    pub fn send_membership(
        &mut self,
        room: &str,
        user: &str,
        action: Action,
        reason: &str,
        request: Request,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            let (room, user, reason) = (room.to_owned(), user.to_owned(), reason.to_owned());
            return self.command(
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
        let mut body = json!({"room_id":room,"user_id":user,"action":action.wire()});
        if !reason.is_empty() {
            body["reason"] = reason.into();
        }
        self.rpc("set_membership", body, request);
    }

    /// Replace the room's power levels document; answered with the event ID.
    pub fn send_power_levels(&mut self, room: &str, content: Value, request: Request) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            let room = room.to_owned();
            return self.command(
                |id| crate::engine::Command::SetPowerLevels { id, room, content },
                request,
            );
        }
        self.rpc(
            "set_state",
            json!({"room_id":room,"type":"m.room.power_levels","state_key":"","content":content}),
            request,
        );
    }

    /// Create a room from a Matrix `createRoom` request body; answered with
    /// the room ID.
    pub fn send_create_room(&mut self, body: Value, request: Request) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            return self.command(
                |id| crate::engine::Command::CreateRoom { id, body },
                request,
            );
        }
        self.rpc("create_room", body, request);
    }

    /// Link `child` into `space`; answered with the event ID.
    pub fn send_space_child(&mut self, space: &str, child: &str, via: &str, request: Request) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            let (space, child, via) = (space.to_owned(), child.to_owned(), vec![via.to_owned()]);
            return self.command(
                |id| crate::engine::Command::AddToSpace {
                    id,
                    space,
                    child,
                    via,
                },
                request,
            );
        }
        self.rpc(
            "set_state",
            json!({"room_id":space,"type":"m.space.child","state_key":child,"content":{"via":[via]}}),
            request,
        );
    }
}
