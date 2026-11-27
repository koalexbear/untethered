//! Custom emoji and stickers (ROADMAP M11): Matrix image packs, MSC2545,
//! now in the spec (v1.19). A pack is a room's state event, keyed by its
//! state key; a space's packs serve its rooms. Your own packs and the rooms
//! whose packs you use everywhere live in your account data.
//!
//! Both names are read: the spec's (`m.room.image_pack`,
//! `m.image_pack.rooms`) and the unstable ones Cinny, FluffyChat and
//! Nheko still write (`im.ponies.room_emotes`, `im.ponies.emote_rooms`).
//! Personal packs only exist unstably (`im.ponies.user_emotes`): the spec
//! dropped them. Sliding sync delivers the spec's room packs with every
//! room; the unstable ones aren't in its `required_state`, so a space's and
//! the open room's default pack (state key "") are fetched.
use super::{Event, Failure};
use async_channel::{Receiver, Sender};
use matrix_sdk::{
    Client, Room,
    deserialized_responses::RawAnySyncOrStrippedState,
    ruma::{
        OwnedEventId, OwnedMxcUri, RoomId,
        api::client::state::get_state_event_for_key,
        events::{
            AnyGlobalAccountDataEvent, AnyMessageLikeEventContent, GlobalAccountDataEventType,
            StateEventType, room::ImageInfo, sticker::StickerEventContent,
        },
        serde::Raw,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Notify;

pub const ROOM_PACK: &str = "m.room.image_pack";
pub const ROOM_PACK_UNSTABLE: &str = "im.ponies.room_emotes";
pub const USER_PACK: &str = "im.ponies.user_emotes";
pub const PACK_ROOMS: &str = "m.image_pack.rooms";
pub const PACK_ROOMS_UNSTABLE: &str = "im.ponies.emote_rooms";

/// One pack, as the pickers show it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Pack {
    /// The room or space holding it; None for your own pack.
    pub room: Option<String>,
    /// Its state key.
    pub key: String,
    /// Under the spec's name, rather than the unstable one.
    pub stable: bool,
    pub name: String,
    pub avatar: Option<String>,
    pub attribution: Option<String>,
    pub images: Vec<Image>,
}

/// One custom emoji or sticker.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Image {
    pub shortcode: String,
    pub url: String,
    /// Its description, for stickers.
    pub body: Option<String>,
    /// `info` as the pack gives it (size, type), for sending as a sticker.
    pub info: Option<String>,
    pub emoticon: bool,
    pub sticker: bool,
}

/// Every pack this account can use, and which room packs it uses
/// everywhere, as (room, state key).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Packs {
    pub packs: Vec<Pack>,
    pub everywhere: BTreeSet<(String, String)>,
}

#[derive(Deserialize, Default)]
struct RawPack {
    #[serde(default)]
    images: BTreeMap<String, RawImage>,
    #[serde(default)]
    pack: RawMeta,
}

