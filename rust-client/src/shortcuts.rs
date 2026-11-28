//! Discord's keyboard shortcuts (ROADMAP N9): Alt+Shift+↓ and ↑ open the next
//! and previous unread channel, and with Ctrl the next and previous one with
//! mentions; Shift+Escape marks the open server read; Ctrl+/ lists every
//! shortcut. They work while typing too, as Ctrl+K does.
//!
//! "Next" follows what's on screen: down the open server's channel list (text
//! channels, then direct messages), then on through the servers below it in
//! the rail, around to the top and back. Voice channels are skipped, because
//! opening one joins its call.
use crate::{App, Msg, Section, context_menu, switcher};
use iced::{
    Subscription, Task,
    keyboard::{self, Key, key::Named},
};

#[derive(Default)]
pub struct Shortcuts {
    /// Ctrl+/'s list is on screen.
    pub open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Alt+Shift+↓ (forward) or ↑; with Ctrl, only rooms with mentions.
    Unread {
        forward: bool,
        mentions: bool,
    },
    /// Shift+Escape.
    MarkServerRead,
    /// Ctrl+/ shows the list, and hides it again.
    Toggle,
    Close,
}

/// The shortcuts' keys, also while a text field has focus.
pub fn subscription() -> Subscription<Msg> {
    iced::event::listen_with(|event, _, _| {
        let iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            physical_key,
            ..
        }) = event
        else {
            return None;
        };
        let input = match key.as_ref() {
            Key::Named(arrow @ (Named::ArrowDown | Named::ArrowUp))
                if modifiers.alt() && modifiers.shift() =>
            {
                Input::Unread {
                    forward: arrow == Named::ArrowDown,
                    mentions: modifiers.control(),
                }
            }
            Key::Named(Named::Escape)
                if modifiers.shift() && !modifiers.alt() && !modifiers.control() =>
            {
                Input::MarkServerRead
            }
            // Some layouts type / with Shift; the key in /'s place counts too.
            Key::Character("/") if modifiers.command() => Input::Toggle,
            _ if modifiers.command()
                && physical_key == keyboard::key::Physical::Code(keyboard::key::Code::Slash) =>
            {
                Input::Toggle
            }
            _ => return None,
        };
        Some(Msg::Shortcut(input))
    })
}

