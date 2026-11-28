//! Server settings (ROADMAP C1): Discord's Server Settings for a space.
//!
//! - **Overview:** the space's name, icon and topic (the spec's state
//!   events) and its banner, our own state event that only Untethered shows
//!   (engine/space_settings.rs).
//! - **Emoji:** the space's image packs (MSC2545): upload emoji and
//!   stickers, rename a shortcode, choose what each image is offered as,
//!   delete one, or create the pack. The pickers follow once the pack event
//!   syncs (engine/packs.rs).
//! - **Channels:** each channel's name and topic. Order stays with the drag
//!   in the sidebar (channel_order.rs).
//!
//! Every control asks the room's power levels first. What this account
//! can't change shows as a value, not as a control that fails (ROADMAP's
//! Matrix support rule 5).
use crate::{
    App, Msg, Request,
    backend::Command as Backend,
    engine::{
        Command, Failure,
        packs::{self, Pack},
        space_settings::{
            self as engine, BANNER, Edit, PICTURE_LIMIT, PackRef, Picture as Kind, Reply, Snapshot,
            Usage,
        },
    },
    media::{Fit, LocalFile, PreviewRef},
    preview::Picture,
};
use iced::{Task, widget::operation};
use std::collections::{BTreeMap, BTreeSet};

/// `Msg::Files` from the file picker carries this, then the picker's ID.
pub const PICKER: &str = "server-settings:";
pub const RENAME_FIELD: &str = "server-emoji-rename";
/// Discord's limits.
pub const NAME_LIMIT: usize = 100;
pub const TOPIC_LIMIT: usize = 1024;
/// A custom emoji or sticker's picture.
const EMOTE_BYTES: u64 = 1024 * 1024;
const EMOTE_SIDE: u32 = 1024;
const PICTURE_SIDE: u32 = 4096;
/// Pictures fetched at once.
const INFLIGHT: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Overview,
    Emoji,
    Channels,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Server Overview",
            Self::Emoji => "Emoji",
            Self::Channels => "Channels",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Emoji => "Emoji",
            Self::Channels => "Channels",
        }
    }
}

#[derive(Default)]
pub struct State {
    pub page: Option<Page>,
    pictures: BTreeMap<String, Option<Picture>>,
    inflight: BTreeSet<String>,
    /// Spaces' banners (mxc URIs), for the channel list: asked once a
    /// session, and again whenever the space's settings load.
    banners: BTreeMap<String, Option<String>>,
}

impl State {
    pub fn picture(&self, url: &str) -> Option<&iced::widget::image::Handle> {
        self.pictures.get(url)?.as_ref().map(Picture::still)
    }
}

/// The open settings page.
pub struct Page {
    pub space: String,
    pub tab: Tab,
    /// The space's state as the homeserver last gave it.
    pub snapshot: Option<Snapshot>,
    pub load_failed: bool,
    /// Edits not saved yet.
    pub name: Option<String>,
    pub topic: Option<String>,
    /// Channels' edited (name, topic).
    pub channels: BTreeMap<String, (Option<String>, Option<String>)>,
    /// Channel names and topics saved here, shown until sync brings them.
    written: BTreeMap<(String, Field), String>,
    pub rename: Option<Rename>,
    picking: Option<(i64, Target)>,
    pub note: Option<Note>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Name,
    Topic,
}

pub struct Rename {
    pub pack: PackRef,
    pub from: String,
    pub input: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    pub problem: bool,
}

/// What a picked picture is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Picture(Kind),
    Image(PackRef, Usage),
}

#[derive(Clone, Debug)]
pub enum Input {
    Open(String),
    Close,
    /// Escape: leave a rename first, then the page.
    Escape,
    Tab(Tab),
    Name(String),
    Topic(String),
    ChannelName(String, String),
    ChannelTopic(String, String),
    Save,
    Reset,
    Pick(Target),
    /// A picked file read (or not), with its name.
    Read(i64, Result<Vec<u8>, &'static str>, String),
    RemovePicture(Kind),
    CreatePack,
    Rename(PackRef, String),
    Shortcode(String),
    SaveRename,
    Usage(PackRef, String, Usage),
    Delete(PackRef, String),
    /// Close the page to drag channels in the sidebar.
    Reorder,
    Fetched(String, Option<Picture>),
}

/// A request in flight, for its answer.
#[derive(Clone, Debug)]
pub enum Op {
    Load(String),
    Banner(String),
    Write { space: String, write: Write },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Name(String, String),
    Topic(String, String),
    Picture(Kind, bool),
    Pack(Edit),
    Upload(String),
}

impl Write {
    fn is_pack(&self) -> bool {
        matches!(self, Self::Pack(_) | Self::Upload(_))
    }
    fn done(&self) -> String {
        match self {
            Self::Name(..) => "Name saved.".into(),
            Self::Topic(..) => "Topic saved.".into(),
            Self::Picture(Kind::Icon, true) => "Server icon saved.".into(),
            Self::Picture(Kind::Banner, true) => "Banner saved.".into(),
            Self::Picture(Kind::Icon, false) => "Server icon removed.".into(),
            Self::Picture(Kind::Banner, false) => "Banner removed.".into(),
            Self::Pack(Edit::Create { .. }) => "Emoji pack created.".into(),
            Self::Pack(Edit::Rename { to, .. }) => format!("Renamed to :{to}:."),
            Self::Pack(Edit::Usage { shortcode, .. }) => format!(":{shortcode}: updated."),
            Self::Pack(Edit::Delete { shortcode }) => format!(":{shortcode}: deleted."),
            Self::Pack(Edit::Add { shortcode, .. }) | Self::Upload(shortcode) => {
                format!(":{shortcode}: added.")
            }
        }
    }
    fn failed(&self) -> &'static str {
        match self {
            Self::Name(..) => "Couldn't save the name. Check your role and connection.",
            Self::Topic(..) => "Couldn't save the topic. Check your role and connection.",
            Self::Picture(..) => {
                "Couldn't save the picture. Use a PNG, JPEG, GIF or WebP image under 8 MiB."
            }
            Self::Pack(Edit::Create { .. }) => "Couldn't create the emoji pack.",
            Self::Pack(Edit::Rename { .. }) => {
                "Couldn't rename it. The shortcode may be taken; the list is reloaded."
            }
            Self::Pack(_) => "Couldn't change the emoji pack. The list is reloaded.",
            Self::Upload(_) => {
                "Couldn't upload it. Use a PNG, JPEG, GIF or WebP image under 1 MiB."
            }
        }
    }
}

