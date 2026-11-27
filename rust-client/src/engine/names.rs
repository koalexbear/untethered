//! Per-server nicknames (ROADMAP M12) on the matrix-sdk engine. Matrix has
//! no per-space name: a nickname is the `displayname` in this account's
//! `m.room.member` event in each room of the space, which Element and other
//! clients show in those rooms only. The App decides which rooms
//! (`crate::names`); this reads and writes the member events.
use super::{Failure, messaging};
use matrix_sdk::{
    Client, Room,
    deserialized_responses::RawAnySyncOrStrippedState,
    ruma::{EventId, api::client::state::get_state_event_for_key, events::StateEventType},
};
use serde_json::Value;

/// This account's membership in a room, as names need it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Own {
    /// The name this account shows in the room.
    pub name: Option<String>,
    /// When the member event was sent (`origin_server_ts`).
    pub since: i64,
    /// The member event is the join itself (Some(true)), not a later change
    /// of name or picture: nobody has set a name here since joining. None
    /// when the event doesn't say: Synapse's sliding sync leaves out the
    /// previous content, so only `follow` can tell.
    pub joined: Option<bool>,
}

/// This account's member event from the store. Simplified Sliding Sync's
/// `required_state` includes it (`$ME`), so this is a local read.
pub(super) async fn own(room: &Room) -> Option<Own> {
    own_from(&stored(room).await?)
}

async fn stored(room: &Room) -> Option<Value> {
    let me = room.own_user_id().as_str();
    let RawAnySyncOrStrippedState::Sync(raw) = room
        .get_state_event(StateEventType::RoomMember, me)
        .await
        .ok()??
    else {
        return None;
    };
    raw.deserialize_as().ok()
}