/// One shortcut in Ctrl+/'s list: what it does, and its keys. Keys are
/// joined with `+`; a row with several has each way of doing it.
pub type Row = (&'static str, &'static [&'static str]);

/// What Ctrl+/ lists, in headed groups.
pub fn list() -> Vec<(&'static str, Vec<Row>)> {
    vec![
        (
            "NAVIGATION",
            vec![
                ("Find or start a conversation", &["Ctrl+K"][..]),
                (
                    "Next or previous unread channel",
                    &["Alt+Shift+↓", "Alt+Shift+↑"],
                ),
                (
                    "Next or previous channel with mentions",
                    &["Ctrl+Alt+Shift+↓", "Ctrl+Alt+Shift+↑"],
                ),
                ("Mark server read", &["Shift+Esc"]),
                ("Scroll the messages", &["PgUp", "PgDn"]),
                ("Close a menu or dialog", &["Esc"]),
                ("Open a context menu", &["Shift+F10", "Menu"]),
                ("Move between controls", &["Tab", "Shift+Tab"]),
                ("Show these shortcuts", &["Ctrl+/"]),
            ],
        ),
        (
            "MESSAGES",
            vec![
                ("Send the message", &["Enter"][..]),
                ("New line", &["Shift+Enter"]),
            ],
        ),
        (
            "MEDIA VIEWER",
            vec![
                ("Previous or next picture", &["←", "→"][..]),
                #[cfg(target_os = "linux")]
                ("Play or pause a video", &["Space"]),
            ],
        ),
        #[cfg(target_os = "linux")]
        (
            "CALLS",
            vec![
                ("Push to talk, in this window", &["Right Ctrl"][..]),
                ("Toggle mute, anywhere", &["Ctrl+Shift+M"]),
                ("Toggle deafen, anywhere", &["Ctrl+Shift+D"]),
            ],
        ),
    ]
}

/// The next room `wanted` picks, going through `sections` (the rail's, each
/// with its rooms in sidebar order) from `selected` in the section at
/// `current`: down the rest of it, on through the sections after it,
/// around, and back down to `selected`. Backward, the same upward.
pub fn next_room<'a>(
    sections: &[Vec<&'a str>],
    current: usize,
    selected: &str,
    forward: bool,
    wanted: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    let here = sections.get(current)?;
    let at = here.iter().position(|id| *id == selected);
    let n = sections.len();
    let (after, before) = match at {
        Some(at) => (&here[at + 1..], &here[..at]),
        None if forward => (&here[..], &here[..0]),
        None => (&here[..0], &here[..]),
    };
    let others = (1..n).map(|k| {
        if forward {
            (current + k) % n
        } else {
            (current + n - k) % n
        }
    });
    let mut order: Vec<&'a str> = Vec::new();
    if forward {
        order.extend(after);
        for i in others {
            order.extend(&sections[i]);
        }
        order.extend(before);
    } else {
        order.extend(before.iter().rev());
        for i in others {
            order.extend(sections[i].iter().rev());
        }
        order.extend(after.iter().rev());
    }
    order.into_iter().find(|id| *id != selected && wanted(id))
}

impl App {
    pub fn shortcut(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Toggle => {
                self.shortcuts.open = !self.shortcuts.open;
                if self.shortcuts.open {
                    self.context_menu.dismiss();
                    self.server_menu = false;
                    if self.switcher.is_some() {
                        self.switcher = None;
                    }
                }
            }
            Input::Close => self.shortcuts.open = false,
            Input::Unread { forward, mentions } => {
                self.shortcuts.open = false;
                if let Some(id) = self.next_unread(forward, mentions) {
                    // Opens it where the sidebar shows it, as Ctrl+K does.
                    return self.switcher(switcher::Input::Go(Some(id)));
                }
            }
            Input::MarkServerRead => return self.mark_section_read(),
        }
        Task::none()
    }

    /// The rail's sections, top to bottom, folders' servers included
    /// (rail_order.rs).
    fn rail(&self) -> Vec<Section> {
        [Section::Direct, Section::All]
            .into_iter()
            .chain(
                crate::rail_order::flat(&self.rail_entries())
                    .into_iter()
                    .map(Section::Space),
            )
            .collect()
    }

    /// A section's rooms as its channel list shows them: text channels,
    /// then direct messages. Voice channels are left out.
    fn sidebar_rooms(&self, section: &Section) -> Vec<&str> {
        let rooms = self.rooms();
        let ids: &[String] = match section {
            Section::All => &rooms.ordered_rooms,
            Section::Direct => &rooms.ordered_direct,
            Section::Space(space) => rooms.ordered_spaces.get(space).map_or(&[], Vec::as_slice),
        };
        let listed = || {
            ids.iter()
                .filter_map(|id| Some((id.as_str(), rooms.list.get(id)?)))
                .filter(|(_, r)| !r.space && !r.voice)
        };
        listed()
            .filter(|(_, r)| !r.dm)
            .chain(listed().filter(|(_, r)| r.dm))
            .map(|(id, _)| id)
            .collect()
    }

    fn next_unread(&self, forward: bool, mentions: bool) -> Option<String> {
        if !self.logged_in || !self.verified {
            return None;
        }
        let rail = self.rail();
        let current = rail.iter().position(|s| *s == self.section)?;
        let sections: Vec<Vec<&str>> = rail.iter().map(|s| self.sidebar_rooms(s)).collect();
        let list = &self.rooms().list;
        next_room(&sections, current, &self.selected, forward, |id| {
            list.get(id).is_some_and(|r| {
                // Discord counts every unread direct message as a mention.
                r.unread && (!mentions || r.dm || r.read.highlights > 0)
            })
        })
        .map(str::to_owned)
    }

    /// Mark every room the open server's icon counts read, through the same
    /// paths as each room's Mark as Read. Only the open room's timeline is
    /// live: another room's newest event the App saw may be old by now (its
    /// timeline closed when you left it), so the engine marks those at the
    /// newest event it has.
    fn mark_section_read(&mut self) -> Task<Msg> {
        if !self.logged_in || !self.verified || !self.connected {
            return Task::none();
        }
        let rooms = &self.live;
        let ids: Vec<&String> = match &self.section {
            Section::Direct => rooms.ordered_direct.iter().collect(),
            Section::All => rooms
                .list
                .iter()
                .filter(|(_, r)| !r.space && !r.dm)
                .map(|(id, _)| id)
                .collect(),
            Section::Space(space) => rooms
                .channels(space)
                .into_keys()
                .filter_map(|id| rooms.list.get_key_value(id).map(|(id, _)| id))
                .collect(),
        };
        let live = |id: &str| id == self.selected && !self.released.contains(id);
        let unread: Vec<(String, Option<String>)> = ids
            .into_iter()
            .filter_map(|id| {
                let r = rooms.list.get(id)?;
                let target = r.read.target().filter(|_| live(id)).map(str::to_owned);
                (r.unread && !r.space).then(|| (id.clone(), target))
            })
            .collect();
        let mut tasks = Vec::new();
        for (room, target) in unread {
            if self.writing_read(&room) {
                continue;
            }
            match target {
                Some(target) => self.mark_room(room, Some(target)),
                None => tasks
                    .push(self.update(Msg::ContextMenu(context_menu::Event::MarkLatestRead(room)))),
            }
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, RoomList, RoomSummary};

    #[test]
    fn next_goes_down_the_list_then_through_later_sections_and_around() {
        let sections = vec![vec!["dm1", "dm2"], vec!["a", "b", "c"], vec!["x", "y"]];
        let unread = |set: &'static [&'static str]| move |id: &str| set.contains(&id);
        // Down the open section first.
        assert_eq!(
            next_room(&sections, 1, "a", true, unread(&["c", "x"])),
            Some("c")
        );
        // Then the sections below it, and around to the top.
        assert_eq!(
            next_room(&sections, 1, "c", true, unread(&["x"])),
            Some("x")
        );
        assert_eq!(
            next_room(&sections, 1, "c", true, unread(&["dm2"])),
            Some("dm2")
        );
        // And finally back above the open room in its own section.
        assert_eq!(
            next_room(&sections, 1, "b", true, unread(&["a"])),
            Some("a")
        );
        // Upward, the same in reverse.
        assert_eq!(
            next_room(&sections, 1, "c", false, unread(&["a", "b"])),
            Some("b")
        );
        assert_eq!(
            next_room(&sections, 1, "a", false, unread(&["dm1", "dm2"])),
            Some("dm2")
        );
        assert_eq!(
            next_room(&sections, 1, "a", false, unread(&["y", "dm1"])),
            Some("dm1")
        );
        assert_eq!(
            next_room(&sections, 1, "a", false, unread(&["y"])),
            Some("y")
        );
        // The open room never counts, and nothing unread means nowhere to go.
        assert_eq!(next_room(&sections, 1, "b", true, unread(&["b"])), None);
        assert_eq!(next_room(&sections, 1, "b", true, unread(&[])), None);
        // With the open room elsewhere, start at the section's top (or bottom).
        assert_eq!(
            next_room(&sections, 2, "a", true, unread(&["y", "c"])),
            Some("y")
        );
        assert_eq!(
            next_room(&sections, 2, "a", false, unread(&["x", "c"])),
            Some("x")
        );
    }

    fn app() -> App {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, _rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let unread = |id: &str, name: &str| RoomSummary {
            unread_messages: 1,
            ..room(id, name)
        };
        let mut children = std::collections::BTreeMap::new();
        children.insert(
            "!s1:h".to_owned(),
            ["!alpha:h", "!beta:h", "!lounge:h", "!gamma:h"]
                .map(String::from)
                .into(),
        );
        children.insert("!s2:h".to_owned(), ["!delta:h"].map(String::from).into());
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                room("!alpha:h", "alpha"),
                unread("!beta:h", "beta"),
                RoomSummary {
                    voice: true,
                    ..unread("!lounge:h", "lounge")
                },
                RoomSummary {
                    unread_mentions: 1,
                    ..unread("!gamma:h", "gamma")
                },
                unread("!delta:h", "delta"),
                RoomSummary {
                    space: true,
                    ..room("!s1:h", "First")
                },
                RoomSummary {
                    space: true,
                    ..room("!s2:h", "Second")
                },
                RoomSummary {
                    dm: true,
                    dm_peer: Some("@mira:h".into()),
                    ..unread("!dm:h", "Mira")
                },
            ],
            children,
        }));
        app
    }

    #[test]
    fn alt_shift_down_opens_unread_channels_in_sidebar_order_skipping_voice() {
        let mut app = app();
        app.section = Section::Space("!s1:h".into());
        let _ = app.update(Msg::Open("!alpha:h".into()));
        let down = Msg::Shortcut(Input::Unread {
            forward: true,
            mentions: false,
        });
        let _ = app.update(down.clone());
        assert_eq!(app.selected, "!beta:h");
        // The unread voice channel is skipped: opening it would join it.
        let _ = app.update(down.clone());
        assert_eq!(app.selected, "!gamma:h");
        // On to the next server in the rail.
        let _ = app.update(down.clone());
        assert_eq!(app.selected, "!delta:h");
        assert!(app.section == Section::Space("!s2:h".into()));
        // Around to the top of the rail: direct messages.
        let _ = app.update(down);
        assert_eq!(app.selected, "!dm:h");
        assert!(app.section == Section::Direct);
        // With Ctrl, only mentions (a DM counts as one).
        let mention_up = Msg::Shortcut(Input::Unread {
            forward: false,
            mentions: true,
        });
        let _ = app.update(mention_up.clone());
        assert_eq!(app.selected, "!gamma:h");
        let _ = app.update(mention_up);
        assert_eq!(app.selected, "!dm:h");
    }

    #[test]
    fn ctrl_slash_toggles_the_list_and_escape_closes_it() {
        let mut app = app();
        let _ = app.update(Msg::Shortcut(Input::Toggle));
        assert!(app.shortcuts.open);
        let _ = app.update(Msg::Shortcut(Input::Toggle));
        assert!(!app.shortcuts.open);
        let _ = app.update(Msg::Shortcut(Input::Toggle));
        app.account = true;
        let _ = app.update(Msg::Escape);
        assert!(!app.shortcuts.open);
        assert!(app.account, "Escape closed only the list");
        assert!(list().iter().all(|(_, rows)| !rows.is_empty()));
    }

    #[test]
    fn shift_escape_marks_every_unread_room_of_the_open_server() {
        let (tx, commands) = async_channel::bounded(64);
        let mut app = app();
        app.sdk = Some(tx);
        app.section = Section::Space("!s1:h".into());
        let _ = app.update(Msg::Open("!gamma:h".into()));
        // gamma's timeline is open; beta's closed when it was left, so the
        // newest event the App saw there may be old.
        app.live
            .list
            .get_mut("!gamma:h")
            .unwrap()
            .read
            .set_latest(Some("$g".into()));
        app.live
            .list
            .get_mut("!beta:h")
            .unwrap()
            .read
            .set_latest(Some("$old".into()));
        app.released.insert("!beta:h".into());
        while commands.try_recv().is_ok() {}
        let _ = app.update(Msg::Shortcut(Input::MarkServerRead));
        let (mut latest, mut at) = (std::collections::BTreeSet::new(), Vec::new());
        while let Ok(command) = commands.try_recv() {
            match command {
                crate::engine::Command::MarkRoomRead { room, .. } => {
                    latest.insert(room);
                }
                crate::engine::Command::MarkRead { room, event, .. } => at.push((room, event)),
                _ => {}
            }
        }
        // beta and the voice channel at the engine's newest event, gamma at
        // its open timeline's; not delta in the other server or the DM.
        assert_eq!(latest, ["!beta:h", "!lounge:h"].map(String::from).into());
        assert_eq!(at, [("!gamma:h".to_owned(), "$g".to_owned())]);
    }
}
