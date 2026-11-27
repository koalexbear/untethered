use super::{Failure, Message, media::Sources, messaging};
use futures_util::{StreamExt, stream};
use matrix_sdk::{Room, ruma::events::room::pinned_events::RoomPinnedEventsEventContent};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Pin {
    pub event: String,
    pub message: Option<Message>,
}

pub(super) async fn list(room: &Room, me: &str, sources: &Sources) -> Result<Vec<Pin>, Failure> {
    let ids = room
        .load_pinned_events()
        .await
        .map_err(|err| messaging::http(&err))?
        .unwrap_or_default();
    Ok(
        stream::iter(ids.into_iter().enumerate().map(|(index, id)| async move {
            let message = room
                .load_or_fetch_event(&id, None)
                .await
                .ok()
                .and_then(|event| messaging::message(&event, index as i64 + 1, me, sources));
            Pin {
                event: id.to_string(),
                message,
            }
        }))
        .buffered(6)
        .collect()
        .await,
    )
}

pub(super) async fn set(room: &Room, event: &str, pinned: bool) -> Result<(), Failure> {
    let event = messaging::event_id(event)?;
    // Read fresh state before mutating it. Sync may not have echoed our most
    // recent change yet, so relying on the cached list can undo that change.
    let mut ids = room
        .load_pinned_events()
        .await
        .map_err(|err| messaging::http(&err))?
        .unwrap_or_default();
    if ids.contains(&event) == pinned {
        return Ok(());
    }
    if pinned {
        ids.push(event);
    } else {
        ids.retain(|id| id != &event);
    }
    room.send_state_event(RoomPinnedEventsEventContent::new(ids))
        .await
        .map(|_| ())
        .map_err(|err| messaging::http(&err))
}