#[derive(Deserialize)]
struct RawImage {
    url: String,
    body: Option<String>,
    info: Option<Value>,
    usage: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct RawMeta {
    display_name: Option<String>,
    avatar_url: Option<String>,
    usage: Option<Vec<String>>,
    attribution: Option<String>,
}

/// A pack's content, under either name. Images without an mxc URI are left
/// out; one without a usage of its own takes the pack's, and a pack
/// without one serves both, as MSC2545 says.
pub fn parse(content: &Value, room: Option<&str>, key: &str, stable: bool) -> Option<Pack> {
    let raw: RawPack = serde_json::from_value(content.clone()).ok()?;
    let usage = |own: &Option<Vec<String>>| {
        let list = own.as_ref().or(raw.pack.usage.as_ref());
        match list {
            Some(l) if !l.is_empty() => (
                l.iter().any(|u| u == "emoticon"),
                l.iter().any(|u| u == "sticker"),
            ),
            _ => (true, true),
        }
    };
    let images = raw
        .images
        .iter()
        .filter(|(code, img)| {
            !code.is_empty() && !code.contains(char::is_whitespace) && img.url.starts_with("mxc://")
        })
        .map(|(code, img)| {
            let (emoticon, sticker) = usage(&img.usage);
            Image {
                shortcode: code.clone(),
                url: img.url.clone(),
                body: img.body.clone(),
                info: img.info.as_ref().map(Value::to_string),
                emoticon,
                sticker,
            }
        })
        .collect();
    Some(Pack {
        room: room.map(str::to_owned),
        key: key.to_owned(),
        stable,
        name: raw.pack.display_name.unwrap_or_default(),
        avatar: raw.pack.avatar_url,
        attribution: raw.pack.attribution,
        images,
    })
}

/// `{rooms: {room: {state key: {}}}}`, under either name.
fn everywhere(content: &Value) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    if let Some(rooms) = content["rooms"].as_object() {
        for (room, keys) in rooms {
            if let Some(keys) = keys.as_object() {
                for key in keys.keys() {
                    out.insert((room.clone(), key.clone()));
                }
            }
        }
    }
    out
}

async fn account(client: &Client, kind: &str) -> Option<Value> {
    client
        .account()
        .account_data_raw(GlobalAccountDataEventType::from(kind))
        .await
        .ok()
        .flatten()
        .and_then(|raw| raw.deserialize_as::<Value>().ok())
}

/// A room's packs from the store: the spec's name sliding sync delivers,
/// and unstable ones seen in its timeline.
async fn stored(room: &Room) -> Vec<Pack> {
    let mut out = Vec::new();
    for (kind, stable) in [(ROOM_PACK, true), (ROOM_PACK_UNSTABLE, false)] {
        let Ok(events) = room.get_state_events(StateEventType::from(kind)).await else {
            continue;
        };
        for raw in events {
            let RawAnySyncOrStrippedState::Sync(raw) = raw else {
                continue;
            };
            let Ok(event) = raw.deserialize_as::<Value>() else {
                continue;
            };
            let key = event["state_key"].as_str().unwrap_or_default();
            if let Some(pack) = parse(
                &event["content"],
                Some(room.room_id().as_str()),
                key,
                stable,
            ) && !pack.images.is_empty()
            {
                out.push(pack);
            }
        }
    }
    out
}

/// A room's unstable default pack, asked of the homeserver: None when it has
/// none (or can't be read).
async fn fetch_unstable(client: &Client, room: &RoomId) -> Option<Value> {
    let request = get_state_event_for_key::v3::Request::new(
        room.to_owned(),
        StateEventType::from(ROOM_PACK_UNSTABLE),
        String::new(),
    );
    let response = client.send(request).await.ok()?;
    serde_json::from_str(response.event_or_content.get()).ok()
}

/// Unstable default packs fetched so far, by room; None where a room has
/// none.
type Fetched = BTreeMap<String, Option<Value>>;

async fn snapshot(client: &Client, fetched: &Fetched) -> Packs {
    let mut packs = Vec::new();
    if let Some(own) = account(client, USER_PACK).await
        && let Some(mut pack) = parse(&own, None, "", false)
        && !pack.images.is_empty()
    {
        if pack.name.is_empty() {
            pack.name = "Your emoji".into();
        }
        packs.push(pack);
    }
    for room in client.joined_rooms() {
        let mut found = stored(&room).await;
        let id = room.room_id().as_str();
        if let Some(Some(content)) = fetched.get(id)
            && !found.iter().any(|p| !p.stable && p.key.is_empty())
            && let Some(pack) = parse(content, Some(id), "", false)
            && !pack.images.is_empty()
        {
            found.push(pack);
        }
        for pack in &mut found {
            if pack.name.is_empty() {
                pack.name = room
                    .cached_display_name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| id.to_owned());
            }
        }
        packs.extend(found);
    }
    let mut used = BTreeSet::new();
    for kind in [PACK_ROOMS, PACK_ROOMS_UNSTABLE] {
        if let Some(content) = account(client, kind).await {
            used.extend(everywhere(&content));
        }
    }
    Packs {
        packs,
        everywhere: used,
    }
}

