//! Right-click menus for messages, people and channels, as on Discord
//! (ROADMAP M4). An entry sends the message its toolbar, panel or footer
//! counterpart already sends wherever one exists, and the hover toolbar is
//! built from the same list, so every toolbar action is also in the menu and
//! the two can't drift apart. The view (`view/context_menu.rs`) draws the
//! menu at the pointer and turns a chosen entry into [`Event::Pick`].
use crate::{
    App, Msg, Request, Section, moderation,
    rooms::{Post, Room},
};
use iced::{Task, Vector};

/// What a menu is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A message in the open room.
    Message {
        room: String,
        event: String,
    },
    /// A person as `room` shows them: an author, a member or a DM peer.
    User {
        room: String,
        user: String,
    },
    Channel(String),
}

/// The menu on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub target: Target,
    /// Which of the places showing `target` was clicked, as the view names
    /// them: a person has a picture and a name on each of their messages and
    /// a row in the member list, and only the clicked one draws the menu.
    pub place: String,
    /// Where it opens, from the top-left corner of what was clicked. None
    /// anchors it to that area instead (Shift+F10 with nothing hovered).
    pub at: Option<Vector>,
    /// Opened from the keyboard, so the first entry starts highlighted.
    pub keyboard: bool,
    /// Tells a menu apart from the next one opened at the same target.
    pub serial: u64,
}

#[derive(Default)]
pub struct State {
    pub open: Option<Open>,
    serial: u64,
    /// The room waiting for Leave to be confirmed.
    pub leaving: Option<String>,
    pub deleting: Option<(String, String)>,
    /// The person waiting for Block to be confirmed (dms.rs).
    pub blocking: Option<String>,
}
impl State {
    /// Escape closes the menu or the leave dialog before anything beneath.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
            || self.leaving.is_some()
            || self.deleting.is_some()
            || self.blocking.is_some()
    }
    pub fn dismiss(&mut self) {
        self.open = None;
        self.leaving = None;
        self.deleting = None;
        self.blocking = None;
    }
}

#[derive(Clone)]
pub enum Event {
    /// Right click, or Shift+F10 / the Menu key, over a menu area.
    Open {
        target: Target,
        place: String,
        at: Option<Vector>,
        keyboard: bool,
    },
    /// Shift+F10 or the Menu key with nothing hovered: the newest message.
    Keyboard,
    Close,
    /// An entry was chosen: close the menu, then do what it says.
    Pick(Box<Msg>),
    // The entries with no counterpart elsewhere in the app. They name what
    // to copy, and read it when chosen: the toolbar builds every row's list.
    Copy(String),
    /// A message's text, as it reads when chosen.
    CopyText(String),
    /// A matrix.to link to a room or one of its messages.
    CopyLink {
        room: String,
        event: Option<String>,
    },
    Profile {
        room: String,
        user: String,
    },
    MemberActions {
        room: String,
        user: String,
    },
    Mention {
        room: String,
        user: String,
    },
    /// Open an existing direct message room.
    Direct(String),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    Alerts(String),
    /// Ask before leaving a room.
    Leave(String),
    ConfirmLeave,
    Delete {
        room: String,
        event: String,
    },
    ConfirmDelete,
    /// Ask before blocking someone; unblocking doesn't ask.
    Block(String),
    ConfirmBlock,
    Unblock(String),
    /// Mark a room read whose timeline isn't open, so its newest event is
    /// unknown here: the engine finds it among what sync delivered.
    MarkLatestRead(String),
}

