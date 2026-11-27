//! Typing notices (M3) on the matrix-sdk engine. The App's start and stop
//! requests go out through `Room::typing_notice`, one at a time so a stop
//! never overtakes the start before it; every room's `m.typing` comes back
//! as [`Event::Typing`] without our own user. The "Send typing indicators"
//! switch is account data, like the read receipt preference.
use super::{Event, Failure, messaging};
use async_channel::{Receiver, Sender};
use matrix_sdk::{
    Client, Room,
    ruma::{RoomId, events::typing::SyncTypingEvent},
};
use serde_json::{Value, json};

/// Account data holding the switch. Missing means on, as in Element.
pub const SHARING: &str = "com.untethered.typing_notices.v1";

/// Whether the switch's account data content says to send notices.
pub fn sends(content: &Value) -> bool {
    content["send"] != false
}

pub async fn share(client: &Client, send: bool) -> Result<(), Failure> {
    messaging::set_preference(client, SHARING, json!({ "send": send })).await
}

/// Send the App's typing notices in order. Once some queue up (a slow
/// homeserver), only each room's newest one matters.
pub async fn notices(client: Client, requests: Receiver<(String, bool)>) {
    while let Ok(first) = requests.recv().await {
        let mut batch = vec![first];
        while let Ok(next) = requests.try_recv() {
            batch.retain(|(room, _)| *room != next.0);
            batch.push(next);
        }
        for (room, typing) in batch {
            let Some(room) = RoomId::parse(&room)
                .ok()
                .and_then(|id| client.get_room(&id))
            else {
                continue;
            };
            // Not worth a message when one fails: the next change to the
            // draft sends another, and the homeserver times a start out.
            let _ = room.typing_notice(typing).await;
        }
    }
}

/// Report who is typing in each room whenever that changes.
pub fn watch(client: &Client, events: Sender<Event>) {
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |event: SyncTypingEvent, room: Room| {
        let events = events.clone();
        async move {
            let me = room.own_user_id();
            let users = event
                .content
                .user_ids
                .iter()
                .filter(|user| *user != me)
                .map(ToString::to_string)
                .collect();
            let room = room.room_id().to_string();
            let _ = events.send(Event::Typing { room, users }).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_on_unless_the_account_data_turns_it_off() {
        assert!(sends(&Value::Null));
        assert!(sends(&json!({})));
        assert!(sends(&json!({ "send": true })));
        assert!(!sends(&json!({ "send": false })));
    }
}
