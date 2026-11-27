//! Invites to rooms, spaces and DMs (ROADMAP J1): the rooms this account is
//! invited to, with who invited it, from the SDK's store. Accepting joins the
//! room (and keeps a DM a DM); declining leaves it (rooms.rs `leave`).
use super::{Failure, Invite};
use matrix_sdk::{
    Client, Room, deserialized_responses::SyncOrStrippedState, room::RoomMember, ruma::RoomId,
};

pub async fn list(client: &Client) -> Vec<Invite> {
    let mut invites = Vec::new();
    for room in client.invited_rooms() {
        invites.push(invite(&room).await);
    }
    invites.sort_by_key(|i| i.name.to_lowercase());
    invites
}

async fn invite(room: &Room) -> Invite {
    let details = room.invite_details().await.ok();
    let inviter = details.as_ref().map(|d| d.inviter_id.to_string());
    let from = details.as_ref().and_then(|d| d.inviter.as_ref());
    let inviter_name = from.and_then(|m| m.display_name().map(ToOwned::to_owned));
    let inviter_avatar = from.and_then(|m| m.avatar_url().map(ToString::to_string));
    // The invite's own `is_direct` says it's a DM before `m.direct` does.
    let dm = details.as_ref().is_some_and(|d| direct(&d.invitee))
        || room.is_direct().await.unwrap_or(false);
    let named = room
        .cached_display_name()
        .map(|n| n.to_string())
        .or_else(|| room.name())
        .filter(|n| !n.is_empty() && !n.starts_with("Empty Room"));
    let name = match (named, dm) {
        (Some(name), false) => name,
        // A DM is the person who wrote.
        (_, true) | (None, false) => inviter_name
            .clone()
            .or_else(|| inviter.clone())
            .unwrap_or_else(|| room.room_id().to_string()),
    };
    Invite {
        id: room.room_id().to_string(),
        name,
        topic: room.topic().unwrap_or_default(),
        avatar: room
            .avatar_url()
            .map(|u| u.to_string())
            .or_else(|| dm.then(|| inviter_avatar.clone()).flatten()),
        space: room.is_space(),
        dm,
        inviter,
        inviter_name,
        inviter_avatar,
    }
}

/// The invite event's `is_direct`: the inviter meant a DM.
fn direct(invitee: &RoomMember) -> bool {
    match &**invitee.event() {
        SyncOrStrippedState::Stripped(e) => e.content.is_direct,
        SyncOrStrippedState::Sync(e) => e.as_original().and_then(|e| e.content.is_direct),
    }
    .unwrap_or(false)
}

/// Join an invited room. A DM invite stays a DM in `m.direct`, as Element
/// marks it when accepting.
pub async fn accept(client: &Client, room: &str) -> Result<(), Failure> {
    let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
    let room = client.get_room(&id).ok_or(Failure::Unavailable)?;
    let dm = room
        .invite_details()
        .await
        .is_ok_and(|d| direct(&d.invitee));
    room.join().await.map_err(|e| super::messaging::http(&e))?;
    if dm && !room.is_direct().await.unwrap_or(false) {
        let _ = room.set_is_direct(true).await;
    }
    Ok(())
}