/// A shortcode for a picked file: its name without the extension, other
/// characters as `_`, numbered past the ones taken.
pub fn shortcode_for(file: &str, taken: &BTreeSet<String>) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let mut code: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    if code.chars().count() < 2 {
        code = format!("emoji{code}");
    }
    if !taken.contains(&code) {
        return code;
    }
    (2..)
        .map(|n| {
            let suffix = format!("_{n}");
            let keep = 32 - suffix.len();
            format!("{}{suffix}", &code[..code.len().min(keep)])
        })
        .find(|c| !taken.contains(c))
        .expect("a free shortcode")
}

/// A pack's images as the pickers read them, in shortcode order.
pub fn pack_images(space: &str, doc: &engine::PackDoc) -> Pack {
    packs::parse(&doc.content, Some(space), &doc.pack.key, doc.pack.stable).unwrap_or(Pack {
        room: Some(space.to_owned()),
        key: doc.pack.key.clone(),
        stable: doc.pack.stable,
        name: String::new(),
        avatar: None,
        attribution: None,
        images: Vec::new(),
    })
}

/// A pack's name as the Emoji tab heads it: its own, or the server's.
pub fn settings_pack_name(pack: &Pack, server: &str) -> String {
    if pack.name.trim().is_empty() {
        server.to_owned()
    } else {
        pack.name.clone()
    }
}

fn msg(input: Input) -> Msg {
    Msg::ServerSettings(input)
}

impl App {
    /// Server Settings is offered: the matrix-sdk engine, signed in and
    /// verified.
    pub fn may_open_server_settings(&self) -> bool {
        self.logged_in && self.verified && self.sdk.is_some()
    }

    /// This account may send `kind` state events in `room` now.
    pub fn may_set(&self, room: &str, kind: &str) -> bool {
        self.connected
            && self
                .live
                .list
                .get(room)
                .is_some_and(|r| r.posting.can_send_state(&self.matrix_id, kind))
    }

    pub fn may_edit_pack(&self, space: &str, pack: &PackRef) -> bool {
        self.may_set(space, pack.event_type())
    }

    /// The pack new images go to and Create makes: the space's first pack,
    /// or its default one under the spec's name.
    pub fn settings_pack(&self) -> Option<PackRef> {
        let page = self.server_settings.page.as_ref()?;
        let snap = page.snapshot.as_ref()?;
        Some(snap.packs.first().map_or(
            PackRef {
                stable: true,
                key: String::new(),
            },
            |p| p.pack.clone(),
        ))
    }

    /// A file picker is open, or its file is being read.
    pub fn settings_picking(&self) -> bool {
        self.server_settings
            .page
            .as_ref()
            .is_some_and(|p| p.picking.is_some())
    }

    /// A space's channels at any depth, by name: the Channels tab's list
    /// where the sidebar has no order for them.
    pub fn settings_channels(&self, space: &str) -> Vec<&str> {
        let store = self.rooms();
        let mut rooms: Vec<&str> = store.channels(space).into_keys().collect();
        rooms.sort_by_cached_key(|id| {
            (
                store
                    .list
                    .get(*id)
                    .map(|r| r.name.to_lowercase())
                    .unwrap_or_default(),
                id.to_string(),
            )
        });
        rooms
    }

    fn settings_writes(&self) -> impl Iterator<Item = &Write> {
        self.pending.values().filter_map(|r| match r {
            Request::ServerSettings(Op::Write { write, .. }) => Some(write),
            _ => None,
        })
    }

    pub fn settings_saving(&self) -> bool {
        self.settings_writes().any(|w| !w.is_pack())
    }