fn own_from(e: &Value) -> Option<Own> {
    let content = &e["content"];
    if content["membership"] != "join" {
        return None;
    }
    let unsigned = &e["unsigned"];
    // A join replaces an invite, a leave or nothing.
    let joined = match unsigned["prev_content"]["membership"].as_str() {
        Some(previous) => Some(previous != "join"),
        None if unsigned.get("replaces_state").is_none() => Some(true),
        None => None,
    };
    Some(Own {
        name: content["displayname"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map(Into::into),
        since: e["origin_server_ts"].as_i64().unwrap_or_default(),
        joined,
    })
}

/// Show `name` as this account's name in each of `rooms`; None puts the
/// global profile name back. Rooms already showing it are left alone, and
/// one room's failure doesn't stop the others.
pub(super) async fn set(
    client: &Client,
    rooms: &[String],
    name: Option<String>,
) -> Result<(), Failure> {
    let name = match name.filter(|n| !n.trim().is_empty()) {
        Some(name) => Some(name),
        None => client
            .account()
            .get_display_name()
            .await
            .map_err(|e| messaging::http(&e))?,
    };
    let mut result = Ok(());
    for room in rooms {
        if let Err(failure) = set_in(client, room, name.as_deref()).await {
            result = Err(failure);
        }
    }
    result
}

/// Give a room joined after its server's nickname was set that nickname,
/// while this account's member event there is still its join: a name set
/// since, here or in another client, stays.
pub(super) async fn follow(client: &Client, room: &str, name: String) -> Result<(), Failure> {
    let joined = messaging::room(client, room)?;
    let Some(e) = stored(&joined).await else {
        return Ok(());
    };
    let still = match own_from(&e).and_then(|own| own.joined) {
        Some(known) => known,
        None => replaces_a_join(&joined, &e).await? == Some(false),
    };
    if !still {
        return Ok(());
    }
    set_in(client, room, Some(&name)).await
}

/// Whether the member event `e` replaced a join, from the replaced event.
async fn replaces_a_join(room: &Room, e: &Value) -> Result<Option<bool>, Failure> {
    let Some(id) = e["unsigned"]["replaces_state"]
        .as_str()
        .and_then(|id| EventId::parse(id).ok())
    else {
        return Ok(None);
    };
    let replaced = room
        .load_or_fetch_event(&id, None)
        .await
        .map_err(|e| messaging::http(&e))?;
    let replaced: Value = replaced
        .raw()
        .deserialize_as()
        .map_err(|_| Failure::Failed)?;
    Ok(Some(replaced["content"]["membership"] == "join"))
}

async fn set_in(client: &Client, room: &str, name: Option<&str>) -> Result<(), Failure> {
    let room = messaging::room(client, room)?;
    let me = room.own_user_id().to_owned();
    let content = match stored(&room).await {
        Some(e) => e["content"].clone(),
        None => fetched(client, &room).await?,
    };
    let Some(content) = renamed(content, name) else {
        return Ok(());
    };
    room.send_state_event_raw("m.room.member", me.as_str(), content)
        .await
        .map(|_| ())
        .map_err(|e| messaging::http(&e))
}

/// The member event's content from the homeserver, when the store has none.
async fn fetched(client: &Client, room: &Room) -> Result<Value, Failure> {
    let request = get_state_event_for_key::v3::Request::new(
        room.room_id().to_owned(),
        StateEventType::RoomMember,
        room.own_user_id().to_string(),
    );
    let response = client.send(request).await.map_err(|e| {
        if e.client_api_error_kind().is_some() {
            Failure::Forbidden
        } else {
            Failure::Unreachable
        }
    })?;
    serde_json::from_str(response.event_or_content.get()).map_err(|_| Failure::Failed)
}

/// The member content with `name` as the display name, or None when it
/// already shows that name or isn't a join. Everything else stays, picture
/// and unknown fields included, except what belongs to the join itself.
fn renamed(mut content: Value, name: Option<&str>) -> Option<Value> {
    let fields = content.as_object_mut()?;
    if fields.get("membership").and_then(Value::as_str) != Some("join")
        || fields
            .get("displayname")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            == name
    {
        return None;
    }
    // A restricted room's join authorisation and the join's reason say how
    // this account joined; a name change mustn't repeat them.
    fields.remove("join_authorised_via_users_server");
    fields.remove("reason");
    match name {
        Some(name) => fields.insert("displayname".into(), name.into()),
        None => fields.remove("displayname"),
    };
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_join_is_told_from_a_later_name_change() {
        let join = json!({
            "content": {"membership": "join", "displayname": "Mira"},
            "origin_server_ts": 5,
            "unsigned": {"prev_content": {"membership": "invite"}, "replaces_state": "$i"},
        });
        assert_eq!(
            own_from(&join),
            Some(Own {
                name: Some("Mira".into()),
                since: 5,
                joined: Some(true)
            })
        );
        let renamed = json!({
            "content": {"membership": "join", "displayname": "Captain"},
            "origin_server_ts": 9,
            "unsigned": {"prev_content": {"membership": "join"}, "replaces_state": "$j"},
        });
        assert_eq!(own_from(&renamed).unwrap().joined, Some(false));
        // The room's first membership event is a join.
        let created = json!({"content": {"membership": "join"}, "origin_server_ts": 1});
        assert_eq!(own_from(&created).unwrap().joined, Some(true));
        assert_eq!(own_from(&created).unwrap().name, None);
        // Replacing an unknown event: not known to be the join.
        let unknown =
            json!({"content": {"membership": "join"}, "unsigned": {"replaces_state": "$x"}});
        assert_eq!(own_from(&unknown).unwrap().joined, None);
        assert_eq!(own_from(&json!({"content": {"membership": "leave"}})), None);
    }

    #[test]
    fn a_rename_keeps_the_picture_and_unknown_fields_but_not_the_join() {
        let content = json!({
            "membership": "join",
            "displayname": "Mira",
            "avatar_url": "mxc://example.org/a",
            "xyz.example.extra": true,
            "join_authorised_via_users_server": "@admin:example.org",
            "reason": "hello",
        });
        assert_eq!(
            renamed(content.clone(), Some("Captain")),
            Some(json!({
                "membership": "join",
                "displayname": "Captain",
                "avatar_url": "mxc://example.org/a",
                "xyz.example.extra": true,
            }))
        );
        assert_eq!(renamed(content.clone(), Some("Mira")), None);
        assert_eq!(
            renamed(content, None).unwrap().get("displayname"),
            None,
            "no global name leaves the ID to show"
        );
        assert_eq!(
            renamed(json!({"membership": "join"}), None),
            None,
            "already nameless"
        );
        assert_eq!(
            renamed(json!({"membership": "leave"}), Some("Captain")),
            None
        );
    }
}