/// One entry. Its label, icon and position come from the view; its order in
/// the list is the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    AddReaction,
    Reply,
    Edit,
    CopyText,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    CopyImage,
    MarkUnread,
    CopyMessageLink,
    Delete,
    Report,
    Profile,
    Mention,
    Message,
    Kick,
    Ban,
    Block,
    Unblock,
    CopyUserId,
    MarkRead,
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    Notifications,
    /// The room's link preview switch (link_previews.rs), as it would turn.
    ShowLinkPreviews,
    HideLinkPreviews,
    CopyLink,
    LeaveChannel,
    LeaveConversation,
}
impl Entry {
    pub fn label(self) -> &'static str {
        match self {
            Self::AddReaction => "Add Reaction",
            Self::Reply => "Reply",
            Self::Edit => "Edit Message",
            Self::CopyText => "Copy Text",
            Self::CopyImage => "Copy Image",
            Self::MarkUnread => "Mark Unread",
            Self::CopyMessageLink => "Copy Message Link",
            Self::Report => "Report Message",
            Self::Delete => "Delete Message…",
            Self::Profile => "Profile",
            Self::Mention => "Mention",
            Self::Message => "Message",
            // Both open the member card, where a reason goes with the action.
            Self::Kick => "Kick…",
            Self::Ban => "Ban…",
            Self::Block => "Block…",
            Self::Unblock => "Unblock",
            Self::CopyUserId => "Copy User ID",
            Self::MarkRead => "Mark as Read",
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Self::Notifications => "Notification Settings",
            Self::ShowLinkPreviews => "Show Link Previews",
            Self::HideLinkPreviews => "Hide Link Previews",
            Self::CopyLink => "Copy Link",
            Self::LeaveChannel => "Leave Channel",
            Self::LeaveConversation => "Leave Conversation",
        }
    }
    /// The hover toolbar's shorter tooltip; None for entries it doesn't show.
    pub fn toolbar_hint(self) -> Option<&'static str> {
        match self {
            Self::AddReaction => Some("Add Reaction"),
            Self::Reply => Some("Reply"),
            Self::Edit => Some("Edit"),
            Self::Report => Some("Report Message"),
            _ => None,
        }
    }
    /// Entries of one group sit together, with a rule between groups.
    pub fn group(self) -> u8 {
        match self {
            Self::AddReaction | Self::Profile | Self::Mention | Self::Message | Self::MarkRead => 0,
            Self::Reply
            | Self::Edit
            | Self::CopyText
            | Self::CopyImage
            | Self::MarkUnread
            | Self::CopyMessageLink
            | Self::Kick
            | Self::Ban => 1,
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Self::Notifications => 1,
            Self::ShowLinkPreviews | Self::HideLinkPreviews => 1,
            Self::Block | Self::Unblock => 2,
            Self::Report | Self::CopyUserId | Self::CopyLink => 2,
            Self::Delete | Self::LeaveChannel | Self::LeaveConversation => 3,
        }
    }
    /// Red, as Discord marks what affects someone else or can't be undone.
    pub fn danger(self) -> bool {
        matches!(
            self,
            Self::Report
                | Self::Delete
                | Self::Kick
                | Self::Ban
                | Self::Block
                | Self::LeaveChannel
                | Self::LeaveConversation
        )
    }
}

#[derive(Clone)]
pub struct Item {
    pub entry: Entry,
    /// None shows the entry disabled.
    pub msg: Option<Msg>,
}
fn item(entry: Entry, msg: Option<Msg>) -> Item {
    Item { entry, msg }
}
fn act(e: Event) -> Msg {
    Msg::ContextMenu(e)
}

