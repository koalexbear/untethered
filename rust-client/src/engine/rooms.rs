//! Joined rooms and space edges from the SDK's store, re-read whenever a sync
//! response or room-info update arrives. Simplified Sliding Sync's default
//! `required_state` includes `m.space.child`, so space edges are local reads.
use super::{Event, RoomList, RoomSummary};
use crate::permissions::StateDoc;
use async_channel::Sender;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::{RawAnySyncOrStrippedState, SyncOrStrippedState},
    ruma::events::{
        RoomAccountDataEventType, StateEventType, SyncStateEvent,
        fully_read::FullyReadEventContent, space::child::SpaceChildEventContent,
    },
};
use serde_json::Value;

/// `notifications::spaces::PREF`; that module is Linux and web only.
const SPACE_ALERTS: &str = "com.untethered.space_alerts.v1";
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::broadcast::error::RecvError;

pub async fn updates(client: Client, events: Sender<Event>) {
    let mut rooms = client.subscribe_to_all_room_updates();
    let mut info = client.room_info_notable_update_receiver();
    loop {
        if events
            .send(Event::Rooms(snapshot(&client).await))
            .await
            .is_err()
        {
            return;
        }
        let closed = tokio::select! {
            r = rooms.recv() => matches!(r, Err(RecvError::Closed)),
            r = info.recv() => matches!(r, Err(RecvError::Closed)),
        };
        if closed {
            return;
        }
        // Coalesce a burst (initial sync, many rooms) into one snapshot.
        tokio::time::sleep(Duration::from_millis(80)).await;
        while rooms.try_recv().is_ok() {}
        while info.try_recv().is_ok() {}
    }
}

async fn snapshot(client: &Client) -> RoomList {
    let mut list = RoomList::default();
    for room in client.joined_rooms() {
        let summary = summary(&room).await;
        if summary.space {
            list.children
                .insert(summary.id.clone(), children(&room).await);
        }
        list.rooms.push(summary);
    }
    list
}

async fn summary(room: &Room) -> RoomSummary {
    let id = room.room_id().to_string();
    let dm = room.is_direct().await.unwrap_or(false);
    let counts = room.unread_notification_counts();
    RoomSummary {
        name: room
            .cached_display_name()
            .map(|n| n.to_string())
            .or_else(|| room.name())
            .or_else(|| room.canonical_alias().map(|a| a.to_string()))
            .unwrap_or_else(|| id.clone()),
        topic: room.topic().unwrap_or_default(),
        space: room.is_space(),
        dm,
        dm_peer: dm
            .then(|| {
                room.direct_targets()
                    .iter()
                    .find_map(|t| t.as_user_id().map(ToString::to_string))
            })
            .flatten(),
        voice: room.is_call(),
        encrypted: room.encryption_state().is_encrypted(),
        unread_messages: room.num_unread_messages(),
        unread_mentions: room.num_unread_mentions(),
        notifications: counts.notification_count,
        highlights: counts.highlight_count,
        marked_unread: room.is_marked_unread(),
        fully_read: room
            .account_data_static::<FullyReadEventContent>()
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.deserialize().ok())
            .map(|e| e.content.event_id.to_string()),
        create: state_doc(room, StateEventType::RoomCreate).await,
        power: state_doc(room, StateEventType::RoomPowerLevels).await,
        space_alerts: room
            .account_data(RoomAccountDataEventType::from(SPACE_ALERTS))
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.deserialize_as::<Value>().ok())
            .map(|e| e["content"].clone()),
        id,
    }
}

/// A state event from the store as the App keeps it. Fields are read from
/// the JSON so power levels keep what ruma doesn't model.
async fn state_doc(room: &Room, kind: StateEventType) -> Option<StateDoc> {
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

/// Children with a non-empty `via`; an empty or redacted edge is a removal.
async fn children(space: &Room) -> BTreeSet<String> {
    let Ok(edges) = space
        .get_state_events_static::<SpaceChildEventContent>()
        .await
    else {
        return BTreeSet::new();
    };
    edges
        .into_iter()
        .filter_map(|raw| match raw.deserialize().ok()? {
            SyncOrStrippedState::Sync(SyncStateEvent::Original(e)) if !e.content.via.is_empty() => {
                Some(e.state_key.to_string())
            }
            SyncOrStrippedState::Stripped(e)
                if e.content.via.as_ref().is_some_and(|v| !v.is_empty()) =>
            {
                Some(e.state_key.to_string())
            }
            _ => None,
        })
        .collect()
}