    /// A pack change in flight: they go one at a time, each reading the
    /// last one's result.
    pub fn pack_saving(&self) -> bool {
        self.settings_writes().any(Write::is_pack)
    }

    /// The space's name, topic and pictures as the page shows them: what
    /// the homeserver said, or the room list's until it has.
    pub fn settings_value(&self, field: Field) -> String {
        let Some(page) = &self.server_settings.page else {
            return String::new();
        };
        let edited = match field {
            Field::Name => &page.name,
            Field::Topic => &page.topic,
        };
        if let Some(v) = edited {
            return v.clone();
        }
        match (&page.snapshot, self.live.list.get(&page.space)) {
            (Some(s), _) => match field {
                Field::Name => s.name.clone(),
                Field::Topic => s.topic.clone(),
            },
            (None, Some(r)) => match field {
                Field::Name => r.name.clone(),
                Field::Topic => r.topic.clone(),
            },
            _ => String::new(),
        }
    }

    pub fn settings_icon(&self) -> Option<&str> {
        let page = self.server_settings.page.as_ref()?;
        match &page.snapshot {
            Some(s) => s.avatar.as_deref(),
            None => self.live.list.get(&page.space)?.avatar.as_deref(),
        }
    }

    /// A channel's name or topic as the Channels tab shows it.
    pub fn channel_value(&self, room: &str, field: Field) -> String {
        let Some(page) = &self.server_settings.page else {
            return String::new();
        };
        let edited = page
            .channels
            .get(room)
            .and_then(|(name, topic)| match field {
                Field::Name => name.as_ref(),
                Field::Topic => topic.as_ref(),
            });
        if let Some(v) = edited.or_else(|| page.written.get(&(room.to_owned(), field))) {
            return v.clone();
        }
        self.live
            .list
            .get(room)
            .map_or_else(String::new, |r| match field {
                Field::Name => r.name.clone(),
                Field::Topic => r.topic.clone(),
            })
    }

    /// Edits waiting for Save Changes.
    pub fn settings_unsaved(&self) -> bool {
        let Some(page) = &self.server_settings.page else {
            return false;
        };
        page.name.is_some()
            || page.topic.is_some()
            || page
                .channels
                .values()
                .any(|(name, topic)| name.is_some() || topic.is_some())
    }

    /// The edits are ones the homeserver will take.
    pub fn settings_valid(&self) -> bool {
        let Some(page) = &self.server_settings.page else {
            return false;
        };
        let name_ok = |n: &str| !n.trim().is_empty() && n.chars().count() <= NAME_LIMIT;
        page.name.as_deref().is_none_or(name_ok)
            && page
                .channels
                .values()
                .all(|(name, _)| name.as_deref().is_none_or(name_ok))
    }

    fn page_mut(&mut self) -> &mut Page {
        self.server_settings.page.as_mut().expect("an open page")
    }

