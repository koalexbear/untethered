//! Server settings (ROADMAP C1): a space's name, icon, banner and topic,
//! its channels' names and topics, and its image packs (MSC2545, in the
//! spec since v1.19).
//!
//! Name, icon and topic are the spec's state events. Matrix has no banner,
//! so it is our own state event ([`BANNER`]) that only Untethered shows.
//! A pack edit reads the pack from the homeserver, changes the one image it
//! names and writes the whole event back, keeping fields it doesn't know:
//! two people editing the same pack lose as little as Matrix allows.
//!
//! Roles (ROADMAP C2, roles.rs) work the same way: custom role names are
//! our own state event on the space ([`ROLES`]), and a power level change
//! is patched onto each room's current `m.room.power_levels`, read from the
//! homeserver just before it's written.
use super::{
    Failure,
    messaging::{http, room},
    packs::{ROOM_PACK, ROOM_PACK_UNSTABLE},
};
use crate::{
    permissions::{POWER, Posting, StateDoc},
    roles::{self, Change, TitleEdit},
};
use matrix_sdk::{
    Client,
    ruma::{
        RoomId,
        api::{
            client::state::{get_state_event_for_key, get_state_events},
            error::ErrorKind,
        },
        events::StateEventType,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A space's banner: `{url, info}` like `m.room.avatar`; `{}` removes it.
pub const BANNER: &str = "io.github.koalexbear.untethered.banner";
/// A space's custom role names and colours (roles.rs):
/// `{"roles": {"<power level>": {"name": …, "color": "#rrggbb"}}}`.
pub const ROLES: &str = "io.github.koalexbear.untethered.roles";
/// Pictures larger than this aren't uploaded.
pub const PICTURE_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// The space's settings from the homeserver, as [`Reply::Loaded`].
    Load {
        space: String,
    },
    /// Only its banner, for the channel list, as [`Reply::Banner`].
    Banner {
        space: String,
    },
    /// Rename a room (the space or a channel).
    Name {
        room: String,
        name: String,
    },
    Topic {
        room: String,
        topic: String,
    },
    /// Upload a picture and make it the room's icon or banner; None removes
    /// it.
    Picture {
        room: String,
        kind: Picture,
        bytes: Option<Vec<u8>>,
    },
    /// Change one image of a pack, or create the pack.
    Pack {
        room: String,
        pack: PackRef,
        edit: Edit,
    },
    /// Upload a picture and add it to a pack.
    AddImage {
        room: String,
        pack: PackRef,
        shortcode: String,
        usage: Usage,
        bytes: Vec<u8>,
    },
    /// The space's custom role names ([`ROLES`]), as [`Reply::Titles`].
    Titles {
        space: String,
    },
    /// Name, rename or remove a custom role: read the event, change the
    /// one role, write it back; the new content as [`Reply::Titles`].
    EditTitles {
        space: String,
        edit: TitleEdit,
    },
    /// Power level changes, one room after another, each made on the
    /// room's current power levels; as [`Reply::Power`].
    Power {
        changes: Vec<(String, Change)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Picture {
    Icon,
    Banner,
}

impl Picture {
    fn event_type(self) -> &'static str {
        match self {
            Self::Icon => "m.room.avatar",
            Self::Banner => BANNER,
        }
    }
}

/// Which pack of a room: the spec's name or the unstable one, and its
/// state key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackRef {
    pub stable: bool,
    pub key: String,
}

impl PackRef {
    pub fn event_type(&self) -> &'static str {
        if self.stable {
            ROOM_PACK
        } else {
            ROOM_PACK_UNSTABLE
        }
    }
}

/// What an image is offered as (MSC2545's `usage`): at least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Usage {
    Emoticon,
    Sticker,
    Both,
}