/// Keep the App's packs current: on every sync's room updates, on account
/// data changes, and when a room's history is first asked for (`rooms`
/// names it), whose unstable pack is then fetched once.
pub async fn run(client: Client, events: Sender<Event>, rooms: Receiver<String>) {
    let changed = Arc::new(Notify::new());
    {
        let changed = changed.clone();
        client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
            let changed = changed.clone();
            async move {
                let kind = raw.get_field::<String>("type").ok().flatten();
                if matches!(
                    kind.as_deref(),
                    Some(USER_PACK | PACK_ROOMS | PACK_ROOMS_UNSTABLE)
                ) {
                    changed.notify_one();
                }
            }
        });
    }
    let mut updates = client.subscribe_to_all_room_updates();
    let mut fetched = Fetched::new();
    let mut last = None;
    loop {
        // Spaces' packs serve all their rooms, and rooms used everywhere
        // serve every room: fetch their unstable packs once.
        let mut wanted: Vec<String> = client
            .joined_rooms()
            .into_iter()
            .filter(|r| r.is_space())
            .map(|r| r.room_id().to_string())
            .collect();
        for kind in [PACK_ROOMS, PACK_ROOMS_UNSTABLE] {
            if let Some(content) = account(&client, kind).await {
                wanted.extend(everywhere(&content).into_iter().map(|(room, _)| room));
            }
        }
        for room in wanted {
            if !fetched.contains_key(&room)
                && let Ok(id) = RoomId::parse(&room)
            {
                let content = fetch_unstable(&client, &id).await;
                fetched.insert(room, content);
            }
        }
        let now = snapshot(&client, &fetched).await;
        if last.as_ref() != Some(&now) {
            if events.send(Event::Packs(now.clone())).await.is_err() {
                return;
            }
            last = Some(now);
        }
        // Wait for something that changes the packs; a room whose history
        // is asked for again changes nothing once its pack was read.
        loop {
            tokio::select! {
                r = updates.recv() => {
                    if matches!(r, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return;
                    }
                    break;
                }
                _ = changed.notified() => break,
                room = rooms.recv() => {
                    let Ok(room) = room else {
                        return;
                    };
                    if fetched.contains_key(&room) {
                        continue;
                    }
                    if let Ok(id) = RoomId::parse(&room) {
                        let content = fetch_unstable(&client, &id).await;
                        fetched.insert(room, content);
                    }
                    break;
                }
            }
        }
        // Coalesce a burst of updates into one snapshot.
        super::rt::sleep(Duration::from_millis(200)).await;
        while updates.try_recv().is_ok() {}
    }
}

/// Use a room's pack in every room, or stop, under both names so Cinny and
/// FluffyChat agree.
pub async fn use_everywhere(
    client: &Client,
    room: &str,
    key: &str,
    on: bool,
) -> Result<(), Failure> {
    for kind in [PACK_ROOMS, PACK_ROOMS_UNSTABLE] {
        let mut content = account(client, kind)
            .await
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if !content["rooms"].is_object() {
            content["rooms"] = json!({});
        }
        let rooms = content["rooms"].as_object_mut().expect("an object");
        if on {
            let keys = rooms.entry(room.to_owned()).or_insert_with(|| json!({}));
            if !keys.is_object() {
                *keys = json!({});
            }
            keys[key] = json!({});
        } else if let Some(keys) = rooms.get_mut(room).and_then(Value::as_object_mut) {
            keys.remove(key);
            if keys.is_empty() {
                rooms.remove(room);
            }
        }
        let raw = Raw::new(&content)
            .map_err(|_| Failure::Failed)?
            .cast_unchecked();
        client
            .account()
            .set_account_data_raw(GlobalAccountDataEventType::from(kind), raw)
            .await
            .map_err(|_| Failure::Failed)?;
    }
    Ok(())
}