    pub fn server_settings(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Open(space) => return self.open_server_settings(space),
            Input::Fetched(url, picture) => {
                if self.server_settings.inflight.remove(&url) {
                    self.server_settings.pictures.insert(url, picture);
                }
                return Task::none();
            }
            _ => {}
        }
        let Some(space) = self.server_settings.page.as_ref().map(|p| p.space.clone()) else {
            return Task::none();
        };
        let pack_saving = self.pack_saving();
        let saving = self.settings_saving();
        let picking = self.page_mut().picking.is_some();
        match input {
            Input::Open(_) | Input::Fetched(..) => {}
            Input::Close | Input::Reorder => self.server_settings.page = None,
            Input::Escape => {
                if self.page_mut().rename.take().is_none() {
                    self.server_settings.page = None;
                }
            }
            Input::Tab(tab) => {
                let page = self.page_mut();
                page.tab = tab;
                page.rename = None;
                page.note = None;
            }
            Input::Name(v) if !saving && self.may_set(&space, "m.room.name") => {
                let page = self.page_mut();
                page.name = Some(v.chars().take(NAME_LIMIT).collect());
                page.note = None;
            }
            Input::Topic(v) if !saving && self.may_set(&space, "m.room.topic") => {
                let page = self.page_mut();
                page.topic = Some(v.chars().take(TOPIC_LIMIT).collect());
                page.note = None;
            }
            Input::ChannelName(room, v) if !saving && self.may_set(&room, "m.room.name") => {
                let page = self.page_mut();
                page.channels.entry(room).or_default().0 =
                    Some(v.chars().take(NAME_LIMIT).collect());
                page.note = None;
            }
            Input::ChannelTopic(room, v) if !saving && self.may_set(&room, "m.room.topic") => {
                let page = self.page_mut();
                page.channels.entry(room).or_default().1 =
                    Some(v.chars().take(TOPIC_LIMIT).collect());
                page.note = None;
            }
            Input::Reset if !saving => {
                let page = self.page_mut();
                page.name = None;
                page.topic = None;
                page.channels.clear();
                page.note = None;
            }
            Input::Save if !saving && self.settings_valid() => self.save_settings(),
            Input::Pick(target) if !picking => {
                let allowed = match &target {
                    Target::Picture(kind) => !saving && self.may_set(&space, picture_type(*kind)),
                    Target::Image(pack, _) => !pack_saving && self.may_edit_pack(&space, pack),
                };
                if allowed {
                    return self.pick_settings_picture(target);
                }
            }
            Input::Read(id, result, name) => {
                let page = self.page_mut();
                let Some((_, target)) = page.picking.take_if(|(wanted, _)| *wanted == id) else {
                    return Task::none();
                };
                match result {
                    Ok(bytes) => self.send_picture(target, bytes, &name),
                    Err(problem) => page.note = Some(problem_note(problem)),
                }
            }
            Input::RemovePicture(kind) if !saving && self.may_set(&space, picture_type(kind)) => {
                self.settings_write(
                    engine::Request::Picture {
                        room: space,
                        kind,
                        bytes: None,
                    },
                    Write::Picture(kind, false),
                );
            }
            Input::CreatePack if !pack_saving => {
                let pack = PackRef {
                    stable: true,
                    key: String::new(),
                };
                let none = self
                    .page_mut()
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.packs.is_empty());
                if none && self.may_edit_pack(&space, &pack) {
                    let name = self.settings_value(Field::Name);
                    let name = if name.trim().is_empty() {
                        "Emoji".to_owned()
                    } else {
                        name
                    };
                    let edit = Edit::Create { name };
                    self.settings_write(
                        engine::Request::Pack {
                            room: space,
                            pack,
                            edit: edit.clone(),
                        },
                        Write::Pack(edit),
                    );
                }
            }
            Input::Rename(pack, from) if !pack_saving && self.may_edit_pack(&space, &pack) => {
                let page = self.page_mut();
                page.rename = Some(Rename {
                    pack,
                    input: from.clone(),
                    from,
                });
                page.note = None;
                return operation::focus(RENAME_FIELD);
            }
            Input::Shortcode(v) => {
                if let Some(r) = &mut self.page_mut().rename {
                    r.input = v.trim_matches(':').chars().take(32).collect();
                }
            }
            Input::SaveRename if !pack_saving => return self.save_rename(space),
            Input::Usage(pack, shortcode, usage)
                if !pack_saving && self.may_edit_pack(&space, &pack) =>
            {
                let edit = Edit::Usage { shortcode, usage };
                self.settings_write(
                    engine::Request::Pack {
                        room: space,
                        pack,
                        edit: edit.clone(),
                    },
                    Write::Pack(edit),
                );
            }
            Input::Delete(pack, shortcode) if !pack_saving && self.may_edit_pack(&space, &pack) => {
                let edit = Edit::Delete { shortcode };
                self.settings_write(
                    engine::Request::Pack {
                        room: space,
                        pack,
                        edit: edit.clone(),
                    },
                    Write::Pack(edit),
                );
            }
            _ => {}
        }
        Task::none()
    }

    fn save_rename(&mut self, space: String) -> Task<Msg> {
        let Some(r) = self.page_mut().rename.take() else {
            return Task::none();
        };
        let to = r.input.trim().to_owned();
        if to == r.from {
            return Task::none();
        }
        let problem = if !engine::valid_shortcode(&to) {
            Some("Shortcodes are 2 to 32 letters, numbers, _ or -.".to_owned())
        } else if self.settings_shortcodes().contains(&to) {
            Some(format!(":{to}: is taken in this server."))
        } else {
            None
        };
        if let Some(text) = problem {
            let page = self.page_mut();
            page.note = Some(Note {
                text,
                problem: true,
            });
            page.rename = Some(r);
            return operation::focus(RENAME_FIELD);
        }
        let edit = Edit::Rename { from: r.from, to };
        self.settings_write(
            engine::Request::Pack {
                room: space,
                pack: r.pack,
                edit: edit.clone(),
            },
            Write::Pack(edit),
        );
        Task::none()
    }

    fn open_server_settings(&mut self, space: String) -> Task<Msg> {
        if !self.may_open_server_settings() || !self.live.list.get(&space).is_some_and(|r| r.space)
        {
            return Task::none();
        }
        self.server_menu = false;
        self.sdk_ui.profile.dismiss_card();
        self.server_settings.page = Some(Page {
            space: space.clone(),
            tab: Tab::Overview,
            snapshot: None,
            load_failed: false,
            name: None,
            topic: None,
            channels: BTreeMap::new(),
            written: BTreeMap::new(),
            rename: None,
            picking: None,
            note: None,
        });
        self.load_settings(space);
        Task::none()
    }

    fn load_settings(&mut self, space: String) {
        let request = engine::Request::Load {
            space: space.clone(),
        };
        self.command(
            move |id| Command::SpaceSettings { id, request },
            Request::ServerSettings(Op::Load(space)),
        );
    }

    fn settings_write(&mut self, request: engine::Request, write: Write) {
        let Some(page) = &mut self.server_settings.page else {
            return;
        };
        page.note = None;
        let space = page.space.clone();
        self.command(
            move |id| Command::SpaceSettings { id, request },
            Request::ServerSettings(Op::Write { space, write }),
        );
    }

    /// Every shortcode in the space's packs.
    pub fn settings_shortcodes(&self) -> BTreeSet<String> {
        let Some(page) = &self.server_settings.page else {
            return BTreeSet::new();
        };
        page.snapshot
            .iter()
            .flat_map(|s| &s.packs)
            .flat_map(|doc| pack_images(&page.space, doc).images)
            .map(|i| i.shortcode)
            .collect()
    }

    fn save_settings(&mut self) {
        let Some(page) = &self.server_settings.page else {
            return;
        };
        let space = page.space.clone();
        let known = page
            .snapshot
            .as_ref()
            .map(|s| (s.name.clone(), s.topic.clone()));
        let mut writes = Vec::new();
        if let Some(name) = &page.name {
            writes.push((space.clone(), Field::Name, name.trim().to_owned()));
        }
        if let Some(topic) = &page.topic {
            writes.push((space.clone(), Field::Topic, topic.trim().to_owned()));
        }
        for (room, (name, topic)) in &page.channels {
            if let Some(name) = name {
                writes.push((room.clone(), Field::Name, name.trim().to_owned()));
            }
            if let Some(topic) = topic {
                writes.push((room.clone(), Field::Topic, topic.trim().to_owned()));
            }
        }
        for (room, field, value) in writes {
            let current = if room == space {
                known.clone().map(|(name, topic)| match field {
                    Field::Name => name,
                    Field::Topic => topic,
                })
            } else {
                self.live.list.get(&room).map(|r| match field {
                    Field::Name => r.name.clone(),
                    Field::Topic => r.topic.clone(),
                })
            };
            let page = self.server_settings.page.as_mut().expect("open");
            // Unchanged: nothing to send.
            if current.as_deref() == Some(value.as_str()) {
                if room == space {
                    match field {
                        Field::Name => page.name = None,
                        Field::Topic => page.topic = None,
                    }
                } else if let Some(edit) = page.channels.get_mut(&room) {
                    match field {
                        Field::Name => edit.0 = None,
                        Field::Topic => edit.1 = None,
                    }
                }
                continue;
            }
            let (request, write) = match field {
                Field::Name => (
                    engine::Request::Name {
                        room: room.clone(),
                        name: value.clone(),
                    },
                    Write::Name(room, value),
                ),
                Field::Topic => (
                    engine::Request::Topic {
                        room: room.clone(),
                        topic: value.clone(),
                    },
                    Write::Topic(room, value),
                ),
            };
            self.settings_write(request, write);
        }
    }

    fn pick_settings_picture(&mut self, target: Target) -> Task<Msg> {
        let Some(page) = &mut self.server_settings.page else {
            return Task::none();
        };
        self.seq += 1;
        let id = self.seq;
        let title = match &target {
            Target::Picture(Kind::Icon) => "Choose a server icon",
            Target::Picture(Kind::Banner) => "Choose a server banner",
            Target::Image(_, Usage::Sticker) => "Upload a sticker",
            Target::Image(..) => "Upload an emoji",
        };
        page.picking = Some((id, target));
        page.note = None;
        let token = format!("{PICKER}{id}");
        #[cfg(target_arch = "wasm32")]
        {
            let _ = title;
            crate::web_files::pick_avatar(&token);
            Task::none()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Task::perform(
                async move {
                    #[cfg(target_os = "linux")]
                    crate::app_id::portal_ready().await;
                    let picked = rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
                        .pick_file()
                        .await?;
                    LocalFile::from_path(picked.path().to_owned())
                },
                move |file| Msg::Files(token, file.into_iter().collect()),
            )
        }
    }

    /// The file picker's answer (`Msg::Files` with [`PICKER`]).
    pub fn server_settings_picked(&mut self, id: i64, file: Option<LocalFile>) -> Task<Msg> {
        let Some(page) = &mut self.server_settings.page else {
            return Task::none();
        };
        let Some((_, target)) = page.picking.as_ref().filter(|(wanted, _)| *wanted == id) else {
            return Task::none();
        };
        let Some(file) = file else {
            page.picking = None;
            return Task::none();
        };
        let (limit, side) = match target {
            Target::Picture(_) => (PICTURE_LIMIT, PICTURE_SIDE),
            Target::Image(..) => (EMOTE_BYTES, EMOTE_SIDE),
        };
        let name = file.name.clone();
        Task::perform(read_picture(file, limit, side), move |result| {
            msg(Input::Read(id, result, name.clone()))
        })
    }

    fn send_picture(&mut self, target: Target, bytes: Vec<u8>, name: &str) {
        let Some(page) = &self.server_settings.page else {
            return;
        };
        let space = page.space.clone();
        match target {
            Target::Picture(kind) => self.settings_write(
                engine::Request::Picture {
                    room: space,
                    kind,
                    bytes: Some(bytes),
                },
                Write::Picture(kind, true),
            ),
            Target::Image(pack, usage) => {
                let shortcode = shortcode_for(name, &self.settings_shortcodes());
                self.settings_write(
                    engine::Request::AddImage {
                        room: space,
                        pack,
                        shortcode: shortcode.clone(),
                        usage,
                        bytes,
                    },
                    Write::Upload(shortcode),
                );
            }
        }
    }

    /// The engine's answer to a settings request.
    pub fn space_settings_answer(&mut self, id: i64, result: Result<Reply, Failure>) {
        let Some(Request::ServerSettings(op)) = self.pending.remove(&id) else {
            return;
        };
        match op {
            Op::Banner(space) => {
                if let Ok(Reply::Banner(banner)) = result {
                    self.server_settings.banners.insert(space, banner);
                }
            }
            Op::Load(space) => {
                if let Ok(Reply::Loaded(snapshot)) = &result {
                    self.server_settings
                        .banners
                        .insert(space.clone(), snapshot.banner.clone());
                }
                let Some(page) = self
                    .server_settings
                    .page
                    .as_mut()
                    .filter(|p| p.space == space)
                else {
                    return;
                };
                match result {
                    Ok(Reply::Loaded(snapshot)) => {
                        page.snapshot = Some(snapshot);
                        page.load_failed = false;
                    }
                    _ => page.load_failed = page.snapshot.is_none(),
                }
            }
            Op::Write { space, write } => {
                let Some(page) = self
                    .server_settings
                    .page
                    .as_mut()
                    .filter(|p| p.space == space)
                else {
                    return;
                };
                let ok = result.is_ok();
                page.note = Some(Note {
                    text: if ok {
                        write.done()
                    } else {
                        write.failed().into()
                    },
                    problem: !ok,
                });
                if ok {
                    match &write {
                        Write::Name(room, value) | Write::Topic(room, value) => {
                            let field = if matches!(write, Write::Name(..)) {
                                Field::Name
                            } else {
                                Field::Topic
                            };
                            if *room == space {
                                if let Some(s) = &mut page.snapshot {
                                    match field {
                                        Field::Name => s.name.clone_from(value),
                                        Field::Topic => s.topic.clone_from(value),
                                    }
                                }
                                match field {
                                    Field::Name => page.name = None,
                                    Field::Topic => page.topic = None,
                                }
                            } else {
                                if let Some(edit) = page.channels.get_mut(room) {
                                    match field {
                                        Field::Name => edit.0 = None,
                                        Field::Topic => edit.1 = None,
                                    }
                                }
                                page.written.insert((room.clone(), field), value.clone());
                            }
                        }
                        _ => {}
                    }
                }
                // What the homeserver has now, after success or failure.
                self.load_settings(space);
            }
        }
    }

    /// Forget channel values saved here once sync shows them.
    pub fn settings_synced(&mut self) {
        let Some(page) = &mut self.server_settings.page else {
            return;
        };
        let list = &self.live.list;
        page.written.retain(|(room, field), value| {
            list.get(room).is_some_and(|r| {
                let synced = match field {
                    Field::Name => &r.name,
                    Field::Topic => &r.topic,
                };
                synced != value
            })
        });
    }

    /// The open space's banner over its channel list, once it's here.
    pub fn space_banner(&self) -> Option<&iced::widget::image::Handle> {
        let crate::Section::Space(space) = &self.section else {
            return None;
        };
        let url = self.server_settings.banners.get(space)?.as_deref()?;
        self.server_settings.picture(url)
    }

    /// Ask for the open space's banner once.
    fn ask_banner(&mut self) {
        let crate::Section::Space(space) = &self.section else {
            return;
        };
        let asked = self.server_settings.banners.contains_key(space)
            || self.pending.values().any(
                |r| matches!(r, Request::ServerSettings(Op::Banner(s) | Op::Load(s)) if s == space),
            );
        if asked
            || !self.may_open_server_settings()
            || !self.connected
            || !self.live.list.get(space).is_some_and(|r| r.space)
        {
            return;
        }
        let space = space.clone();
        let request = engine::Request::Banner {
            space: space.clone(),
        };
        self.command(
            move |id| Command::SpaceSettings { id, request },
            Request::ServerSettings(Op::Banner(space)),
        );
    }

    /// The pictures on screen: the open space's banner, and the settings
    /// page's banner and emoji.
    fn wanted_settings_pictures(&self) -> Vec<(String, Fit)> {
        let mut wanted = Vec::new();
        if let crate::Section::Space(space) = &self.section
            && let Some(Some(url)) = self.server_settings.banners.get(space)
        {
            wanted.push((url.clone(), Fit::Inline));
        }
        let Some(page) = &self.server_settings.page else {
            return wanted;
        };
        let Some(snap) = &page.snapshot else {
            return wanted;
        };
        wanted.extend(match page.tab {
            Tab::Overview => snap
                .banner
                .iter()
                .map(|b| (b.clone(), Fit::Inline))
                .collect(),
            Tab::Emoji => snap
                .packs
                .iter()
                .flat_map(|doc| pack_images(&page.space, doc).images)
                .map(|i| (i.url, Fit::Emote))
                .collect(),
            Tab::Channels => Vec::new(),
        });
        wanted
    }

    pub fn schedule_settings_pictures(&mut self) -> Task<Msg> {
        if !self.logged_in || !self.connected {
            return Task::none();
        }
        self.ask_banner();
        let wanted = self.wanted_settings_pictures();
        let mut tasks = Vec::new();
        for (url, fit) in wanted {
            let state = &mut self.server_settings;
            if state.inflight.len() >= INFLIGHT {
                break;
            }
            if state.pictures.contains_key(&url) || state.inflight.contains(&url) {
                continue;
            }
            let (tx, rx) = async_channel::bounded(1);
            let source = PreviewRef {
                mxc: url.clone(),
                encrypted: false,
                fit,
            };
            if self
                .commands
                .try_send(Backend::Inline { source, result: tx })
                .is_err()
            {
                break;
            }
            self.server_settings.inflight.insert(url.clone());
            tasks.push(Task::perform(
                async move { rx.recv().await.ok().flatten() },
                move |picture| msg(Input::Fetched(url.clone(), picture)),
            ));
        }
        Task::batch(tasks)
    }
}

