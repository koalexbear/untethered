//! A jump opens a bounded slice of history, without moving the live page cursor.
use crate::{App, Request, rooms::Room};
use serde_json::json;

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
}

#[cfg(test)]
mod tests {}
