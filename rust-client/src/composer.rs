//! The composer's keys, as on Discord (ROADMAP M13): ↑ in an empty composer
//! edits your last message in the channel, and Escape marks the channel
//! read once nothing else wants it.
//!
//! Escape closes whatever is open first: the main Escape chain in `update`
//! (dialogs, menus, pickers), the autocomplete popup and the inline editor
//! (which take the key themselves), a focused text field. Only an Escape no
//! widget captured reaches [`App::escape_fallback`]; the composer lets it
//! through and keeps its focus, so typing can go on, as on Discord. There
//! it cancels an edit or a reply, and otherwise marks the open channel read.
use crate::{App, Msg, context_menu};
use iced::{
    Task, keyboard,
    widget::text_editor::{self, KeyPress},
};

/// Escape in the focused composer: no binding, so the editor neither
/// captures the key nor gives up its focus.
pub fn lets_escape_through(press: &KeyPress) -> bool {
    press.key == keyboard::Key::Named(keyboard::key::Named::Escape)
        && matches!(press.status, text_editor::Status::Focused { .. })
}

#[derive(Default)]
pub struct State {
    /// While an Escape no widget captured runs the Escape chain.
    free_escape: bool,
}

impl App {
    /// ↑ in the empty, focused composer: edit our newest message here.
    pub fn edit_last_key(&self, press: &KeyPress) -> Option<Msg> {
        let up = press.key == keyboard::Key::Named(keyboard::key::Named::ArrowUp)
            && press.modifiers.is_empty()
            && matches!(press.status, text_editor::Status::Focused { .. });
        if !up || !self.editor.text().trim().is_empty() {
            return None;
        }
        self.last_editable().map(Msg::Edit)
    }

    /// Our newest message in the open channel that we can edit.
    pub fn last_editable(&self) -> Option<String> {
        if !self.connected || !self.verified || self.editing.is_some() {
            return None;
        }
        self.live
            .list
            .get(&self.selected)?
            .posts()
            .rev()
            .find(|p| p.may_edit(&self.matrix_id) && p.from.is_none())?
            .event_id
            .clone()
    }

    /// An Escape that no widget captured: the Escape chain, then
    /// [`Self::escape_fallback`] if nothing in it was open.
    pub fn free_escape(&mut self) -> Task<Msg> {
        self.composer.free_escape = true;
        let task = self.update(Msg::Escape);
        self.composer.free_escape = false;
        task
    }

    /// The end of the Escape chain: true when the key was free for us.
    pub fn escape_is_free(&self) -> bool {
        self.composer.free_escape
    }

    /// Nothing else was open: cancel an edit or a reply here, else mark
    /// the channel read, as Discord's Escape does.
    pub fn escape_fallback(&mut self) -> Task<Msg> {
        let room = self.selected.clone();
        if self
            .editing
            .as_ref()
            .is_some_and(|d| d.room == room && d.pending.is_none())
        {
            self.editing = None;
            return Task::none();
        }
        if self.reply_drafts.remove(&room).is_some() {
            return Task::none();
        }
        self.mark_open_read()
    }

    /// Mark the open channel read up to its newest message.
    fn mark_open_read(&mut self) -> Task<Msg> {
        let room = self.selected.clone();
        let Some(r) = self.live.list.get(&room) else {
            return Task::none();
        };
        if !self.logged_in || !r.unread || r.space || self.writing_read(&room) {
            return Task::none();
        }
        // The open timeline knows its newest event; a released one may not.
        let target = r
            .read
            .target()
            .filter(|_| !self.released.contains(&room))
            .map(str::to_owned);
        match target {
            Some(target) => {
                self.mark_room(room, Some(target));
                Task::none()
            }
            None => self.update(Msg::ContextMenu(context_menu::Event::MarkLatestRead(room))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        Command, Delivery, Event, Kind, Message, RoomList, RoomSummary, TimelineSnapshot,
    };

    const ROOM: &str = "!general:h";

    fn message(row: i64, sender: &str, body: &str) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: sender.into(),
            sender_name: None,
            sender_avatar: None,
            time: row,
            body: body.into(),
            formatted: None,
            kind: Kind::Text,
            encrypted: false,
            delivery: Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: sender == "@you:h",
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
            poll: None,
            emote: false,
        }
    }

    /// Signed in, with #general open and unread: ours, theirs, our action.
    fn app() -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@you:h".into();
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![RoomSummary {
                id: ROOM.into(),
                name: "general".into(),
                unread_messages: 2,
                ..Default::default()
            }],
            children: Default::default(),
        }));
        let _ = app.update(Msg::Open(ROOM.into()));
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: ROOM.into(),
            messages: vec![
                message(1, "@you:h", "first"),
                message(2, "@you:h", "mine"),
                message(3, "@mira:h", "theirs"),
                Message {
                    text: false,
                    emote: true,
                    ..message(4, "@you:h", "waves")
                },
            ],
            start_reached: true,
            latest_event: Some("$4".into()),
            seams: Vec::new(),
        }));
        while rx.try_recv().is_ok() {}
        (app, rx)
    }

    fn marked(rx: &async_channel::Receiver<Command>) -> Vec<String> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|c| match c {
                Command::MarkRead { room, event, .. } => Some(format!("{room} {event}")),
                Command::MarkRoomRead { room, .. } => Some(room),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_free_escape_marks_the_open_channel_read() {
        let (mut app, rx) = app();
        let _ = app.update(Msg::FreeEscape);
        assert_eq!(marked(&rx), [format!("{ROOM} $4")]);
    }

    #[test]
    fn an_escape_a_widget_took_or_that_closed_something_marks_nothing() {
        let (mut app, rx) = app();
        // The popup or the inline editor captured it.
        let _ = app.update(Msg::Escape);
        assert!(marked(&rx).is_empty());
        // A menu was open: it closes, and only that.
        app.composer_menu = true;
        let _ = app.update(Msg::FreeEscape);
        assert!(!app.composer_menu);
        assert!(marked(&rx).is_empty());
        assert!(!app.escape_is_free(), "the flag never outlives the key");
    }

    #[test]
    fn escape_cancels_a_reply_before_marking_read() {
        let (mut app, rx) = app();
        let _ = app.update(Msg::Reply("$3".into()));
        assert!(app.reply_drafts.contains_key(ROOM));
        let _ = app.update(Msg::FreeEscape);
        assert!(!app.reply_drafts.contains_key(ROOM));
        assert!(marked(&rx).is_empty());
        let _ = app.update(Msg::FreeEscape);
        assert_eq!(marked(&rx), [format!("{ROOM} $4")]);
    }

    #[test]
    fn up_edits_our_newest_message_that_can_be_edited() {
        let (mut app, _rx) = app();
        // Not theirs, nor our action (only m.text can be edited here).
        assert_eq!(app.last_editable().as_deref(), Some("$2"));
        app.editor = text_editor::Content::with_text("draft");
        let up = KeyPress {
            key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
            modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowUp),
            modifiers: keyboard::Modifiers::default(),
            text: None,
            status: text_editor::Status::Focused { is_hovered: false },
        };
        assert!(app.edit_last_key(&up).is_none(), "the draft isn't empty");
        app.editor = text_editor::Content::new();
        assert!(matches!(app.edit_last_key(&up), Some(Msg::Edit(id)) if id == "$2"));
        let shifted = KeyPress {
            modifiers: keyboard::Modifiers::SHIFT,
            ..up.clone()
        };
        assert!(app.edit_last_key(&shifted).is_none());
    }
}
