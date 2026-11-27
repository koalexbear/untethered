//! People to message and to block (ROADMAP J3): the homeserver's user
//! directory, DMs with one or several people, and the ignore list
//! (`m.ignored_user_list`), which homeservers and other clients honour too.
use super::{Failure, Person, messaging::http};
use matrix_sdk::{
    Client,
    ruma::{
        OwnedUserId, UserId,
        api::client::room::create_room::v3::{Request, RoomPreset},
        events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
    },
};

/// How many people a directory search returns.
const RESULTS: u64 = 20;

/// People in the user directory matching `term`, never this account. A
/// homeserver may list only people who share a room with it or are in a
/// public room.
pub async fn search(client: &Client, term: &str) -> Result<Vec<Person>, Failure> {
    let found = client
        .search_users(term, RESULTS)
        .await
        .map_err(|e| http(&e.into()))?;
    let me = client.user_id();
    Ok(found
        .results
        .into_iter()
        .filter(|u| Some(u.user_id.as_ref()) != me)
        .map(|u| Person {
            id: u.user_id.to_string(),
            name: u.display_name.filter(|n| !n.trim().is_empty()),
            avatar: u.avatar_url.map(|a| a.to_string()),
        })
        .collect())
}

/// A conversation with `users`: the DM this account already has with one
/// person, or a new encrypted room that every client lists as a DM (an
/// `is_direct` invite, and `m.direct` for each person).
pub async fn start(client: &Client, users: &[String]) -> Result<String, Failure> {
    let me = client.user_id().ok_or(Failure::Unavailable)?;
    let users = users
        .iter()
        .map(|u| UserId::parse(u.as_str()))
        .collect::<Result<Vec<OwnedUserId>, _>>()
        .map_err(|_| Failure::Failed)?;
    if users.is_empty() || users.iter().any(|u| u == me) {
        return Err(Failure::Failed);
    }
    if let [user] = users.as_slice()
        && let Some(room) = client.get_dm_room(user)
    {
        return Ok(room.room_id().to_string());
    }
    let mut request = Request::new();
    request.invite = users;
    request.is_direct = true;
    request.preset = Some(RoomPreset::TrustedPrivateChat);
    request.initial_state = vec![
        InitialStateEvent::with_empty_state_key(
            RoomEncryptionEventContent::with_recommended_defaults(),
        )
        .to_raw_any(),
    ];
    // The SDK adds the room to `m.direct` for everyone invited.
    let room = client.create_room(request).await.map_err(|e| http(&e))?;
    Ok(room.room_id().to_string())
}

/// Block or unblock someone: their messages, invites and calls stay away.
pub async fn block(client: &Client, user: &str, block: bool) -> Result<(), Failure> {
    let user = UserId::parse(user).map_err(|_| Failure::Failed)?;
    let account = client.account();
    if block {
        account.ignore_user(&user).await
    } else {
        account.unignore_user(&user).await
    }
    .map_err(|e| http(&e))
}

/// The ignore list's people, from its account data content.
pub fn blocked(content: &serde_json::Value) -> Vec<String> {
    content["ignored_users"]
        .as_object()
        .map(|users| users.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_ignore_list_names_its_people() {
        assert_eq!(
            blocked(&json!({"ignored_users": {"@mira:example.org": {}, "@bob:example.org": {}}})),
            vec![
                "@bob:example.org".to_owned(),
                "@mira:example.org".to_owned()
            ]
        );
        assert!(blocked(&json!({})).is_empty());
    }
}
