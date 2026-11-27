//! Rooms reached by a link or address (ROADMAP J2): what the homeserver
//! says about one before joining (the SDK's room preview: the room summary,
//! else the space hierarchy), and joining it, by ID or by address.
use super::{Failure, JoinRule, RoomPreview};
use matrix_sdk::{
    Client, RoomState,
    ruma::{OwnedServerName, RoomOrAliasId, ServerName, room::JoinRuleSummary, room::RoomType},
};

fn servers(via: &[String]) -> Vec<OwnedServerName> {
    via.iter()
        .filter_map(|s| ServerName::parse(s).ok())
        .collect()
}

pub async fn preview(
    client: &Client,
    target: &str,
    via: &[String],
) -> Result<RoomPreview, Failure> {
    let id = RoomOrAliasId::parse(target).map_err(|_| Failure::Unavailable)?;
    let p = client
        .get_room_preview(&id, servers(via))
        .await
        .map_err(|e| super::messaging::http(&e))?;
    Ok(RoomPreview {
        room_id: p.room_id.to_string(),
        alias: p.canonical_alias.map(|a| a.to_string()),
        name: p.name.filter(|n| !n.trim().is_empty()),
        topic: p.topic.filter(|t| !t.trim().is_empty()),
        avatar: p.avatar_url.map(|u| u.to_string()),
        members: p.num_joined_members,
        space: p.room_type == Some(RoomType::Space),
        joined: p.state == Some(RoomState::Joined),
        join: match p.join_rule {
            None | Some(JoinRuleSummary::Public) => JoinRule::Open,
            Some(JoinRuleSummary::Restricted(_)) => JoinRule::Members,
            Some(JoinRuleSummary::Knock | JoinRuleSummary::KnockRestricted(_)) => JoinRule::Knock,
            Some(_) => JoinRule::Invite,
        },
    })
}

pub async fn join(client: &Client, target: &str, via: &[String]) -> Result<(), Failure> {
    let id = RoomOrAliasId::parse(target).map_err(|_| Failure::Unavailable)?;
    client
        .join_room_by_id_or_alias(&id, &servers(via))
        .await
        .map(|_| ())
        .map_err(|e| super::messaging::http(&e))
}
