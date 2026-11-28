//! Per-server profiles on the matrix-sdk engine: nicknames (ROADMAP M12)
//! and pictures (M15). Matrix has no per-space profile: a nickname is the
//! `displayname`, and a server picture the `avatar_url`, in this account's
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
    /// The picture this account shows in the room (mxc URI).
    pub avatar: Option<String>,
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
        name: text(&content["displayname"]).map(Into::into),
        avatar: text(&content["avatar_url"]).map(Into::into),
        since: e["origin_server_ts"].as_i64().unwrap_or_default(),
        joined,
    })
}

/// A member event field as the room shows it: unset when empty.
fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|v| !v.is_empty())
}

/// What Edit Server Profile changes in each room's member event: None
/// leaves a field as it is, Some(None) puts the global profile's back.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub name: Option<Option<String>>,
    pub avatar: Option<Option<Picture>>,
}

/// A server picture: uploaded once, then shown in every room.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Picture {
    Uri(String),
    Upload(Vec<u8>),
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uri(uri) => write!(f, "Uri({uri})"),
            Self::Upload(bytes) => write!(f, "Upload({} bytes)", bytes.len()),
        }
    }
}

/// The member event fields to write: None leaves one, Some(None) unsets it.
#[derive(Clone, Copy, Default)]
struct Fields<'a> {
    name: Option<Option<&'a str>>,
    avatar: Option<Option<&'a str>>,
}

/// Apply `change` to this account's member event in each of `rooms`.
/// Rooms already showing it are left alone, and one room's failure doesn't
/// stop the others.
pub(super) async fn set(client: &Client, rooms: &[String], change: Change) -> Result<(), Failure> {
    let account = client.account();
    let name = match change.name {
        None => None,
        Some(Some(name)) if !name.trim().is_empty() => Some(Some(name)),
        Some(_) => Some(
            account
                .get_display_name()
                .await
                .map_err(|e| messaging::http(&e))?,
        ),
    };
    let avatar = match change.avatar {
        None => None,
        Some(Some(Picture::Uri(uri))) => Some(Some(uri)),
        Some(Some(Picture::Upload(bytes))) => {
            Some(Some(super::profile::upload(client, bytes).await?))
        }
        Some(None) => Some(
            account
                .get_avatar_url()
                .await
                .map_err(|e| messaging::http(&e))?
                .map(|uri| uri.to_string()),
        ),
    };
    let fields = Fields {
        name: name.as_ref().map(Option::as_deref),
        avatar: avatar.as_ref().map(Option::as_deref),
    };
    let mut result = Ok(());
    for room in rooms {
        if let Err(failure) = set_in(client, room, fields).await {
            result = Err(failure);
        }
    }
    result
}

/// Give a room joined after its server's nickname or picture was set that
/// profile, while this account's member event there is still its join: a
/// name or picture set since, here or in another client, stays.
pub(super) async fn follow(
    client: &Client,
    room: &str,
    name: Option<String>,
    avatar: Option<String>,
) -> Result<(), Failure> {
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
    let fields = Fields {
        name: name.as_deref().map(Some),
        avatar: avatar.as_deref().map(Some),
    };
    set_in(client, room, fields).await
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

async fn set_in(client: &Client, room: &str, fields: Fields<'_>) -> Result<(), Failure> {
    let room = messaging::room(client, room)?;
    let me = room.own_user_id().to_owned();
    let content = match stored(&room).await {
        Some(e) => e["content"].clone(),
        None => fetched(client, &room).await?,
    };
    let Some(content) = edited(content, fields) else {
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

/// The member content with `fields` applied, or None when it already shows
/// them or isn't a join. Everything else stays, unknown fields included,
/// except what belongs to the join itself.
fn edited(mut content: Value, fields: Fields<'_>) -> Option<Value> {
    let map = content.as_object_mut()?;
    if map.get("membership").and_then(Value::as_str) != Some("join") {
        return None;
    }
    let changes: Vec<(&str, Option<&str>)> =
        [("displayname", fields.name), ("avatar_url", fields.avatar)]
            .into_iter()
            .filter_map(|(key, value)| Some((key, value?)))
            .filter(|(key, value)| map.get(*key).and_then(text) != *value)
            .collect();
    if changes.is_empty() {
        return None;
    }
    // A restricted room's join authorisation and the join's reason say how
    // this account joined; a profile change mustn't repeat them.
    map.remove("join_authorised_via_users_server");
    map.remove("reason");
    for (key, value) in changes {
        match value {
            Some(value) => map.insert(key.into(), value.into()),
            None => map.remove(key),
        };
    }
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
                avatar: None,
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

    fn name(name: Option<&str>) -> Fields<'_> {
        Fields {
            name: Some(name),
            ..Fields::default()
        }
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
            edited(content.clone(), name(Some("Captain"))),
            Some(json!({
                "membership": "join",
                "displayname": "Captain",
                "avatar_url": "mxc://example.org/a",
                "xyz.example.extra": true,
            }))
        );
        assert_eq!(edited(content.clone(), name(Some("Mira"))), None);
        assert_eq!(
            edited(content, name(None)).unwrap().get("displayname"),
            None,
            "no global name leaves the ID to show"
        );
        assert_eq!(
            edited(json!({"membership": "join"}), name(None)),
            None,
            "already nameless"
        );
        assert_eq!(
            edited(json!({"membership": "leave"}), name(Some("Captain"))),
            None
        );
    }

    #[test]
    fn a_server_picture_changes_only_the_picture() {
        let content = json!({"membership": "join", "displayname": "Mira", "reason": "hi"});
        let picture = |avatar| Fields {
            avatar: Some(avatar),
            ..Fields::default()
        };
        assert_eq!(
            edited(content.clone(), picture(Some("mxc://h/server"))),
            Some(
                json!({"membership": "join", "displayname": "Mira", "avatar_url": "mxc://h/server"})
            )
        );
        let pictured = edited(content, picture(Some("mxc://h/server"))).unwrap();
        assert_eq!(
            edited(pictured.clone(), picture(Some("mxc://h/server"))),
            None
        );
        // The global profile has no picture: the room shows none either.
        assert_eq!(
            edited(pictured.clone(), picture(None)),
            Some(json!({"membership": "join", "displayname": "Mira"}))
        );
        // Both at once, in one member event; one already right is no change.
        let both = Fields {
            name: Some(Some("Mira")),
            avatar: Some(Some("mxc://h/other")),
        };
        assert_eq!(
            edited(pictured, both).unwrap()["avatar_url"],
            "mxc://h/other"
        );
        assert_eq!(
            own_from(&json!({"content": {"membership": "join", "avatar_url": "mxc://h/a"}}))
                .unwrap()
                .avatar
                .as_deref(),
            Some("mxc://h/a")
        );
    }
}
