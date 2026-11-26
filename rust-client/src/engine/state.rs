//! Room state and changes to it: the membership list and permission
//! documents for the App's [`RoomState`], membership actions, power levels,
//! new rooms and space links.
use super::{
    Failure,
    messaging::{http, room},
};
use crate::{
    moderation::{Action, Kind, Member},
    permissions::StateDoc,
    room_state::RoomState,
};
use matrix_sdk::{
    Client,
    ruma::{
        RoomId, ServerName, UserId,
        api::client::{room::create_room, state::get_state_events},
        events::space::child::SpaceChildEventContent,
        serde::Raw,
    },
};
use serde_json::Value;

/// The room's state from the homeserver (`/state`), not the local store:
/// changes are checked against it before they're made.
pub async fn room_state(client: &Client, room: &str) -> Result<RoomState, Failure> {
    let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
    let response = client
        .send(get_state_events::v3::Request::new(id))
        .await
        .map_err(|e| {
            if e.client_api_error_kind().is_some() {
                Failure::Forbidden
            } else {
                Failure::Unreachable
            }
        })?;
    let mut state = RoomState::default();
    for raw in response.room_state {
        // Read fields rather than ruma's typed events: power levels must keep
        // fields ruma doesn't know, and a malformed member event shouldn't
        // hide the rest.
        let Ok(e) = raw.deserialize_as::<Value>() else {
            continue;
        };
        let (Some(kind), Some(key), Some(event_id)) = (
            e["type"].as_str(),
            e["state_key"].as_str(),
            e["event_id"].as_str(),
        ) else {
            continue;
        };
        match (kind, key) {
            ("m.room.create", "") => state.create = Some(doc(&e, event_id)),
            ("m.room.power_levels", "") => state.power = Some(doc(&e, event_id)),
            ("m.room.member", user) if UserId::parse(user).is_ok() => {
                let kind = match e["content"]["membership"].as_str() {
                    Some("join") => Kind::Joined,
                    Some("invite") => Kind::Invited,
                    Some("knock") => Kind::Knocking,
                    Some("ban") => Kind::Banned,
                    Some("leave") => Kind::Left,
                    _ => continue,
                };
                let member = Member {
                    kind,
                    name: e["content"]["displayname"]
                        .as_str()
                        .unwrap_or_default()
                        .into(),
                    avatar: e["content"]["avatar_url"].as_str().map(Into::into),
                    event: event_id.into(),
                };
                state.members.insert(user.into(), member);
            }
            _ => {}
        }
    }
    Ok(state)
}

/// Pass on membership changes as they sync, for an open member directory.
pub fn member_changes(client: &Client, events: async_channel::Sender<super::Event>) {
    use matrix_sdk::{
        Room,
        ruma::events::room::member::{MembershipState, SyncRoomMemberEvent},
    };
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |event: SyncRoomMemberEvent, room: Room| {
        let events = events.clone();
        async move {
            let kind = match event.membership() {
                MembershipState::Join => Kind::Joined,
                MembershipState::Invite => Kind::Invited,
                MembershipState::Knock => Kind::Knocking,
                MembershipState::Ban => Kind::Banned,
                MembershipState::Leave => Kind::Left,
                _ => return,
            };
            let member = Member {
                kind,
                name: event
                    .as_original()
                    .and_then(|e| e.content.displayname.clone())
                    .unwrap_or_default(),
                avatar: event
                    .as_original()
                    .and_then(|e| e.content.avatar_url.as_ref())
                    .map(ToString::to_string),
                event: event.event_id().to_string(),
            };
            let _ = events
                .send(super::Event::Member {
                    room: room.room_id().to_string(),
                    user: event.state_key().to_string(),
                    member,
                })
                .await;
        }
    });
}

fn doc(e: &Value, event_id: &str) -> StateDoc {
    StateDoc {
        event_id: event_id.into(),
        sender: e["sender"].as_str().unwrap_or_default().into(),
        content: e["content"].clone(),
        redacted: e["unsigned"]["redacted_because"].is_object(),
    }
}

pub async fn moderate(
    client: &Client,
    room: &str,
    user: &str,
    action: Action,
    reason: &str,
) -> Result<(), Failure> {
    let room = self::room(client, room)?;
    let user = UserId::parse(user).map_err(|_| Failure::Unavailable)?;
    let reason = Some(reason.trim()).filter(|r| !r.is_empty());
    match action {
        Action::Kick => room.kick_user(&user, reason).await,
        Action::Ban => room.ban_user(&user, reason).await,
        Action::Unban => room.unban_user(&user, reason).await,
        Action::Invite => room.invite_user_by_id(&user).await,
    }
    .map_err(|e| http(&e))
}

/// Replace the power levels document as it is, unknown fields included.
pub async fn set_power_levels(
    client: &Client,
    room: &str,
    content: Value,
) -> Result<String, Failure> {
    let room = self::room(client, room)?;
    let response = room
        .send_state_event_raw("m.room.power_levels", "", content)
        .await
        .map_err(|e| http(&e))?;
    Ok(response.event_id.to_string())
}

/// A room from a Matrix `createRoom` body, with the fields community setup
/// uses.
pub async fn create_room(client: &Client, body: Value) -> Result<String, Failure> {
    let mut request = create_room::v3::Request::new();
    request.name = body["name"].as_str().map(str::to_owned);
    request.topic = body["topic"].as_str().map(str::to_owned);
    request.is_direct = body["is_direct"] == true;
    if let Ok(visibility) = serde_json::from_value(body["visibility"].clone()) {
        request.visibility = visibility;
    }
    request.preset = serde_json::from_value(body["preset"].clone()).ok();
    request.invite = serde_json::from_value(body["invite"].clone()).unwrap_or_default();
    request.creation_content = raw(&body["creation_content"]);
    request.power_level_content_override = raw(&body["power_level_content_override"]);
    request.initial_state = body["initial_state"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(raw)
        .collect();
    let room = client.create_room(request).await.map_err(|e| http(&e))?;
    Ok(room.room_id().to_string())
}

fn raw<T>(value: &Value) -> Option<Raw<T>> {
    (!value.is_null())
        .then(|| {
            serde_json::value::to_raw_value(value)
                .ok()
                .map(Raw::from_json)
        })
        .flatten()
}

pub async fn add_to_space(
    client: &Client,
    space: &str,
    child: &str,
    via: Vec<String>,
) -> Result<String, Failure> {
    let space = self::room(client, space)?;
    let child = RoomId::parse(child).map_err(|_| Failure::Unavailable)?;
    let via = via
        .iter()
        .filter_map(|s| ServerName::parse(s).ok())
        .collect();
    let response = space
        .send_state_event_for_key(&child, SpaceChildEventContent::new(via))
        .await
        .map_err(|e| http(&e))?;
    Ok(response.event_id.to_string())
}