/// A sticker from a pack, as `m.sticker`.
pub fn sticker(image: &Image) -> Result<AnyMessageLikeEventContent, Failure> {
    let info: ImageInfo = image
        .info
        .as_deref()
        .and_then(|i| serde_json::from_str(i).ok())
        .unwrap_or_default();
    let body = image
        .body
        .clone()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| image.shortcode.clone());
    Ok(StickerEventContent::new(body, info, OwnedMxcUri::from(image.url.as_str())).into())
}

/// A reaction with a custom emoji: its mxc URI is the key, and the
/// shortcode goes alongside (MSC4027), so clients without image packs can
/// name it.
pub fn reaction(
    event: OwnedEventId,
    url: &str,
    shortcode: &str,
) -> Raw<AnyMessageLikeEventContent> {
    let shortcode = format!(":{}:", shortcode.trim_matches(':'));
    Raw::new(&json!({
        "m.relates_to": {"rel_type": "m.annotation", "event_id": event, "key": url},
        "shortcode": shortcode,
        "com.beeper.reaction.shortcode": shortcode,
    }))
    .expect("a JSON object")
    .cast_unchecked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_reads_under_either_name_with_usage_from_image_or_pack() {
        let content = json!({
            "pack": {"display_name": "Blobs", "usage": ["emoticon"]},
            "images": {
                "blobwave": {"url": "mxc://h/wave"},
                "bigblob": {"url": "mxc://h/big", "usage": ["sticker"], "body": "A big blob",
                            "info": {"w": 256, "h": 256, "mimetype": "image/png"}},
                "bad": {"url": "https://h/x"},
                "with space": {"url": "mxc://h/s"},
            }
        });
        let pack = parse(&content, Some("!space:h"), "", true).unwrap();
        assert_eq!(pack.name, "Blobs");
        assert_eq!(pack.room.as_deref(), Some("!space:h"));
        let codes: Vec<_> = pack.images.iter().map(|i| i.shortcode.as_str()).collect();
        assert_eq!(codes, ["bigblob", "blobwave"]);
        let big = &pack.images[0];
        assert!(big.sticker && !big.emoticon);
        assert_eq!(big.body.as_deref(), Some("A big blob"));
        let wave = &pack.images[1];
        assert!(wave.emoticon && !wave.sticker);
        // No usage anywhere: both.
        let plain = parse(
            &json!({"images": {"a": {"url": "mxc://h/a"}}}),
            None,
            "",
            false,
        )
        .unwrap();
        assert!(plain.images[0].emoticon && plain.images[0].sticker);
    }

    #[test]
    fn rooms_used_everywhere_list_each_pack() {
        let content = json!({"rooms": {"!a:h": {"": {}, "cats": {}}, "!b:h": {"": {}}}});
        let used = everywhere(&content);
        assert_eq!(used.len(), 3);
        assert!(used.contains(&("!a:h".into(), "cats".into())));
    }

    #[test]
    fn a_sticker_and_a_custom_reaction_read_as_other_clients_expect() {
        let image = Image {
            shortcode: "bigblob".into(),
            url: "mxc://h/big".into(),
            body: None,
            info: Some(r#"{"w":256,"h":256,"mimetype":"image/png"}"#.into()),
            emoticon: false,
            sticker: true,
        };
        let json = serde_json::to_value(sticker(&image).unwrap()).unwrap();
        assert_eq!(json["url"], "mxc://h/big");
        assert_eq!(json["body"], "bigblob");
        assert_eq!(json["info"]["w"], 256);
        let event = OwnedEventId::try_from("$e:h").unwrap();
        let reaction = reaction(event, "mxc://h/wave", "blobwave")
            .deserialize_as::<Value>()
            .unwrap();
        assert_eq!(reaction["m.relates_to"]["key"], "mxc://h/wave");
        assert_eq!(reaction["shortcode"], ":blobwave:");
    }
}