/// A matrix.to permalink, as Element builds them: the room ID, the event ID
/// if any, and the servers to join through.
pub fn matrix_to(room: &str, event: Option<&str>, me: &str) -> String {
    fn escape(id: &str) -> String {
        let mut out = String::with_capacity(id.len());
        for c in id.chars() {
            if c.is_ascii_alphanumeric() || "!$:@+-._~=".contains(c) {
                out.push(c);
            } else {
                let mut buf = [0; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
        out
    }
    let mut link = format!("https://matrix.to/#/{}", escape(room));
    if let Some(event) = event {
        link.push('/');
        link.push_str(&escape(event));
    }
    // Version 12 room IDs name no server, so also offer our own.
    let mut via: Vec<&str> = Vec::new();
    for server in [room.split_once(':'), me.split_once(':')]
        .into_iter()
        .flatten()
        .map(|(_, s)| s)
    {
        if !server.is_empty() && !via.contains(&server) {
            via.push(server);
        }
    }
    for (i, server) in via.iter().enumerate() {
        link.push(if i == 0 { '?' } else { '&' });
        link.push_str("via=");
        link.push_str(&escape(server));
    }
    link
}

/// Put `text` on the clipboard.
pub fn copy(text: String) -> Task<Msg> {
    #[cfg(not(target_arch = "wasm32"))]
    return iced::clipboard::write(text);
    #[cfg(target_arch = "wasm32")]
    Task::perform(
        wasm_bindgen_futures::JsFuture::from(
            web_sys::window()
                .unwrap()
                .navigator()
                .clipboard()
                .write_text(&text),
        ),
        |r| Msg::ClipboardResult(r.is_ok()),
    )
}

impl App {
    /// What a message's menu offers, in order. The hover toolbar shows the
    /// entries that have a [`Entry::toolbar_hint`].
    pub fn message_items(&self, room: &str, p: &Post) -> Vec<Item> {
        let mut items = Vec::new();
        let (Some(r), Some(id)) = (self.live.list.get(room), p.event_id.as_ref()) else {
            return items;
        };
        if !self.logged_in || room != self.selected {
            return items;
        }
        let ready = self.verified && self.connected;
        if p.reactable {
            items.push(item(
                Entry::AddReaction,
                ready.then(|| Msg::ReactionPicker(id.clone())),
            ));
        }
        // An older room's message (upgrades.rs) can't be replied to from here.
        if p.from.is_none() {
            items.push(item(Entry::Reply, Some(Msg::Reply(id.clone()))));
        }
        if p.may_edit(&self.matrix_id) && r.has_metadata && r.encrypted == p.encrypted {
            items.push(item(
                Entry::Edit,
                (self.connected && self.editing.is_none()).then(|| Msg::Edit(id.clone())),
            ));
        }
        if (p.reactable || p.from.is_some()) && !p.text().trim().is_empty() {
            items.push(item(
                Entry::CopyText,
                Some(act(Event::CopyText(id.clone()))),
            ));
        }
        // The picture itself, as on Discord (copy_image.rs).
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(media) = p
            .media
            .as_ref()
            .filter(|m| m.kind == crate::media::Kind::Image)
        {
            items.push(item(
                Entry::CopyImage,
                (self.connected && !self.copying_image()).then(|| Msg::CopyImage(media.clone())),
            ));
        }
        // Receipts never move back in Matrix, so this marks the room unread
        // (`m.marked_unread`), as the footer's Mark unread does.
        items.push(item(
            Entry::MarkUnread,
            (ready && !r.unread && !self.writing_read(room))
                .then(|| Msg::MarkRoom(room.to_owned(), None)),
        ));
        items.push(item(
            Entry::CopyMessageLink,
            Some(act(Event::CopyLink {
                room: p.from.clone().unwrap_or_else(|| room.to_owned()),
                event: Some(id.clone()),
            })),
        ));
        if self.verified && p.report_id(&self.matrix_id).is_some() {
            items.push(item(
                Entry::Report,
                (self.connected && self.reporting.is_none() && !self.report_pending())
                    .then(|| Msg::Report(room.to_owned(), id.clone())),
            ));
        }
        if self.verified && self.may_delete(room, p) {
            items.push(item(
                Entry::Delete,
                (ready && !self.deleting_message(room, id)).then(|| {
                    act(Event::Delete {
                        room: room.to_owned(),
                        event: id.clone(),
                    })
                }),
            ));
        }
        items
    }

    fn may_delete(&self, room: &str, post: &Post) -> bool {
        post.from.is_none()
            && post.reactable
            && self
                .live
                .list
                .get(room)
                .is_some_and(|r| r.posting.can_redact_message(&self.matrix_id, &post.sender))
    }

    fn deleting_message(&self, room: &str, event: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Redact(r, e) if r == room && e == event))
    }

    /// What a person's menu offers, as `room` shows them.
    pub fn user_items(&self, room: &str, user: &str) -> Vec<Item> {
        let mut items = Vec::new();
        let Some(r) = self.live.list.get(room) else {
            return items;
        };
        if !self.logged_in || user.is_empty() {
            return items;
        }
        let ready = self.verified && self.connected;
        let me = user == self.matrix_id;
        let profile = || {
            act(Event::Profile {
                room: room.to_owned(),
                user: user.to_owned(),
            })
        };
        items.push(item(Entry::Profile, ready.then(profile)));
        items.push(item(
            Entry::Mention,
            (room == self.selected && self.can_stage()).then(|| {
                act(Event::Mention {
                    room: room.to_owned(),
                    user: user.to_owned(),
                })
            }),
        ));
        if !me && let Some(dm) = self.direct_with(user) {
            items.push(item(Entry::Message, Some(act(Event::Direct(dm.clone())))));
        }
        if !me && ready {
            // The member list knows their membership once it has loaded.
            let membership = (self.people && room == self.selected)
                .then(|| self.directory.users.get(user).map(|m| m.kind))
                .flatten();
            for (entry, action) in [
                (Entry::Kick, moderation::Action::Kick),
                (Entry::Ban, moderation::Action::Ban),
            ] {
                if r.posting.can_moderate(&self.matrix_id, user, action)
                    && membership.is_none_or(|kind| action.applies(kind))
                {
                    items.push(item(
                        entry,
                        Some(act(Event::MemberActions {
                            room: room.to_owned(),
                            user: user.to_owned(),
                        })),
                    ));
                }
            }
        }
        if !me {
            let block = if self.is_blocked(user) {
                (Entry::Unblock, Event::Unblock(user.to_owned()))
            } else {
                (Entry::Block, Event::Block(user.to_owned()))
            };
            items.push(item(
                block.0,
                (ready && !self.blocking(user)).then(|| act(block.1)),
            ));
        }
        items.push(item(
            Entry::CopyUserId,
            Some(act(Event::Copy(user.to_owned()))),
        ));
        items
    }

    /// What a channel's (or DM's) menu in the sidebar offers.
    pub fn channel_items(&self, room: &str) -> Vec<Item> {
        let mut items = Vec::new();
        let Some(r) = self.live.list.get(room) else {
            return items;
        };
        if !self.logged_in {
            return items;
        }
        let ready = self.verified && self.connected;
        items.push(item(
            Entry::MarkRead,
            (ready && r.unread && !self.writing_read(room)).then(|| match r.read.target() {
                Some(target) => Msg::MarkRoom(room.to_owned(), Some(target.to_owned())),
                None => act(Event::MarkLatestRead(room.to_owned())),
            }),
        ));
        if r.dm
            && let Some(peer) = self.dm_peer(r)
        {
            items.push(item(
                Entry::Profile,
                ready.then(|| {
                    act(Event::Profile {
                        room: room.to_owned(),
                        user: peer.to_owned(),
                    })
                }),
            ));
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        items.push(item(
            Entry::Notifications,
            self.verified.then(|| act(Event::Alerts(room.to_owned()))),
        ));
        items.push(item(
            if r.link_previews {
                Entry::HideLinkPreviews
            } else {
                Entry::ShowLinkPreviews
            },
            ready.then(|| Msg::LinkPreviews(room.to_owned(), !r.link_previews)),
        ));
        items.push(item(
            Entry::CopyLink,
            Some(act(Event::CopyLink {
                room: room.to_owned(),
                event: None,
            })),
        ));
        items.push(item(
            if r.dm {
                Entry::LeaveConversation
            } else {
                Entry::LeaveChannel
            },
            (ready && !self.leaving_room(room)).then(|| act(Event::Leave(room.to_owned()))),
        ));
        items
    }

    pub fn menu_items(&self, target: &Target) -> Vec<Item> {
        match target {
            Target::Message { room, event } => self
                .visible_post(event)
                .map(|p| self.message_items(room, p))
                .unwrap_or_default(),
            Target::User { room, user } => self.user_items(room, user),
            Target::Channel(room) => self.channel_items(room),
        }
    }

    /// Use the SDK's direct target even before the room's timeline is loaded.
    fn dm_peer<'a>(&self, r: &'a Room) -> Option<&'a str> {
        if let Some(peer) = &r.dm_peer {
            return Some(peer);
        }
        let mut others = r
            .members
            .keys()
            .map(String::as_str)
            .filter(|user| *user != self.matrix_id);
        let peer = others.next()?;
        others.next().is_none().then_some(peer)
    }

    pub(crate) fn direct_with(&self, user: &str) -> Option<&String> {
        self.live
            .list
            .iter()
            .find(|(_, r)| r.dm && self.dm_peer(r) == Some(user))
            .map(|(id, _)| id)
    }

    pub fn leaving_room(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::LeaveRoom(r) if r == room))
    }

    /// The newest message on screen, for Shift+F10 with nothing hovered.
    fn keyboard_target(&self) -> Option<Target> {
        let event = if let Some(c) = self.original.as_ref().filter(|c| c.room == self.selected) {
            c.event.clone()
        } else {
            let r = self.live.list.get(&self.selected)?;
            r.posts_in(self.timeline_range(r))
                .rev()
                .find(|p| p.reactable)?
                .event_id
                .clone()?
        };
        Some(Target::Message {
            room: self.selected.clone(),
            event,
        })
    }

    pub fn context_menu_event(&mut self, e: Event) -> Task<Msg> {
        match e {
            Event::Open {
                target,
                place,
                at,
                keyboard,
            } => {
                if !self.account
                    && self.viewer.is_none()
                    && self.context_menu.leaving.is_none()
                    && self.context_menu.deleting.is_none()
                    && !self.menu_items(&target).is_empty()
                {
                    self.server_menu = false;
                    self.context_menu.serial += 1;
                    self.context_menu.open = Some(Open {
                        target,
                        place,
                        at,
                        keyboard,
                        serial: self.context_menu.serial,
                    });
                }
            }
            Event::Keyboard => {
                if self.context_menu.open.is_none()
                    && self.mentions.is_none()
                    && self.setup.is_none()
                    && let Some(target) = self.keyboard_target()
                {
                    return self.context_menu_event(Event::Open {
                        target,
                        place: String::new(),
                        at: None,
                        keyboard: true,
                    });
                }
            }
            Event::Close => self.context_menu.dismiss(),
            Event::Pick(msg) => {
                self.context_menu.open = None;
                return self.update(*msg);
            }
            Event::Copy(text) => return copy(text),
            Event::CopyText(event) => {
                if let Some(p) = self.visible_post(&event) {
                    return copy(p.text().to_owned());
                }
            }
            Event::CopyLink { room, event } => {
                return copy(matrix_to(&room, event.as_deref(), &self.matrix_id));
            }
            Event::Profile { room, user } => {
                return self.profile_action(crate::engine::profile::Action::Open { room, user });
            }
            Event::MemberActions { room, user } => return self.show_member_actions(room, user),
            Event::Mention { room, user } => return self.insert_mention(&room, &user),
            Event::Direct(room) if self.live.list.contains_key(&room) => {
                // Open first, so the section doesn't open another DM on the way.
                let open = self.update(Msg::Open(room));
                return Task::batch([open, self.update(Msg::Section(Section::Direct))]);
            }
            Event::Direct(_) => {}
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Event::Alerts(room) if self.live.list.contains_key(&room) => {
                // The room alerts panel is for the open room.
                let task = if room == self.selected {
                    Task::none()
                } else {
                    self.update(Msg::Open(room))
                };
                self.alerts.panel = true;
                return task;
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Event::Alerts(_) => {}
            Event::Leave(room) => {
                if self.live.list.contains_key(&room) && !self.leaving_room(&room) {
                    self.context_menu.open = None;
                    self.context_menu.leaving = Some(room);
                }
            }
            Event::MarkLatestRead(room) => {
                if self.verified
                    && self.live.list.get(&room).is_some_and(|r| r.unread)
                    && !self.writing_read(&room)
                {
                    let public = self.read_sharing.public && !self.changing_read_sharing();
                    let target = room.clone();
                    // An empty event: the newest one, which the engine looks up.
                    self.command(
                        move |id| crate::engine::Command::MarkRoomRead {
                            id,
                            room: target,
                            public,
                        },
                        Request::ReadState(room, Some(String::new())),
                    );
                }
            }
            Event::Delete { room, event } => {
                if self.can_delete(&room, &event) {
                    self.context_menu.dismiss();
                    self.context_menu.deleting = Some((room, event));
                }
            }
            Event::ConfirmDelete => {
                if let Some((room, event)) = self.context_menu.deleting.take()
                    && self.can_delete(&room, &event)
                {
                    let request = Request::Redact(room.clone(), event.clone());
                    self.command(
                        |id| crate::engine::Command::Redact { id, room, event },
                        request,
                    );
                }
            }
            Event::Block(user) => {
                if user != self.matrix_id && !self.is_blocked(&user) {
                    self.context_menu.open = None;
                    // The card would cover the dialog.
                    self.sdk_ui.profile.dismiss_card();
                    self.context_menu.blocking = Some(user);
                }
            }
            Event::ConfirmBlock => {
                if let Some(user) = self.context_menu.blocking.take()
                    && self.verified
                {
                    self.set_blocked(user, true);
                }
            }
            Event::Unblock(user) => self.set_blocked(user, false),
            Event::ConfirmLeave => {
                if let Some(room) = self.context_menu.leaving.take()
                    && self.verified
                    && !self.leaving_room(&room)
                {
                    let target = room.clone();
                    self.command(
                        move |id| crate::engine::Command::LeaveRoom { id, room: target },
                        Request::LeaveRoom(room),
                    );
                }
            }
        }
        Task::none()
    }

    pub(super) fn can_delete(&self, room: &str, event: &str) -> bool {
        self.logged_in
            && self.verified
            && self.connected
            && room == self.selected
            && self
                .visible_post(event)
                .is_some_and(|post| self.may_delete(room, post))
            && !self.deleting_message(room, event)
    }

    /// Close a menu whose message, person or room has gone (a removed
    /// message, a left room): nothing would draw it, yet toolbars stay hidden.
    pub fn prune_context_menu(&mut self) {
        if let Some(open) = &self.context_menu.open
            && self.menu_items(&open.target).is_empty()
        {
            self.context_menu.open = None;
        }
        if let Some((room, event)) = &self.context_menu.deleting
            && !self.can_delete(room, event)
        {
            self.context_menu.deleting = None;
        }
        if let Some(room) = &self.context_menu.leaving
            && !self.live.list.contains_key(room)
        {
            self.context_menu.leaving = None;
        }
    }

    /// The member card for `user` in `room`'s member list, which also holds
    /// Kick and Ban with a reason.
    fn show_member_actions(&mut self, room: String, user: String) -> Task<Msg> {
        if !self.live.list.contains_key(&room) {
            return Task::none();
        }
        let task = if room == self.selected {
            Task::none()
        } else {
            self.update(Msg::Open(room))
        };
        if self.directory.users.contains_key(&user) {
            self.directory.choose(Some(user));
            return task;
        }
        if !self.people {
            self.people = true;
            self.reset_directory();
        }
        self.directory.want = Some(user);
        if !self.membership_busy() {
            self.read_directory(None);
        }
        task
    }

    /// Mention `user` in the composer: the same pill a pick from the
    /// autocomplete popup makes, sent with `m.mentions`.
    pub fn insert_mention(&mut self, room: &str, user: &str) -> Task<Msg> {
        if !self.live.list.contains_key(room) || room != self.selected || !self.can_stage() {
            return Task::none();
        }
        let task = self.mention(user);
        self.record_current_draft();
        task
    }

    /// The homeserver answered a leave request.
    pub fn left_room(&mut self, room: &str, ok: bool) {
        if !ok {
            let name = self
                .live
                .list
                .get(room)
                .map_or("the room", |r| r.name.as_str());
            self.status = format!("Could not leave {name}. Try again.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        moderation::{Kind, Member},
        permissions::StateDoc,
    };
    use serde_json::{Value, json};

    const ME: &str = "@me:h";
    const ROOM: &str = "!room:h";

    fn doc(id: &str, sender: &str, content: Value) -> StateDoc {
        StateDoc {
            event_id: id.into(),
            sender: sender.into(),
            content,
            redacted: false,
        }
    }
    fn message(row: i64, sender: &str, body: &str) -> Value {
        json!({"rowid":row,"event_id":format!("$m{row}"),"timestamp":row,"sender":sender,
            "type":"m.room.message","content":{"msgtype":"m.text","body":body}})
    }
    /// Signed in as `@me:h` at power `level` (kick and ban need 50) in a room
    /// with one message from each of us and `@low:h` at 0, `@high:h` at 100.
    fn app(level: i64) -> App {
        let (tx, _) = async_channel::bounded(8);
        let mut a = App::new(tx);
        a.logged_in = true;
        a.verified = true;
        a.connected = true;
        a.matrix_id = ME.into();
        a.selected = ROOM.into();
        let mut r = Room {
            has_metadata: true,
            ..Room::default()
        };
        r.insert(&message(1, "@low:h", "hello"));
        r.insert(&message(2, ME, "hi"));
        r.posting.store_snapshot(
            Some(doc("$c", "@creator:h", json!({"room_version":"11"}))),
            Some(doc(
                "$p",
                "@creator:h",
                json!({"users":{ME:level,"@high:h":100},"kick":50,"ban":50}),
            )),
        );
        a.live.list.insert(ROOM.into(), r);
        a
    }
    fn entries(items: &[Item]) -> Vec<Entry> {
        items.iter().map(|i| i.entry).collect()
    }
    fn post(a: &App, event: &str) -> Vec<Entry> {
        entries(&a.message_items(ROOM, a.visible_post(event).unwrap()))
    }

    #[test]
    fn own_messages_can_be_edited_or_deleted_and_others_reported() {
        let a = app(0);
        use Entry::*;
        assert_eq!(
            post(&a, "$m2"),
            [
                AddReaction,
                Reply,
                Edit,
                CopyText,
                MarkUnread,
                CopyMessageLink,
                Delete
            ]
        );
        assert_eq!(
            post(&a, "$m1"),
            [
                AddReaction,
                Reply,
                CopyText,
                MarkUnread,
                CopyMessageLink,
                Report
            ]
        );
        assert!(post(&app(50), "$m1").contains(&Delete));
    }

    #[test]
    fn every_toolbar_action_is_in_the_menu() {
        let a = app(0);
        for event in ["$m1", "$m2"] {
            let items = a.message_items(ROOM, a.visible_post(event).unwrap());
            let toolbar: Vec<_> = items
                .iter()
                .filter(|i| i.entry.toolbar_hint().is_some())
                .map(|i| i.entry)
                .collect();
            // The toolbar is built from this list; it shows what it used to.
            let expected: &[Entry] = if event == "$m2" {
                &[Entry::AddReaction, Entry::Reply, Entry::Edit]
            } else {
                &[Entry::AddReaction, Entry::Reply, Entry::Report]
            };
            assert_eq!(toolbar, expected);
        }
    }

    #[test]
    fn entries_disable_while_their_action_is_busy() {
        let mut a = app(0);
        a.connected = false;
        let items = a.message_items(ROOM, a.visible_post("$m2").unwrap());
        let enabled: Vec<_> = items
            .iter()
            .filter(|i| i.msg.is_some())
            .map(|i| i.entry)
            .collect();
        // Offline, only what needs no homeserver still works.
        assert_eq!(
            enabled,
            [Entry::Reply, Entry::CopyText, Entry::CopyMessageLink]
        );
        a.connected = true;
        a.live.list.get_mut(ROOM).unwrap().unread = true;
        let items = a.message_items(ROOM, a.visible_post("$m2").unwrap());
        assert!(
            items
                .iter()
                .any(|i| i.entry == Entry::MarkUnread && i.msg.is_none()),
            "an unread room can't be marked unread"
        );
    }

    #[test]
    fn kick_and_ban_follow_power_levels_and_membership() {
        use Entry::*;
        let moderator = app(50);
        assert_eq!(
            entries(&moderator.user_items(ROOM, "@low:h")),
            [Profile, Mention, Kick, Ban, Block, CopyUserId]
        );
        // Not over someone with more power, nor over yourself, whom you
        // can't block either.
        assert_eq!(
            entries(&moderator.user_items(ROOM, "@high:h")),
            [Profile, Mention, Block, CopyUserId]
        );
        assert_eq!(
            entries(&moderator.user_items(ROOM, ME)),
            [Profile, Mention, CopyUserId]
        );
        let member = app(0);
        assert_eq!(
            entries(&member.user_items(ROOM, "@low:h")),
            [Profile, Mention, Block, CopyUserId]
        );
        // Once the member list says they've left, there's no one to kick.
        let mut a = app(50);
        a.people = true;
        a.directory.users.insert(
            "@low:h".into(),
            Member {
                kind: Kind::Left,
                name: String::new(),
                avatar: None,
                event: "$leave".into(),
            },
        );
        assert_eq!(
            entries(&a.user_items(ROOM, "@low:h")),
            [Profile, Mention, Ban, Block, CopyUserId]
        );
    }

    #[test]
    fn a_direct_message_room_is_offered_for_its_peer() {
        let mut a = app(0);
        let dm = Room {
            dm: true,
            members: [("@low:h".to_owned(), "Low".to_owned())].into(),
            ..Room::default()
        };
        a.live.list.insert("!dm:h".into(), dm);
        let items = a.user_items(ROOM, "@low:h");
        let message = items.iter().find(|i| i.entry == Entry::Message).unwrap();
        assert!(matches!(
            &message.msg,
            Some(Msg::ContextMenu(Event::Direct(room))) if room == "!dm:h"
        ));
        let dm = entries(&a.channel_items("!dm:h"));
        assert_eq!(dm.first(), Some(&Entry::MarkRead));
        assert!(dm.contains(&Entry::Profile));
        assert_eq!(dm.last(), Some(&Entry::LeaveConversation));
    }

    #[test]
    fn channels_mark_read_only_when_unread() {
        let mut a = app(0);
        a.live
            .list
            .get_mut(ROOM)
            .unwrap()
            .read
            .set_latest(Some("$m2".into()));
        let mark = |a: &App| {
            a.channel_items(ROOM)
                .into_iter()
                .find(|i| i.entry == Entry::MarkRead)
                .unwrap()
                .msg
        };
        assert!(mark(&a).is_none());
        a.live.list.get_mut(ROOM).unwrap().unread = true;
        // A room never opened here has no known newest event; the engine finds it.
        let mut side = Room {
            unread: true,
            ..Room::default()
        };
        side.read.set_latest(None);
        a.live.list.insert("!side:h".into(), side);
        assert!(matches!(
            a.channel_items("!side:h")[0].msg,
            Some(Msg::ContextMenu(Event::MarkLatestRead(ref room))) if room == "!side:h"
        ));
        assert!(matches!(
            mark(&a),
            Some(Msg::MarkRoom(room, Some(event))) if room == ROOM && event == "$m2"
        ));
        let channel = entries(&a.channel_items(ROOM));
        assert!(!channel.contains(&Entry::Profile));
        assert_eq!(channel.last(), Some(&Entry::LeaveChannel));
    }

    #[test]
    fn signed_out_previews_have_no_menus() {
        let mut a = app(50);
        a.logged_in = false;
        assert!(a.menu_items(&Target::Channel(ROOM.into())).is_empty());
        assert!(
            a.menu_items(&Target::User {
                room: ROOM.into(),
                user: "@low:h".into()
            })
            .is_empty()
        );
    }

    #[test]
    fn links_point_at_the_room_or_event_through_both_servers() {
        assert_eq!(
            matrix_to("!a:one.org", Some("$e"), "@me:two.org"),
            "https://matrix.to/#/!a:one.org/$e?via=one.org&via=two.org"
        );
        // Version 12 room IDs have no server part; reserved characters are escaped.
        assert_eq!(
            matrix_to("!a/b?c", None, "@me:h"),
            "https://matrix.to/#/!a%2Fb%3Fc?via=h"
        );
    }

    #[test]
    fn opening_choosing_and_escaping() {
        let mut a = app(0);
        let target = Target::Message {
            room: ROOM.into(),
            event: "$m1".into(),
        };
        let _ = a.context_menu_event(Event::Open {
            target: target.clone(),
            place: String::new(),
            at: Some(Vector::new(4., 4.)),
            keyboard: false,
        });
        assert_eq!(
            a.context_menu.open.as_ref().map(|o| &o.target),
            Some(&target)
        );
        // Choosing an entry closes the menu and does what it says.
        let _ = a.context_menu_event(Event::Pick(Box::new(Msg::Reply("$m1".into()))));
        assert!(a.context_menu.open.is_none());
        assert_eq!(a.reply_drafts.get(ROOM).map(|q| q.id.as_str()), Some("$m1"));
        // Shift+F10 with nothing hovered opens the newest message's menu.
        let _ = a.context_menu_event(Event::Keyboard);
        let open = a.context_menu.open.clone().unwrap();
        assert!(open.keyboard);
        assert_eq!(
            open.target,
            Target::Message {
                room: ROOM.into(),
                event: "$m2".into()
            }
        );
        let _ = a.update(Msg::Escape);
        assert!(a.context_menu.open.is_none());
        // Leave asks first; Escape cancels.
        let _ = a.context_menu_event(Event::Leave(ROOM.into()));
        assert_eq!(a.context_menu.leaving.as_deref(), Some(ROOM));
        let _ = a.update(Msg::Escape);
        assert!(a.context_menu.leaving.is_none());
    }
}
