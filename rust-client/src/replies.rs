//! A jump opens a bounded slice of history, without moving the live page cursor.
use crate::{
    App, Request,
    rooms::{Room, items},
};
use serde_json::{Value, json};

pub struct Original {
    pub room: String,
    pub event: String,
    pub slice: Option<Room>,
    pub preceding: bool,
}
impl App {
    pub fn visible_post(&self, id: &str) -> Option<&crate::rooms::Post> {
        self.live.list.get(&self.selected)?.by_id(id).or_else(|| {
            self.original
                .as_ref()
                .filter(|c| c.room == self.selected)?
                .slice
                .as_ref()?
                .by_id(id)
        })
    }
    pub fn timeline_id(&self) -> iced::widget::Id {
        self.original
            .as_ref()
            .map_or_else(
                || format!("timeline-{}", self.selected),
                |c| format!("context-{}-{}", c.room, c.event),
            )
            .into()
    }
    pub fn anchor_history(&self) -> iced::Task<crate::Msg> {
        // Both anchors use distance from their respective start: context
        // begins at the target, while the live feed is bottom-anchored.
        iced::widget::operation::scroll_to(
            self.timeline_id(),
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(0.),
            },
        )
    }
    pub fn close_original(&mut self) {
        self.original = None;
        self.reaction_picker = None;
        self.editing = None;
        self.reporting = None;
        self.pending
            .retain(|_, r| !matches!(r, Request::Context(..)));
    }
    pub fn jump(&mut self, id: String) {
        self.close_original();
        self.status.clear();
        let room = self.selected.clone();
        let Some(r) = self.live.list.get(&room) else {
            return;
        };
        if r.by_id(&id).is_some() {
            self.original = Some(Original {
                room,
                event: id,
                slice: None,
                preceding: false,
            });
        } else {
            self.rpc(
                "get_event_context",
                json!({"room_id":room,"event_id":id,"limit":12}),
                Request::Context(room, id),
            );
        }
    }
    pub fn show_context(&mut self, room: String, id: String, data: &Value) {
        if self.selected != room {
            return;
        }
        let event = &data["event"];
        let mut slice = Room::default();
        if event["room_id"] == room && event["event_id"] == id {
            slice.insert(event);
        }
        if slice.by_id(&id).is_none() {
            self.status = "The original message is unavailable in this room.".into();
            return;
        }
        for e in items(&data["before"])
            .chain(items(&data["after"]))
            .filter(|e| e["room_id"] == room)
        {
            slice.insert(e);
        }
        slice.related_events(&room, data);
        self.status.clear();
        self.original = Some(Original {
            room,
            event: id,
            slice: Some(slice),
            preceding: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Msg;
    #[test]
    fn context_must_match_the_requested_event_room_and_current_navigation() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.connected = true;
        app.selected = "!a".into();
        app.live.list.insert("!a".into(), Room::default());
        let good = json!({"room_id":"!a","rowid":1,"event_id":"$first","sender":"@a:h","type":"m.room.message","content":{"body":"duplicate"}});
        for (key, other) in [("event_id", "$second"), ("room_id", "!b")] {
            let mut bad = good.clone();
            bad[key] = json!(other);
            app.show_context("!a".into(), "$first".into(), &json!({"event":bad}));
            assert!(app.original.is_none());
        }
        app.jump("$first".into());
        let seq = rx.try_recv().unwrap().into_rpc()["request_id"].clone();
        let _ = app.update(Msg::Open("!b".into()));
        app.frame(json!({"command":"response","request_id":seq,"data":{"event":good}}));
        assert!(app.original.is_none());
        assert!(app.live.list["!a"].history.is_empty());
    }
    #[test]
    fn reply_and_text_stay_in_their_room_and_survive_a_failed_send() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.selected = "!a".into();
        for id in ["!a", "!b"] {
            app.live.list.insert(id.into(), Room::default());
            app.loaded.insert(id.into());
        }
        let src = json!({"rowid":2,"event_id":"$a","type":"m.room.message","sender":"@peer:h","content":{"body":"original"}});
        app.live.list.get_mut("!a").unwrap().insert(&src);
        let _ = app.update(Msg::Reply("$a".into()));
        let text = "/unencrypted literal text";
        app.editor = iced::widget::text_editor::Content::with_text(text);
        app.open("!b".into());
        assert!(!app.reply_drafts.contains_key("!b") && app.editor.text().is_empty());
        app.open("!a".into());
        let _ = app.update(Msg::Send);
        let req = rx.try_recv().unwrap().into_rpc();
        let data = &req["data"];
        assert_eq!(data["room_id"], "!a");
        assert_eq!(data["relates_to"]["m.in_reply_to"]["event_id"], "$a");
        assert_eq!(data["base_content"]["body"], text);
        assert_eq!(data["text"], "");
        app.frame(json!({"command":"error","request_id":req["request_id"]}));
        assert_eq!(app.reply_drafts["!a"].id, "$a");
        assert_eq!(app.editor.text(), text);
        let _ = app.update(Msg::CancelReply);
        assert!(app.reply_drafts.is_empty());
        assert_eq!(app.editor.text(), text);
    }
}