impl Usage {
    pub fn from_flags(emoticon: bool, sticker: bool) -> Self {
        match (emoticon, sticker) {
            (true, false) => Self::Emoticon,
            (false, true) => Self::Sticker,
            _ => Self::Both,
        }
    }
    fn list(self) -> Value {
        match self {
            Self::Emoticon => json!(["emoticon"]),
            Self::Sticker => json!(["sticker"]),
            Self::Both => json!(["emoticon", "sticker"]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    /// A new, empty pack named `name`; refused if the pack exists.
    Create {
        name: String,
    },
    Add {
        shortcode: String,
        url: String,
        info: Value,
        usage: Usage,
    },
    Rename {
        from: String,
        to: String,
    },
    Usage {
        shortcode: String,
        usage: Usage,
    },
    Delete {
        shortcode: String,
    },
}

/// A space's settings as its state has them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub topic: String,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    /// Its packs under either name, with their content as it is.
    pub packs: Vec<PackDoc>,
    /// Its custom role names ([`ROLES`]) as they are.
    pub roles: Option<Value>,
    /// Who has joined it, by user ID.
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    /// Their display name in the space; empty without one.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackDoc {
    pub pack: PackRef,
    pub content: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    Loaded(Snapshot),
    Banner(Option<String>),
    /// The change's event ID.
    Written(String),
    /// A space's custom role names; None without any.
    Titles(Option<Value>),
    /// Each room's answer: the power levels written and their event ID, or
    /// None where there was nothing to change.
    Power(Vec<(String, PowerWritten)>),
}

/// A room's answer to a power levels change: the content written and its
/// event ID, or None where there was nothing to change.
pub type PowerWritten = Result<Option<(String, Value)>, Failure>;

/// A shortcode Discord would take: 2 to 32 letters, digits, `_` or `-`.
/// MSC2545 only forbids whitespace and `:`; this keeps `:code:` readable
/// in every client.
pub fn valid_shortcode(code: &str) -> bool {
    (2..=32).contains(&code.chars().count())
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Apply `edit` to a pack's current content (None: no such pack). Only
/// what the edit names changes; unknown fields stay.
pub fn apply(content: Option<Value>, edit: &Edit) -> Result<Value, Failure> {
    // A pack whose content was removed (`{}`) can be made again.
    let exists = content
        .as_ref()
        .is_some_and(|c| c.as_object().is_some_and(|o| !o.is_empty()));
    if let Edit::Create { name } = edit {
        if exists {
            return Err(Failure::Failed);
        }
        return Ok(json!({"pack": {"display_name": name}, "images": {}}));
    }
    let mut content = content
        .filter(|_| exists)
        .filter(Value::is_object)
        .ok_or(Failure::Failed)?;
    if !content["images"].is_object() {
        content["images"] = json!({});
    }
    let images = content["images"].as_object_mut().expect("an object");
    match edit {
        Edit::Create { .. } => unreachable!(),
        Edit::Add {
            shortcode,
            url,
            info,
            usage,
        } => {
            if !valid_shortcode(shortcode) || images.contains_key(shortcode) {
                return Err(Failure::Failed);
            }
            let mut image = json!({"url": url, "usage": usage.list()});
            if info.is_object() {
                image["info"] = info.clone();
            }
            images.insert(shortcode.clone(), image);
        }
        Edit::Rename { from, to } => {
            if from == to {
                return Ok(content);
            }
            if !valid_shortcode(to) || images.contains_key(to) {
                return Err(Failure::Failed);
            }
            let image = images.remove(from).ok_or(Failure::Failed)?;
            images.insert(to.clone(), image);
        }
        Edit::Usage { shortcode, usage } => {
            let image = images.get_mut(shortcode).ok_or(Failure::Failed)?;
            if !image.is_object() {
                return Err(Failure::Failed);
            }
            image["usage"] = usage.list();
        }
        Edit::Delete { shortcode } => {
            images.remove(shortcode).ok_or(Failure::Failed)?;
        }
    }
    Ok(content)
}

pub async fn run(client: &Client, request: Request) -> Result<Reply, Failure> {
    match request {
        Request::Load { space } => load(client, &space).await.map(Reply::Loaded),
        Request::Banner { space } => {
            let target = self::room(client, &space)?;
            let content = state(client, &target, BANNER, "").await?;
            Ok(Reply::Banner(content.and_then(|c| {
                c["url"]
                    .as_str()
                    .filter(|u| u.starts_with("mxc://"))
                    .map(str::to_owned)
            })))
        }
        Request::Name { room, name } => {
            let response = self::room(client, &room)?
                .set_name(name)
                .await
                .map_err(|e| http(&e))?;
            Ok(Reply::Written(response.event_id.to_string()))
        }
        Request::Topic { room, topic } => {
            let response = self::room(client, &room)?
                .set_room_topic(&topic)
                .await
                .map_err(|e| http(&e))?;
            Ok(Reply::Written(response.event_id.to_string()))
        }
        Request::Picture { room, kind, bytes } => {
            let target = self::room(client, &room)?;
            let content = match bytes {
                Some(bytes) => {
                    let (url, info) = upload(client, bytes).await?;
                    json!({"url": url, "info": info})
                }
                None => json!({}),
            };
            let response = target
                .send_state_event_raw(kind.event_type(), "", content)
                .await
                .map_err(|e| http(&e))?;
            Ok(Reply::Written(response.event_id.to_string()))
        }
        Request::Pack { room, pack, edit } => write_pack(client, &room, &pack, &edit).await,
        Request::AddImage {
            room,
            pack,
            shortcode,
            usage,
            bytes,
        } => {
            if !valid_shortcode(&shortcode) {
                return Err(Failure::Failed);
            }
            // The room must be known before anything is uploaded.
            self::room(client, &room)?;
            let (url, info) = upload(client, bytes).await?;
            let edit = Edit::Add {
                shortcode,
                url,
                info,
                usage,
            };
            write_pack(client, &room, &pack, &edit).await
        }
        Request::Titles { space } => {
            let target = self::room(client, &space)?;
            Ok(Reply::Titles(state(client, &target, ROLES, "").await?))
        }
        Request::EditTitles { space, edit } => {
            let target = self::room(client, &space)?;
            let current = state(client, &target, ROLES, "").await?;
            let content = roles::apply_titles(current, &edit)?;
            target
                .send_state_event_raw(ROLES, "", content.clone())
                .await
                .map_err(|e| http(&e))?;
            Ok(Reply::Titles(Some(content)))
        }
        Request::Power { changes } => {
            let mut out = Vec::with_capacity(changes.len());
            for (room, change) in changes {
                let result = write_power(client, &room, &change).await;
                out.push((room, result));
            }
            Ok(Reply::Power(out))
        }
    }
}

/// Make `change` on the room's power levels as the homeserver has them
/// now, if this account may: what else is there stays.
async fn write_power(client: &Client, room: &str, change: &Change) -> PowerWritten {
    let me = client.user_id().ok_or(Failure::Unavailable)?.to_string();
    let target = self::room(client, room)?;
    let current = state(client, &target, POWER, "")
        .await?
        .ok_or(Failure::Failed)?;
    // The create event never changes: the store's is the room's.
    let create = super::rooms::state_doc(&target, StateEventType::RoomCreate).await;
    let posting = Posting::new(
        create,
        Some(StateDoc {
            event_id: String::new(),
            sender: String::new(),
            content: current.clone(),
            redacted: false,
        }),
    );
    let content = roles::patch(&posting, &me, change).ok_or(Failure::Forbidden)?;
    if content == current {
        return Ok(None);
    }
    let response = target
        .send_state_event_raw(POWER, "", content.clone())
        .await
        .map_err(|e| http(&e))?;
    Ok(Some((response.event_id.to_string(), content)))
}

/// The space's state from the homeserver (`/state`), not the store: the
/// banner and unstable packs aren't in sliding sync's required state.
async fn load(client: &Client, space: &str) -> Result<Snapshot, Failure> {
    let id = RoomId::parse(space).map_err(|_| Failure::Unavailable)?;
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
    let events = response
        .room_state
        .iter()
        .filter_map(|raw| raw.deserialize_as::<Value>().ok());
    Ok(snapshot(events))
}

/// Read the settings out of a room's state events.
fn snapshot(events: impl Iterator<Item = Value>) -> Snapshot {
    let mut out = Snapshot::default();
    let mxc = |v: &Value| {
        v.as_str()
            .filter(|u| u.starts_with("mxc://"))
            .map(str::to_owned)
    };
    for e in events {
        let (Some(kind), Some(key)) = (e["type"].as_str(), e["state_key"].as_str()) else {
            continue;
        };
        let content = &e["content"];
        match (kind, key) {
            ("m.room.name", "") => {
                out.name = content["name"].as_str().unwrap_or_default().to_owned()
            }
            ("m.room.topic", "") => {
                out.topic = content["topic"].as_str().unwrap_or_default().to_owned()
            }
            ("m.room.avatar", "") => out.avatar = mxc(&content["url"]),
            (BANNER, "") => out.banner = mxc(&content["url"]),
            (ROLES, "") => out.roles = Some(content.clone()),
            ("m.room.member", user) if content["membership"] == "join" => {
                out.members.push(Member {
                    id: user.to_owned(),
                    name: content["displayname"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                })
            }
            (ROOM_PACK | ROOM_PACK_UNSTABLE, key)
                if content.as_object().is_some_and(|o| !o.is_empty()) =>
            {
                out.packs.push(PackDoc {
                    pack: PackRef {
                        stable: kind == ROOM_PACK,
                        key: key.to_owned(),
                    },
                    content: content.clone(),
                })
            }
            _ => {}
        }
    }
    // The spec's packs first, then by state key, the default ("") first.
    out.packs
        .sort_by(|a, b| (!a.pack.stable, &a.pack.key).cmp(&(!b.pack.stable, &b.pack.key)));
    out
}

/// Read the pack, apply `edit`, write it back.
async fn write_pack(
    client: &Client,
    room: &str,
    pack: &PackRef,
    edit: &Edit,
) -> Result<Reply, Failure> {
    let target = self::room(client, room)?;
    let current = state(client, &target, pack.event_type(), &pack.key).await?;
    let content = apply(current, edit)?;
    let response = target
        .send_state_event_raw(pack.event_type(), &pack.key, content)
        .await
        .map_err(|e| http(&e))?;
    Ok(Reply::Written(response.event_id.to_string()))
}

/// A state event's content from the homeserver; None only where it says
/// there is none, so an edit never writes over what it couldn't read.
async fn state(
    client: &Client,
    room: &matrix_sdk::Room,
    kind: &str,
    key: &str,
) -> Result<Option<Value>, Failure> {
    let request = get_state_event_for_key::v3::Request::new(
        room.room_id().to_owned(),
        StateEventType::from(kind),
        key.to_owned(),
    );
    match client.send(request).await {
        Ok(response) => serde_json::from_str::<Value>(response.event_or_content.get())
            .map(Some)
            .map_err(|_| Failure::Failed),
        Err(e) if e.client_api_error_kind() == Some(&ErrorKind::NotFound) => Ok(None),
        Err(e) if e.client_api_error_kind().is_some() => Err(Failure::Forbidden),
        Err(_) => Err(Failure::Unreachable),
    }
}

/// Upload a picture as it is; its mxc URI and `info` (type, size, width,
/// height).
async fn upload(client: &Client, bytes: Vec<u8>) -> Result<(String, Value), Failure> {
    let (mime, (w, h)) = describe(&bytes).ok_or(Failure::Failed)?;
    if bytes.len() as u64 > PICTURE_LIMIT {
        return Err(Failure::Failed);
    }
    let size = bytes.len();
    let response = client
        .media()
        .upload(&mime, bytes, None)
        .await
        .map_err(|e| {
            if e.client_api_error_kind().is_some() {
                Failure::Failed
            } else {
                Failure::Unreachable
            }
        })?;
    let info = json!({"mimetype": mime.essence_str(), "size": size, "w": w, "h": h});
    Ok((response.content_uri.to_string(), info))
}

/// A PNG, JPEG, GIF or WebP picture's type and size.
pub fn describe(bytes: &[u8]) -> Option<(mime::Mime, (u32, u32))> {
    let mime = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => mime::IMAGE_PNG,
        image::ImageFormat::Jpeg => mime::IMAGE_JPEG,
        image::ImageFormat::Gif => mime::IMAGE_GIF,
        image::ImageFormat::WebP => "image/webp".parse().ok()?,
        _ => return None,
    };
    let size = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .filter(|&(w, h)| w > 0 && h > 0)?;
    Some((mime, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs() -> Value {
        json!({
            "pack": {"display_name": "Blobs", "x.custom": 1},
            "images": {
                "wave": {"url": "mxc://h/wave", "body": "Waving", "x.keep": true},
                "big": {"url": "mxc://h/big", "usage": ["sticker"]},
            }
        })
    }

    #[test]
    fn a_pack_is_created_only_where_there_is_none() {
        let made = apply(
            None,
            &Edit::Create {
                name: "Workshop".into(),
            },
        )
        .unwrap();
        assert_eq!(made["pack"]["display_name"], "Workshop");
        assert_eq!(made["images"], json!({}));
        // A pack removed with `{}` can be made again.
        assert!(apply(Some(json!({})), &Edit::Create { name: "W".into() }).is_ok());
        assert!(apply(Some(blobs()), &Edit::Create { name: "W".into() }).is_err());
        // Nothing else edits a pack that isn't there.
        let delete = Edit::Delete {
            shortcode: "wave".into(),
        };
        assert!(apply(None, &delete).is_err());
        assert!(apply(Some(json!({})), &delete).is_err());
    }

    #[test]
    fn an_edit_changes_one_image_and_keeps_unknown_fields() {
        let added = apply(
            Some(blobs()),
            &Edit::Add {
                shortcode: "party_parrot".into(),
                url: "mxc://h/pp".into(),
                info: json!({"w": 64, "h": 64, "mimetype": "image/png", "size": 10}),
                usage: Usage::Emoticon,
            },
        )
        .unwrap();
        assert_eq!(added["images"]["party_parrot"]["url"], "mxc://h/pp");
        assert_eq!(
            added["images"]["party_parrot"]["usage"],
            json!(["emoticon"])
        );
        assert_eq!(added["images"]["party_parrot"]["info"]["w"], 64);
        assert_eq!(added["pack"]["x.custom"], 1);
        // Taken or invalid shortcodes are refused.
        for code in ["wave", "a", "has space", "co:lon"] {
            let edit = Edit::Add {
                shortcode: code.into(),
                url: "mxc://h/x".into(),
                info: Value::Null,
                usage: Usage::Both,
            };
            assert!(apply(Some(blobs()), &edit).is_err(), "{code}");
        }

        let renamed = apply(
            Some(blobs()),
            &Edit::Rename {
                from: "wave".into(),
                to: "hello".into(),
            },
        )
        .unwrap();
        assert!(renamed["images"].get("wave").is_none());
        assert_eq!(renamed["images"]["hello"]["body"], "Waving");
        assert_eq!(renamed["images"]["hello"]["x.keep"], true);
        let onto = Edit::Rename {
            from: "wave".into(),
            to: "big".into(),
        };
        assert!(apply(Some(blobs()), &onto).is_err());

        let both = apply(
            Some(blobs()),
            &Edit::Usage {
                shortcode: "big".into(),
                usage: Usage::Both,
            },
        )
        .unwrap();
        assert_eq!(
            both["images"]["big"]["usage"],
            json!(["emoticon", "sticker"])
        );
        // What the pickers read back.
        let pack = super::super::packs::parse(&both, None, "", true).unwrap();
        let big = pack.images.iter().find(|i| i.shortcode == "big").unwrap();
        assert!(big.emoticon && big.sticker);

        let deleted = apply(
            Some(blobs()),
            &Edit::Delete {
                shortcode: "big".into(),
            },
        )
        .unwrap();
        assert_eq!(deleted["images"].as_object().unwrap().len(), 1);
        let missing = Edit::Delete {
            shortcode: "nope".into(),
        };
        assert!(apply(Some(blobs()), &missing).is_err());
    }

    #[test]
    fn the_snapshot_reads_settings_and_packs_under_either_name() {
        let event = |kind: &str, key: &str, content: Value| json!({"type": kind, "state_key": key, "content": content});
        let snap = snapshot(
            [
                event("m.room.name", "", json!({"name": "Workshop"})),
                event("m.room.topic", "", json!({"topic": "Making things"})),
                event("m.room.avatar", "", json!({"url": "mxc://h/icon"})),
                event(BANNER, "", json!({"url": "mxc://h/banner"})),
                event(ROOM_PACK_UNSTABLE, "", json!({"images": {}})),
                event(ROOM_PACK, "extra", json!({"images": {}})),
                event(ROOM_PACK, "", json!({"images": {}})),
                event(ROOM_PACK, "removed", json!({})),
                event("m.room.member", "@a:h", json!({"membership": "join"})),
                event(
                    "m.room.member",
                    "@b:h",
                    json!({"membership": "join", "displayname": "Bee"}),
                ),
                event("m.room.member", "@c:h", json!({"membership": "leave"})),
                event(ROLES, "", json!({"roles": {"25": {"name": "Helper"}}})),
            ]
            .into_iter(),
        );
        let members: Vec<_> = snap
            .members
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str()))
            .collect();
        assert_eq!(members, [("@a:h", ""), ("@b:h", "Bee")]);
        assert_eq!(snap.roles.unwrap()["roles"]["25"]["name"], "Helper");
        assert_eq!(snap.name, "Workshop");
        assert_eq!(snap.topic, "Making things");
        assert_eq!(snap.avatar.as_deref(), Some("mxc://h/icon"));
        assert_eq!(snap.banner.as_deref(), Some("mxc://h/banner"));
        let packs: Vec<_> = snap
            .packs
            .iter()
            .map(|p| (p.pack.stable, p.pack.key.as_str()))
            .collect();
        assert_eq!(packs, [(true, ""), (true, "extra"), (false, "")]);
        // A removed banner or icon is gone.
        let none = snapshot([event(BANNER, "", json!({}))].into_iter());
        assert_eq!(none.banner, None);
    }

    #[test]
    fn shortcodes_are_discord_shaped() {
        assert!(valid_shortcode("blob_wave-2"));
        assert!(!valid_shortcode("x"));
        assert!(!valid_shortcode(&"a".repeat(33)));
        assert!(!valid_shortcode("blob wave"));
        assert!(!valid_shortcode("é_é"));
    }

    #[test]
    fn pictures_are_described_by_their_bytes() {
        let mut png = Vec::new();
        image::RgbaImage::new(3, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (mime, size) = describe(&png).unwrap();
        assert_eq!((mime.essence_str(), size), ("image/png", (3, 2)));
        assert!(describe(b"not a picture").is_none());
    }
}