pub fn picture_type(kind: Kind) -> &'static str {
    match kind {
        Kind::Icon => "m.room.avatar",
        Kind::Banner => BANNER,
    }
}

fn problem_note(problem: &str) -> Note {
    Note {
        text: problem.into(),
        problem: true,
    }
}

/// A picked picture's bytes, if it's a picture within the limits.
async fn read_picture(file: LocalFile, limit: u64, side: u32) -> Result<Vec<u8>, &'static str> {
    const READ: &str = "Couldn't read the selected image.";
    let too_big = if limit < PICTURE_LIMIT {
        "Choose an image under 1 MiB."
    } else {
        "Choose an image under 8 MiB."
    };
    if file.size > limit {
        return Err(too_big);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = match &file.origin {
        crate::media::NativeSource::Clipboard(bytes) => bytes.to_vec(),
        origin => {
            use tokio::io::AsyncReadExt;
            let path = origin.read_path().ok_or(READ)?;
            let f = tokio::fs::File::open(path).await.map_err(|_| READ)?;
            let mut bytes = Vec::new();
            f.take(limit + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| READ)?;
            bytes
        }
    };
    #[cfg(target_arch = "wasm32")]
    let bytes = {
        let buffer = wasm_bindgen_futures::JsFuture::from(file.file.array_buffer())
            .await
            .map_err(|_| READ)?;
        js_sys::Uint8Array::new(&buffer).to_vec()
    };
    if bytes.len() as u64 > limit {
        return Err(too_big);
    }
    match engine::describe(&bytes) {
        Some((_, (w, h))) if w <= side && h <= side => Ok(bytes),
        Some(_) if side == EMOTE_SIDE => Err("Choose an image up to 1024 × 1024."),
        Some(_) => Err("Choose an image up to 4096 × 4096."),
        None => Err("Choose a PNG, JPEG, GIF or WebP image."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{RoomList, RoomSummary, space_settings::PackDoc};
    use serde_json::json;

    #[test]
    fn a_picked_file_names_its_shortcode() {
        let taken: BTreeSet<String> = ["party".into(), "party_2".into()].into();
        assert_eq!(shortcode_for("blob wave.png", &taken), "blob_wave");
        assert_eq!(shortcode_for("party.gif", &taken), "party_3");
        assert_eq!(shortcode_for("x.png", &taken), "emojix");
        assert_eq!(shortcode_for("noext", &taken), "noext");
        let long = format!("{}.png", "a".repeat(40));
        let code = shortcode_for(&long, &BTreeSet::from(["a".repeat(32)]));
        assert_eq!(code, format!("{}_2", "a".repeat(30)));
        assert!(engine::valid_shortcode(&code));
    }

    fn app(level: i64) -> (App, async_channel::Receiver<Command>) {
        let (backend, _) = async_channel::bounded(8);
        let (sdk, rx) = async_channel::bounded(16);
        let mut app = App::new(backend);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.matrix_id = "@me:h".into();
        let power = json!({"users": {"@me:h": level}, "users_default": 0,
            "state_default": 50, "events": {}});
        let doc = |content: serde_json::Value| crate::permissions::StateDoc {
            event_id: "$p".into(),
            sender: "@owner:h".into(),
            content,
            redacted: false,
        };
        let create = doc(json!({"room_version": "10"}));
        let rooms = ["!space:h", "!general:h"].map(|id| RoomSummary {
            id: id.into(),
            name: id[1..id.find(':').unwrap()].into(),
            space: id == "!space:h",
            create: Some(create.clone()),
            power: Some(doc(power.clone())),
            ..Default::default()
        });
        app.live.apply_engine(RoomList {
            rooms: rooms.into(),
            ..Default::default()
        });
        (app, rx)
    }

    fn answer(app: &mut App, rx: &async_channel::Receiver<Command>, reply: Result<Reply, Failure>) {
        let Ok(Command::SpaceSettings { id, .. }) = rx.try_recv() else {
            panic!("no settings request");
        };
        app.space_settings_answer(id, reply);
    }

    fn loaded() -> Snapshot {
        Snapshot {
            name: "Workshop".into(),
            topic: "Making".into(),
            packs: vec![PackDoc {
                pack: PackRef {
                    stable: true,
                    key: String::new(),
                },
                content: json!({"images": {"wave": {"url": "mxc://h/w"}}}),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn opening_loads_the_space_and_saving_sends_only_changes() {
        let (mut app, rx) = app(100);
        let _ = app.server_settings(Input::Open("!space:h".into()));
        answer(&mut app, &rx, Ok(Reply::Loaded(loaded())));
        assert_eq!(app.settings_value(Field::Name), "Workshop");
        let _ = app.server_settings(Input::Name("Workshop 2".into()));
        let _ = app.server_settings(Input::Topic("Making".into()));
        let _ = app.server_settings(Input::ChannelTopic("!general:h".into(), "Chat".into()));
        assert!(app.settings_unsaved());
        let _ = app.server_settings(Input::Save);
        let mut sent = Vec::new();
        while let Ok(Command::SpaceSettings { id, request }) = rx.try_recv() {
            sent.push((id, request));
        }
        let requests: Vec<_> = sent.iter().map(|(_, r)| r.clone()).collect();
        assert_eq!(
            requests,
            [
                engine::Request::Name {
                    room: "!space:h".into(),
                    name: "Workshop 2".into()
                },
                engine::Request::Topic {
                    room: "!general:h".into(),
                    topic: "Chat".into()
                },
            ]
        );
        // The unchanged topic was dropped; the rest wait for their answers.
        assert!(app.settings_saving());
        for (id, _) in sent {
            app.space_settings_answer(id, Ok(Reply::Written("$e".into())));
        }
        assert!(!app.settings_unsaved());
        assert_eq!(app.settings_value(Field::Name), "Workshop 2");
        assert_eq!(app.channel_value("!general:h", Field::Topic), "Chat");
        // Each write reloads the space.
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::SpaceSettings {
                request: engine::Request::Load { .. },
                ..
            })
        ));
    }

    #[test]
    fn a_member_without_permission_changes_nothing() {
        let (mut app, rx) = app(0);
        let _ = app.server_settings(Input::Open("!space:h".into()));
        answer(&mut app, &rx, Ok(Reply::Loaded(loaded())));
        let pack = PackRef {
            stable: true,
            key: String::new(),
        };
        assert!(!app.may_set("!space:h", "m.room.name"));
        assert!(!app.may_edit_pack("!space:h", &pack));
        let _ = app.server_settings(Input::Name("Mine".into()));
        let _ = app.server_settings(Input::ChannelName("!general:h".into(), "mine".into()));
        let _ = app.server_settings(Input::Save);
        let _ = app.server_settings(Input::Delete(pack.clone(), "wave".into()));
        let _ = app.server_settings(Input::Rename(pack, "wave".into()));
        let _ = app.server_settings(Input::RemovePicture(Kind::Icon));
        assert!(!app.settings_unsaved());
        assert!(app.server_settings.page.as_ref().unwrap().rename.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pack_edits_go_one_at_a_time_and_check_shortcodes_first() {
        let (mut app, rx) = app(100);
        let _ = app.server_settings(Input::Open("!space:h".into()));
        answer(&mut app, &rx, Ok(Reply::Loaded(loaded())));
        let pack = app.settings_pack().unwrap();
        let _ = app.server_settings(Input::Rename(pack.clone(), "wave".into()));
        let _ = app.server_settings(Input::Shortcode(":x:".into()));
        let _ = app.server_settings(Input::SaveRename);
        // Too short: the rename stays open with a note.
        assert!(app.server_settings.page.as_ref().unwrap().rename.is_some());
        assert!(rx.try_recv().is_err());
        let _ = app.server_settings(Input::Shortcode("hello".into()));
        let _ = app.server_settings(Input::SaveRename);
        let Ok(Command::SpaceSettings { id, request }) = rx.try_recv() else {
            panic!("no rename");
        };
        assert_eq!(
            request,
            engine::Request::Pack {
                room: "!space:h".into(),
                pack: pack.clone(),
                edit: Edit::Rename {
                    from: "wave".into(),
                    to: "hello".into()
                }
            }
        );
        // Another pack change waits for this one.
        let _ = app.server_settings(Input::Delete(pack, "wave".into()));
        assert!(rx.try_recv().is_err());
        app.space_settings_answer(id, Err(Failure::Failed));
        let note = app
            .server_settings
            .page
            .as_ref()
            .unwrap()
            .note
            .clone()
            .unwrap();
        assert!(note.problem);
    }

    #[test]
    fn a_space_without_a_pack_can_have_one_made() {
        let (mut app, rx) = app(100);
        let _ = app.server_settings(Input::Open("!space:h".into()));
        let mut snap = loaded();
        snap.packs.clear();
        answer(&mut app, &rx, Ok(Reply::Loaded(snap)));
        let _ = app.server_settings(Input::CreatePack);
        let Ok(Command::SpaceSettings { request, .. }) = rx.try_recv() else {
            panic!("no create");
        };
        assert_eq!(
            request,
            engine::Request::Pack {
                room: "!space:h".into(),
                pack: PackRef {
                    stable: true,
                    key: String::new()
                },
                edit: Edit::Create {
                    name: "Workshop".into()
                }
            }
        );
    }
}
