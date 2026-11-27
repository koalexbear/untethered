//! Link previews (ROADMAP M7) through the homeserver's `preview_url`, which
//! fetches the page itself and answers with its Open Graph data; the page
//! never learns who asked. Whether a room shows them is stored where Element
//! keeps it, so both clients agree: unencrypted rooms show them unless you or
//! the room turned them off (`org.matrix.room.preview_urls`), encrypted rooms
//! only when you turned them on (`urlPreviewsEnabled_e2ee` in
//! `im.vector.web.settings`), since asking means telling the homeserver the
//! link.
use super::{Failure, LinkPreview};
use matrix_sdk::{
    Client, Room,
    ruma::{
        RoomId,
        events::{GlobalAccountDataEventType, RoomAccountDataEventType, StateEventType},
        serde::Raw,
    },
};
use serde_json::{Value, json};

/// A room's own switch, and Element's room-wide default.
const ROOM_SWITCH: &str = "org.matrix.room.preview_urls";
/// Your default for unencrypted rooms.
const ACCOUNT_SWITCH: &str = "org.matrix.preview_urls";
/// Element's per-room settings; encrypted rooms keep the switch here.
const ELEMENT_SETTINGS: &str = "im.vector.web.settings";
const E2EE_KEY: &str = "urlPreviewsEnabled_e2ee";

/// The page behind `url`, as the homeserver previews it.
pub async fn fetch(client: &Client, url: &str) -> Option<LinkPreview> {
    use matrix_sdk::ruma::api::client::authenticated_media::get_media_preview::v1;
    let data = match client.send(v1::Request::new(url.to_owned())).await {
        Ok(response) => response.data,
        // Homeservers from before authenticated media (Matrix 1.11).
        Err(_) => {
            #[allow(deprecated)]
            use matrix_sdk::ruma::api::client::media::get_media_preview::v3;
            #[allow(deprecated)]
            let response = client.send(v3::Request::new(url.to_owned())).await.ok()?;
            response.data
        }
    }?;
    parse(url, data.get())
}

/// Open Graph data as `preview_url` answers it; None when there's nothing
/// to show.
pub fn parse(url: &str, json: &str) -> Option<LinkPreview> {
    let data: Value = serde_json::from_str(json).ok()?;
    let text = |key: &str| {
        data[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(600).collect::<String>())
    };
    let number = |key: &str| {
        data[key]
            .as_u64()
            .or_else(|| data[key].as_str()?.parse().ok())
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
    };
    let image = text("og:image").filter(|s| s.starts_with("mxc://"));
    let preview = LinkPreview {
        url: url.to_owned(),
        title: text("og:title"),
        description: text("og:description"),
        site: text("og:site_name"),
        image_size: image
            .as_ref()
            .and(number("og:image:width").zip(number("og:image:height"))),
        image,
    };
    (preview.title.is_some() || preview.description.is_some() || preview.image.is_some())
        .then_some(preview)
}

/// Whether `room` shows link previews for this account.
pub async fn enabled(room: &Room) -> bool {
    let account = |kind: &'static str| async move {
        let raw = room
            .account_data(RoomAccountDataEventType::from(kind))
            .await
            .ok()??;
        content(raw.json().get())
    };
    if room.encryption_state().is_encrypted() {
        return account(ELEMENT_SETTINGS)
            .await
            .and_then(|c| c[E2EE_KEY].as_bool())
            .unwrap_or(false);
    }
    if let Some(disable) = account(ROOM_SWITCH)
        .await
        .and_then(|c| c["disable"].as_bool())
    {
        return !disable;
    }
    let room_default = super::rooms::state_doc(room, StateEventType::from(ROOM_SWITCH)).await;
    if room_default.and_then(|d| d.content["disable"].as_bool()) == Some(true) {
        return false;
    }
    let mine = room
        .client()
        .account()
        .account_data_raw(GlobalAccountDataEventType::from(ACCOUNT_SWITCH))
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Value>(raw.json().get()).ok());
    !mine.is_some_and(|c| c["disable"] == true)
}

/// Turn `room`'s link previews on or off, where Element keeps the switch.
pub async fn set(client: &Client, room: &str, on: bool) -> Result<(), Failure> {
    let id = RoomId::parse(room).map_err(|_| Failure::Unavailable)?;
    let room = client.get_room(&id).ok_or(Failure::Unavailable)?;
    let (kind, content) = if room.encryption_state().is_encrypted() {
        // Keep Element's other settings in the same event.
        let mut settings = match room
            .account_data(RoomAccountDataEventType::from(ELEMENT_SETTINGS))
            .await
        {
            Ok(Some(raw)) => content(raw.json().get()).unwrap_or_else(|| json!({})),
            _ => json!({}),
        };
        if !settings.is_object() {
            settings = json!({});
        }
        settings[E2EE_KEY] = json!(on);
        (ELEMENT_SETTINGS, settings)
    } else {
        (ROOM_SWITCH, json!({ "disable": !on }))
    };
    let raw =
        Raw::from_json(serde_json::value::to_raw_value(&content).map_err(|_| Failure::Failed)?);
    room.set_account_data_raw(RoomAccountDataEventType::from(kind), raw)
        .await
        .map(|_| ())
        .map_err(|_| Failure::Unreachable)
}

/// An account data event's content.
fn content(event: &str) -> Option<Value> {
    let event: Value = serde_json::from_str(event).ok()?;
    Some(event.get("content").cloned().unwrap_or(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_data_becomes_a_preview() {
        let p = parse(
            "https://example.org/a",
            r#"{"og:title":" A page ","og:description":"About it","og:site_name":"Example",
                "og:image":"mxc://s/img","og:image:width":1200,"og:image:height":"630",
                "matrix:image:size":12345}"#,
        )
        .unwrap();
        assert_eq!(p.title.as_deref(), Some("A page"));
        assert_eq!(p.site.as_deref(), Some("Example"));
        assert_eq!(p.image.as_deref(), Some("mxc://s/img"));
        assert_eq!(p.image_size, Some((1200, 630)));
    }

    #[test]
    fn an_empty_answer_or_a_web_image_shows_nothing() {
        assert!(parse("https://x", "{}").is_none());
        // Only the homeserver's own copy of an image is shown, never a web URL.
        assert!(parse("https://x", r#"{"og:image":"https://tracker/p.png"}"#).is_none());
    }
}
