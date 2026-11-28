//! The server rail's order and folders (ROADMAP N8; the App's side is
//! rail_order.rs).
//!
//! The order is Element's, MSC3230: each space's room account data
//! `org.matrix.msc3230.space_order` holds `{"order": "..."}`, read with the
//! room list (rooms.rs). Folders are Untethered's own: one global account
//! data event lists them, so other clients keep showing plain ordered
//! spaces.
use super::{
    Event, Failure,
    messaging::{http, room},
};
use async_channel::Sender;
use matrix_sdk::{
    Client,
    ruma::{
        events::{AnyGlobalAccountDataEvent, GlobalAccountDataEventType, RoomAccountDataEventType},
        serde::Raw,
    },
};
use serde_json::{Value, json};

/// MSC3230's unstable name, as Element reads and writes it.
pub const SPACE_ORDER: &str = "org.matrix.msc3230.space_order";
/// Our folders: `{"folders": [{"id", "name", "color": "#rrggbb", "spaces": [room ID]}]}`.
pub const FOLDERS: &str = "io.github.koalexbear.untethered.space_folders";

/// A folder of servers in the rail.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Folder {
    pub id: String,
    /// Empty: the rail names it by its servers.
    pub name: String,
    /// 0xRRGGBB.
    pub color: u32,
    /// The spaces in it. The rail shows them in the spaces' own order.
    pub spaces: Vec<String>,
}

/// Discord's default folder colour.
pub const DEFAULT_COLOR: u32 = 0x5865f2;

/// The folders in our account data's content. Entries without an ID or
/// spaces are skipped, a space listed twice stays in the first folder,
/// and an unreadable colour is the default.
pub fn folders(content: &Value) -> Vec<Folder> {
    let mut seen = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    crate::rooms::items(&content["folders"])
        .filter_map(|f| {
            let id = f["id"].as_str().filter(|id| !id.is_empty())?;
            if !ids.insert(id.to_owned()) {
                return None;
            }
            let spaces: Vec<String> = crate::rooms::items(&f["spaces"])
                .filter_map(Value::as_str)
                .filter(|s| seen.insert(s.to_string()))
                .map(str::to_owned)
                .collect();
            (!spaces.is_empty()).then(|| Folder {
                id: id.to_owned(),
                name: f["name"].as_str().unwrap_or_default().trim().to_owned(),
                color: f["color"]
                    .as_str()
                    .and_then(|c| u32::from_str_radix(c.strip_prefix('#')?, 16).ok())
                    .filter(|c| *c <= 0xffffff)
                    .unwrap_or(DEFAULT_COLOR),
                spaces,
            })
        })
        .collect()
}

/// Our account data's content for these folders.
pub fn content(folders: &[Folder]) -> Value {
    let list: Vec<Value> = folders
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.name,
                "color": format!("#{:06x}", f.color),
                "spaces": f.spaces,
            })
        })
        .collect();
    json!({ "folders": list })
}

/// Report the folders now and whenever they sync.
pub async fn watch(client: Client, events: Sender<Event>) {
    let kind = GlobalAccountDataEventType::from(FOLDERS);
    let stored = client
        .account()
        .account_data_raw(kind)
        .await
        .ok()
        .flatten()
        .and_then(|raw| raw.deserialize_as::<Value>().ok());
    if let Some(content) = stored {
        let _ = events.send(Event::Folders(folders(&content))).await;
    }
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
        let events = events.clone();
        async move {
            let Ok(value) = raw.deserialize_as::<Value>() else {
                return;
            };
            if value["type"] == FOLDERS {
                let _ = events
                    .send(Event::Folders(folders(&value["content"])))
                    .await;
            }
        }
    });
}

/// Write these spaces' orders, then the folders (if they changed).
pub async fn arrange(
    client: &Client,
    orders: Vec<(String, String)>,
    folders: Option<Vec<Folder>>,
) -> Result<(), Failure> {
    for (space, order) in orders {
        let space = room(client, &space)?;
        let raw = serde_json::value::to_raw_value(&json!({ "order": order }))
            .map_err(|_| Failure::Failed)?;
        space
            .set_account_data_raw(
                RoomAccountDataEventType::from(SPACE_ORDER),
                Raw::from_json(raw),
            )
            .await
            .map_err(|e| http(&e))?;
    }
    if let Some(folders) = folders {
        super::messaging::set_preference(client, FOLDERS, content(&folders)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_read_leniently_and_round_trip() {
        let stored = json!({"folders": [
            {"id": "a", "name": " Games ", "color": "#ff0000", "spaces": ["!x", "!y", 5]},
            // No ID, no spaces, a repeated ID: skipped.
            {"name": "nameless", "spaces": ["!z"]},
            {"id": "b", "spaces": []},
            {"id": "a", "spaces": ["!w"]},
            // !x is in the first folder already; a bad colour is the default.
            {"id": "c", "color": "red", "spaces": ["!x", "!v"]},
        ]});
        let read = folders(&stored);
        assert_eq!(
            read,
            [
                Folder {
                    id: "a".into(),
                    name: "Games".into(),
                    color: 0xff0000,
                    spaces: vec!["!x".into(), "!y".into()],
                },
                Folder {
                    id: "c".into(),
                    name: String::new(),
                    color: DEFAULT_COLOR,
                    spaces: vec!["!v".into()],
                },
            ]
        );
        assert_eq!(folders(&content(&read)), read);
        assert_eq!(content(&read)["folders"][0]["color"], "#ff0000");
        assert!(folders(&json!({})).is_empty());
    }
}
