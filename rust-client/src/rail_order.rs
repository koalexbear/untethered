//! The server rail's order and folders (ROADMAP N8), arranged as on
//! Discord: drag a server to move it, drop it on another to make a folder,
//! drag it out of a folder to take it out again; click a folder to open or
//! close it, right-click it for its name and colour.
//!
//! The order is Element's, MSC3230: each space's room account data
//! `org.matrix.msc3230.space_order` holds an `order` of at most 50
//! printable ASCII characters (0x20 to 0x7E), compared byte by byte; a
//! space without a valid one comes after those with, and ties go by room
//! ID. A drop gives the moved server an order between its new neighbours',
//! and one to each server without an order ahead of it (they'd sort after
//! it otherwise), renumbering neighbours only when there's no room: N5's
//! order strings (channel_order.rs), base 95 over the same alphabet Element
//! uses. Element orders only top-level spaces (their `m.space.child` edges
//! order the rest); the rail lists subspaces too and orders them alike.
//!
//! Folders are Untethered's own (engine/rail.rs): one account data event
//! lists each folder's servers, name and colour. A folder sits where its
//! first server is in the order, with its other servers right after, so a
//! drop keeps a folder's servers together in the order other clients see.
//! A folder with no servers left is dropped.
//!
//! What's written shows at once: an order until sync brings that space's
//! order back changed, the folders until sync brings them back or the write
//! fails.
use crate::{
    App, Msg, Request, channel_order,
    engine::{
        ChildEdge, Command,
        rail::{DEFAULT_COLOR, Folder},
    },
};
use iced::Task;
use std::collections::{BTreeMap, BTreeSet};

/// Discord's folder colours, the default first.
pub const COLORS: [u32; 11] = [
    DEFAULT_COLOR,
    0x1abc9c,
    0x2ecc71,
    0x3498db,
    0x9b59b6,
    0xe91e63,
    0xf1c40f,
    0xe67e22,
    0xe74c3c,
    0x95a5a6,
    0x607d8b,
];
/// The longest folder name, as on Discord.
pub const NAME: usize = 100;
/// Folder Settings' name field.
pub const FIELD: &str = "folder-name";

/// A rail entry under Direct Messages and All Rooms.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Server(String),
    /// A folder with its joined servers, in the rail's order.
    Folder {
        id: String,
        servers: Vec<String>,
    },
}

impl Entry {
    pub fn servers(&self) -> &[String] {
        match self {
            Entry::Server(space) => std::slice::from_ref(space),
            Entry::Folder { servers, .. } => servers,
        }
    }
}

/// The rail: servers in `order`, each folder where its first server is,
/// with its servers after it in order. Folders with none joined don't show.
pub fn layout(order: &[String], folders: &[Folder]) -> Vec<Entry> {
    // A space in two folders belongs to the first.
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, f) in folders.iter().enumerate() {
        for space in &f.spaces {
            owner.entry(space.as_str()).or_insert(i);
        }
    }
    let mut placed = BTreeSet::new();
    let mut entries = Vec::new();
    for space in order {
        match owner.get(space.as_str()) {
            None => entries.push(Entry::Server(space.clone())),
            Some(&i) if placed.insert(i) => entries.push(Entry::Folder {
                id: folders[i].id.clone(),
                servers: order
                    .iter()
                    .filter(|s| owner.get(s.as_str()) == Some(&i))
                    .cloned()
                    .collect(),
            }),
            Some(_) => {}
        }
    }
    entries
}

/// The servers top to bottom, folders' included.
pub fn flat(entries: &[Entry]) -> Vec<String> {
    entries.iter().flat_map(Entry::servers).cloned().collect()
}

/// What's being dragged.
#[derive(Clone, Debug, PartialEq)]
pub enum Held {
    Server(String),
    Folder(String),
}

/// An icon a drag can be over.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    /// A server's; `folder`: the open folder it's in.
    Server {
        space: String,
        folder: Option<String>,
    },
    /// A folder's.
    Folder(String),
}

/// Which part of an icon the pointer is over: dropping in the middle of a
/// server makes a folder, of a folder puts the server in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Above,
    Into,
    Below,
}

impl Zone {
    /// The zone of `index` among an icon's `zones` (2: halves, 3: thirds).
    pub fn of(index: u8, zones: u8) -> Zone {
        match (index, zones) {
            (0, _) => Zone::Above,
            (1, 3) => Zone::Into,
            _ => Zone::Below,
        }
    }
}

/// What the rail draws while dragging.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// The drop line above this icon, or below.
    Line(Spot, bool),
    /// This icon lights up: the drop makes a folder or fills one.
    Into(Spot),
}

/// A drop's outcome: the rail's servers top to bottom, and the folders.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub mark: Mark,
    pub order: Vec<String>,
    pub folders: Vec<Folder>,
}

/// Where dropping `held` on `spot` puts it. `open`: the folders open in
/// the rail; `fresh`: the ID a new folder gets.
pub fn plan(
    entries: &[Entry],
    folders: &[Folder],
    open: &BTreeSet<String>,
    held: &Held,
    spot: &Spot,
    zone: Zone,
    fresh: &str,
) -> Option<Plan> {
    match held {
        Held::Server(space) => place_server(entries, folders, open, space, spot, zone, fresh),
        Held::Folder(id) => place_folder(entries, folders, open, id, spot, zone),
    }
}

fn folder_at(entries: &[Entry], id: &str) -> Option<usize> {
    entries
        .iter()
        .position(|e| matches!(e, Entry::Folder { id: f, .. } if f == id))
}

fn place_server(
    entries: &[Entry],
    folders: &[Folder],
    open: &BTreeSet<String>,
    held: &str,
    spot: &Spot,
    zone: Zone,
    fresh: &str,
) -> Option<Plan> {
    let home = entries.iter().find_map(|e| match e {
        Entry::Folder { id, servers } if servers.iter().any(|s| s == held) => Some(id.as_str()),
        _ => None,
    });
    match spot {
        // On itself, or in the middle of its own folder: it stays.
        Spot::Server { space, .. } if space == held => return None,
        Spot::Folder(id) if zone == Zone::Into && home == Some(id.as_str()) => return None,
        _ => {}
    }
    // Take it out of wherever it is; a folder left empty keeps its place
    // until the end.
    let mut entries: Vec<Entry> = entries
        .iter()
        .filter(|e| **e != Entry::Server(held.to_owned()))
        .cloned()
        .collect();
    for e in &mut entries {
        if let Entry::Folder { servers, .. } = e {
            servers.retain(|s| s != held);
        }
    }
    let mut folders = folders.to_vec();
    for f in &mut folders {
        f.spaces.retain(|s| s != held);
    }
    let held = held.to_owned();
    // Put it in folder `id`'s data too.
    let join = |folders: &mut Vec<Folder>, id: &str| -> Option<()> {
        folders
            .iter_mut()
            .find(|f| f.id == id)?
            .spaces
            .push(held.clone());
        Some(())
    };
    let mark = match spot {
        Spot::Server {
            space,
            folder: None,
        } => {
            let at = entries
                .iter()
                .position(|e| *e == Entry::Server(space.clone()))?;
            match zone {
                Zone::Above => entries.insert(at, Entry::Server(held.clone())),
                Zone::Below => entries.insert(at + 1, Entry::Server(held.clone())),
                Zone::Into => {
                    let spaces = vec![space.clone(), held.clone()];
                    entries[at] = Entry::Folder {
                        id: fresh.to_owned(),
                        servers: spaces.clone(),
                    };
                    folders.push(Folder {
                        id: fresh.to_owned(),
                        name: String::new(),
                        color: DEFAULT_COLOR,
                        spaces,
                    });
                    return finish(entries, folders, Mark::Into(spot.clone()));
                }
            }
            Mark::Line(spot.clone(), zone == Zone::Below)
        }
        Spot::Server {
            space,
            folder: Some(id),
        } => {
            let at = folder_at(&entries, id)?;
            let Entry::Folder { servers, .. } = &mut entries[at] else {
                return None;
            };
            let below = zone != Zone::Above;
            let i = servers.iter().position(|s| s == space)?;
            servers.insert(i + usize::from(below), held.clone());
            join(&mut folders, id)?;
            Mark::Line(spot.clone(), below)
        }
        Spot::Folder(id) => {
            let at = folder_at(&entries, id)?;
            match zone {
                Zone::Above => entries.insert(at, Entry::Server(held.clone())),
                Zone::Below if !open.contains(id) => {
                    entries.insert(at + 1, Entry::Server(held.clone()))
                }
                // Under an open folder's icon: its first server.
                Zone::Below | Zone::Into => {
                    if let Entry::Folder { servers, .. } = &mut entries[at] {
                        if zone == Zone::Into {
                            servers.push(held.clone());
                        } else {
                            servers.insert(0, held.clone());
                        }
                    }
                    join(&mut folders, id)?;
                }
            }
            if zone == Zone::Into {
                Mark::Into(spot.clone())
            } else {
                Mark::Line(spot.clone(), zone == Zone::Below)
            }
        }
    };
    finish(entries, folders, mark)
}

fn place_folder(
    entries: &[Entry],
    folders: &[Folder],
    open: &BTreeSet<String>,
    held: &str,
    spot: &Spot,
    zone: Zone,
) -> Option<Plan> {
    let mut entries = entries.to_vec();
    let block = entries.remove(folder_at(&entries, held)?);
    // Folders don't nest: next to the folder a spot is in.
    let (at, below, mark) = match spot {
        Spot::Server {
            space,
            folder: None,
        } => {
            let at = entries
                .iter()
                .position(|e| *e == Entry::Server(space.clone()))?;
            let below = zone != Zone::Above;
            (at, below, Mark::Line(spot.clone(), below))
        }
        Spot::Server {
            folder: Some(id), ..
        }
        | Spot::Folder(id) => {
            if id == held {
                return None;
            }
            let at = folder_at(&entries, id)?;
            let below = zone != Zone::Above || matches!(spot, Spot::Server { .. });
            // Below an open folder: under its last server.
            let last = entries[at].servers().last().cloned();
            let mark = match (below, last) {
                (true, Some(space)) if open.contains(id) => Mark::Line(
                    Spot::Server {
                        space,
                        folder: Some(id.clone()),
                    },
                    true,
                ),
                _ => Mark::Line(Spot::Folder(id.clone()), below),
            };
            (at, below, mark)
        }
    };
    entries.insert(at + usize::from(below), block);
    finish(entries, folders.to_vec(), mark)
}

fn finish(mut entries: Vec<Entry>, mut folders: Vec<Folder>, mark: Mark) -> Option<Plan> {
    entries.retain(|e| !matches!(e, Entry::Folder { servers, .. } if servers.is_empty()));
    folders.retain(|f| !f.spaces.is_empty());
    Some(Plan {
        mark,
        order: flat(&entries),
        folders,
    })
}

#[derive(Clone, Debug)]
pub enum Input {
    /// An icon was pressed and moved far enough to drag it.
    Lift(Held),
    /// The pointer is over this part of an icon.
    Over(Spot, Zone),
    /// The pointer left an icon.
    Leave(Spot),
    /// The pointer left the window.
    Away,
    /// The mouse button came up.
    Drop,
    /// A folder was clicked: open or close it.
    Toggle(String),
    /// Open a folder's right-click menu, or close it.
    Menu(Option<String>),
    /// Open Folder Settings for a folder, or close it.
    Settings(Option<String>),
    Name(String),
    Color(u32),
    Save,
}

/// The drag in progress.
#[derive(Clone, Debug, Default)]
pub struct Drag {
    pub held: Option<Held>,
    pub over: Option<(Spot, Zone)>,
}

/// Folder Settings, open.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub folder: String,
    pub name: String,
    pub color: u32,
}

/// The rail's state in the App.
#[derive(Default)]
pub struct Rail {
    /// The folders as last synced.
    synced: Vec<Folder>,
    /// Folders written here, shown until sync brings them back.
    writing: Option<Writing>,
    /// Orders written here.
    written: Vec<Written>,
    /// Folders open in the rail, on this device.
    pub open: BTreeSet<String>,
    pub drag: Drag,
    /// The folder whose right-click menu is open.
    pub menu: Option<String>,
    pub settings: Option<Settings>,
    /// The layout last written to the QA event log.
    #[cfg(not(target_arch = "wasm32"))]
    logged: Option<serde_json::Value>,
}

struct Writing {
    request: i64,
    folders: Vec<Folder>,
    /// The write succeeded; any sync of the folders ends this.
    done: bool,
}

/// An order written here, shown until sync brings the space's order back
/// changed from `was` (ours, or anyone's since), or the write fails.
struct Written {
    request: i64,
    space: String,
    order: String,
    was: Option<String>,
}

impl Rail {
    /// Escape closes what's open; false if nothing was.
    pub fn escape(&mut self) -> bool {
        if self.drag.held.is_some() {
            self.drag = Drag::default();
        } else if self.settings.is_some() {
            self.settings = None;
        } else if self.menu.is_some() {
            self.menu = None;
        } else {
            return false;
        }
        true
    }

    pub fn busy(&self) -> bool {
        self.drag.held.is_some() || self.settings.is_some() || self.menu.is_some()
    }
}

impl App {
    /// The rail can be rearranged: the order and folders are account data.
    pub fn may_rearrange(&self) -> bool {
        self.logged_in && self.verified && self.connected && self.sdk.is_some()
    }

    /// The folders, with what's being written here.
    pub fn rail_folders(&self) -> &[Folder] {
        if !self.logged_in {
            return &[];
        }
        self.rail
            .writing
            .as_ref()
            .map_or(&self.rail.synced, |w| &w.folders)
    }

    /// The servers in MSC3230's order, with what's being written here.
    pub fn rail_order(&self) -> Vec<ChildEdge> {
        let mut spaces: Vec<ChildEdge> = self
            .rooms()
            .list
            .iter()
            .filter(|(_, r)| r.space)
            .map(|(id, r)| ChildEdge {
                child: id.clone(),
                order: self
                    .rail
                    .written
                    .iter()
                    .rev()
                    .find(|w| w.space == *id)
                    .map(|w| w.order.clone())
                    .or_else(|| r.space_order.clone()),
                // Ties go by room ID, as Element sorts them.
                ts: 0,
            })
            .collect();
        channel_order::sort(&mut spaces);
        spaces
    }

    /// The rail's servers and folders, top to bottom.
    pub fn rail_entries(&self) -> Vec<Entry> {
        let order: Vec<String> = self.rail_order().into_iter().map(|e| e.child).collect();
        layout(&order, self.rail_folders())
    }

    /// The drag in progress, if any.
    pub fn rail_held(&self) -> Option<&Held> {
        self.rail.drag.held.as_ref()
    }

    /// What a drop would do now, if it changes anything.
    pub fn rail_landing(&self) -> Option<Plan> {
        self.rail_plan("")
    }

    fn rail_plan(&self, fresh: &str) -> Option<Plan> {
        let held = self.rail.drag.held.as_ref()?;
        let (spot, zone) = self.rail.drag.over.as_ref()?;
        let entries = self.rail_entries();
        let folders = self.rail_folders();
        let plan = plan(&entries, folders, &self.rail.open, held, spot, *zone, fresh)?;
        (plan.order != flat(&entries) || plan.folders != folders).then_some(plan)
    }

    pub fn rail_input(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Lift(held) => {
                if self.may_rearrange() && self.rail.drag.held.is_none() {
                    self.rail.drag = Drag {
                        held: Some(held),
                        over: None,
                    };
                    self.rail.menu = None;
                }
            }
            Input::Over(spot, zone) if self.rail.drag.held.is_some() => {
                self.rail.drag.over = Some((spot, zone));
            }
            Input::Over(..) => {}
            Input::Leave(spot) => {
                if self
                    .rail
                    .drag
                    .over
                    .as_ref()
                    .is_some_and(|(s, _)| *s == spot)
                {
                    self.rail.drag.over = None;
                }
            }
            Input::Away => self.rail.drag.over = None,
            Input::Drop => {
                let fresh = self.fresh_folder_id();
                let plan = self.rail_plan(&fresh);
                let held = std::mem::take(&mut self.rail.drag).held;
                if let (Some(held), Some(plan)) = (held, plan) {
                    self.rail_drop(held, plan);
                }
            }
            Input::Toggle(id) => {
                if !self.rail.open.remove(&id) {
                    self.rail.open.insert(id);
                }
                self.note_rail();
            }
            Input::Menu(folder) => self.rail.menu = folder,
            Input::Settings(Some(id)) => {
                self.rail.menu = None;
                self.rail.settings =
                    self.rail_folders()
                        .iter()
                        .find(|f| f.id == id)
                        .map(|f| Settings {
                            folder: id,
                            name: f.name.clone(),
                            color: f.color,
                        });
                return iced::widget::operation::focus(FIELD);
            }
            Input::Settings(None) => self.rail.settings = None,
            Input::Name(name) => {
                if let Some(s) = &mut self.rail.settings {
                    s.name = name.chars().take(NAME).collect();
                }
            }
            Input::Color(color) => {
                if let Some(s) = &mut self.rail.settings {
                    s.color = color;
                }
            }
            Input::Save => {
                let Some(s) = self.rail.settings.take() else {
                    return Task::none();
                };
                let mut folders = self.rail_folders().to_vec();
                if let Some(f) = folders.iter_mut().find(|f| f.id == s.folder) {
                    f.name = s.name.trim().to_owned();
                    f.color = s.color;
                }
                if folders != self.rail_folders() {
                    self.write_rail(Vec::new(), Some(folders));
                }
            }
        }
        Task::none()
    }

    /// An ID no folder has.
    fn fresh_folder_id(&self) -> String {
        let mut n = chrono::Utc::now().timestamp_millis();
        let taken = |id: &str| self.rail_folders().iter().any(|f| f.id == id);
        while taken(&format!("{n:x}")) {
            n += 1;
        }
        format!("{n:x}")
    }

    fn rail_drop(&mut self, held: Held, plan: Plan) {
        let order = self.rail_order();
        let entries = self.rail_entries();
        let now = flat(&entries);
        let block: Vec<String> = match &held {
            Held::Server(space) => vec![space.clone()],
            Held::Folder(id) => entries
                .iter()
                .find(|e| matches!(e, Entry::Folder { id: f, .. } if f == id))
                .map(|e| e.servers().to_vec())
                .unwrap_or_default(),
        };
        let mut orders = Vec::new();
        if plan.order != now && !block.is_empty() {
            // Neighbours as the rail shows them, which is the order unless
            // another client split a folder's servers up.
            let siblings: Vec<ChildEdge> = now
                .iter()
                .filter(|s| !block.contains(s))
                .filter_map(|s| order.iter().find(|e| e.child == *s).cloned())
                .collect();
            let last = block
                .last()
                .and_then(|l| plan.order.iter().position(|s| s == l));
            let before = last
                .and_then(|at| plan.order.get(at + 1))
                .map(String::as_str);
            let block: Vec<&str> = block.iter().map(String::as_str).collect();
            orders = channel_order::reorder_block(&siblings, &block, before);
        }
        let folders = (plan.folders != self.rail_folders()).then_some(plan.folders);
        if !orders.is_empty() || folders.is_some() {
            self.write_rail(orders, folders);
        }
    }

    /// Write orders and folders, showing them at once.
    fn write_rail(&mut self, orders: Vec<(String, String)>, folders: Option<Vec<Folder>>) {
        let (sent_orders, sent_folders) = (orders.clone(), folders.clone());
        self.command(
            move |id| Command::ArrangeRail {
                id,
                orders: sent_orders,
                folders: sent_folders,
            },
            Request::ArrangeRail,
        );
        // Queued: show it at once.
        if !matches!(self.pending.get(&self.seq), Some(Request::ArrangeRail)) {
            return;
        }
        let request = self.seq;
        for (space, order) in orders {
            let was = self
                .live
                .list
                .get(&space)
                .and_then(|r| r.space_order.clone());
            self.rail.written.push(Written {
                request,
                space,
                order,
                was,
            });
        }
        if let Some(folders) = folders {
            self.rail.writing = Some(Writing {
                request,
                folders,
                done: false,
            });
        }
        self.note_rail();
    }

    /// A rail write finished; one that failed stops showing.
    pub fn rail_arranged(&mut self, request: i64, ok: bool) {
        if !ok {
            self.rail.written.retain(|w| w.request != request);
            self.status = "Couldn't move it there. Try again.".into();
        }
        if let Some(w) = &mut self.rail.writing
            && w.request == request
        {
            if !ok || w.folders == self.rail.synced {
                self.rail.writing = None;
            } else {
                w.done = true;
            }
        }
        self.note_rail();
    }

    /// The room list changed: orders that came back stop being pending.
    pub fn rail_synced(&mut self) {
        let live = &self.live;
        self.rail.written.retain(|w| {
            live.list
                .get(&w.space)
                .is_some_and(|r| r.space_order == w.was)
        });
        self.note_rail();
    }

    /// The folders synced.
    pub fn folders_synced(&mut self, folders: Vec<Folder>) {
        if self
            .rail
            .writing
            .as_ref()
            .is_some_and(|w| w.done || w.folders == folders)
        {
            self.rail.writing = None;
        }
        self.rail.synced = folders;
        // Settings for a folder that's gone close.
        if let Some(s) = &self.rail.settings
            && !self.rail_folders().iter().any(|f| f.id == s.folder)
        {
            self.rail.settings = None;
        }
        self.note_rail();
    }

    /// While something's held, the button coming up anywhere drops it.
    pub fn rail_subscription(&self) -> iced::Subscription<Msg> {
        if self.rail.drag.held.is_none() {
            return iced::Subscription::none();
        }
        iced::event::listen_with(|event, _, _| match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Msg::Rail(Input::Drop))
            }
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Msg::Rail(Input::Away)),
            _ => None,
        })
    }

    /// QA fixtures (UNTETHERED_QA_EVENTS, as engine/qa.rs; release builds
    /// too, since the fixtures drive those): each new rail layout as
    /// `{"kind":"rail","entries":[{"space","name"} or {"folder","name",
    /// "color","open","servers":[{"space","name"}]}]}`.
    pub fn note_rail(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use serde_json::json;
            use std::io::Write;
            let Some(path) = std::env::var_os("UNTETHERED_QA_EVENTS") else {
                return;
            };
            if !self.logged_in {
                return;
            }
            let server = |id: &String| {
                let name = self.live.list.get(id).map_or("", |r| r.name.as_str());
                json!({"space": id, "name": name})
            };
            let entries: Vec<serde_json::Value> = self
                .rail_entries()
                .iter()
                .map(|e| match e {
                    Entry::Server(id) => server(id),
                    Entry::Folder { id, servers } => {
                        let f = self.rail_folders().iter().find(|f| f.id == *id);
                        json!({
                            "folder": id,
                            "name": f.map_or("", |f| f.name.as_str()),
                            "color": format!("#{:06x}", f.map_or(DEFAULT_COLOR, |f| f.color)),
                            "open": self.rail.open.contains(id),
                            "servers": servers.iter().map(server).collect::<Vec<_>>(),
                        })
                    }
                })
                .collect();
            let entries = serde_json::Value::Array(entries);
            if self.rail.logged.as_ref() == Some(&entries) {
                return;
            }
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{}", json!({"kind": "rail", "entries": entries}));
            }
            self.rail.logged = Some(entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn folder(id: &str, spaces: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: String::new(),
            color: DEFAULT_COLOR,
            spaces: ids(spaces),
        }
    }

    fn server(space: &str) -> Spot {
        Spot::Server {
            space: space.into(),
            folder: None,
        }
    }

    fn inside(space: &str, folder: &str) -> Spot {
        Spot::Server {
            space: space.into(),
            folder: Some(folder.into()),
        }
    }

    fn edge(child: &str, order: Option<&str>) -> ChildEdge {
        ChildEdge {
            child: child.into(),
            order: order.map(str::to_owned),
            ts: 0,
        }
    }

    #[test]
    fn servers_sort_as_element_sorts_them() {
        // A valid order first by its bytes, then by room ID; an invalid
        // order counts as none.
        let mut spaces = vec![
            edge("!d", None),
            edge("!b", Some("b")),
            edge("!a", None),
            edge("!z", Some("a")),
            edge("!y", Some("a")),
            edge("!bad", Some("\u{e9}")),
            edge("!long", Some(&"x".repeat(51))),
        ];
        channel_order::sort(&mut spaces);
        let order: Vec<&str> = spaces.iter().map(|e| e.child.as_str()).collect();
        assert_eq!(order, ["!y", "!z", "!b", "!a", "!bad", "!d", "!long"]);
    }

    #[test]
    fn a_folder_sits_at_its_first_server() {
        let order = ids(&["!a", "!b", "!c", "!d", "!e"]);
        // !d and !b are in a folder (listed in any order); !x isn't joined;
        // !b in a second folder stays in the first; a folder with nothing
        // joined doesn't show.
        let folders = [
            folder("f", &["!d", "!x", "!b"]),
            folder("g", &["!b", "!e"]),
            folder("h", &["!gone"]),
        ];
        assert_eq!(
            layout(&order, &folders),
            [
                Entry::Server("!a".into()),
                Entry::Folder {
                    id: "f".into(),
                    servers: ids(&["!b", "!d"]),
                },
                Entry::Server("!c".into()),
                Entry::Folder {
                    id: "g".into(),
                    servers: ids(&["!e"]),
                },
            ]
        );
        assert_eq!(
            flat(&layout(&order, &folders)),
            ids(&["!a", "!b", "!d", "!c", "!e"])
        );
    }

    fn rail(order: &[&str], folders: &[Folder]) -> Vec<Entry> {
        layout(&ids(order), folders)
    }

    fn go(
        entries: &[Entry],
        folders: &[Folder],
        held: Held,
        spot: Spot,
        zone: Zone,
    ) -> Option<(Vec<String>, Vec<Folder>)> {
        let open = BTreeSet::from(["f".to_owned()]);
        plan(entries, folders, &open, &held, &spot, zone, "new").map(|p| (p.order, p.folders))
    }

    #[test]
    fn a_server_moves_above_or_below_another() {
        let entries = rail(&["!a", "!b", "!c"], &[]);
        let held = || Held::Server("!c".into());
        assert_eq!(
            go(&entries, &[], held(), server("!a"), Zone::Above),
            Some((ids(&["!c", "!a", "!b"]), vec![]))
        );
        assert_eq!(
            go(&entries, &[], held(), server("!a"), Zone::Below),
            Some((ids(&["!a", "!c", "!b"]), vec![]))
        );
        // On itself: nothing.
        assert_eq!(go(&entries, &[], held(), server("!c"), Zone::Into), None);
    }

    #[test]
    fn dropping_a_server_on_another_makes_a_folder() {
        let entries = rail(&["!a", "!b", "!c"], &[]);
        let plan = plan(
            &entries,
            &[],
            &BTreeSet::new(),
            &Held::Server("!c".into()),
            &server("!a"),
            Zone::Into,
            "new",
        )
        .unwrap();
        assert_eq!(plan.mark, Mark::Into(server("!a")));
        // The folder takes the target's place, the dropped server after it.
        assert_eq!(plan.order, ids(&["!a", "!c", "!b"]));
        assert_eq!(plan.folders, [folder("new", &["!a", "!c"])]);
        assert_eq!(
            layout(&plan.order, &plan.folders),
            [
                Entry::Folder {
                    id: "new".into(),
                    servers: ids(&["!a", "!c"]),
                },
                Entry::Server("!b".into()),
            ]
        );
    }

    #[test]
    fn servers_go_into_folders_and_out_again() {
        let folders = [folder("f", &["!b", "!c"]), folder("g", &["!e", "!f"])];
        let entries = rail(&["!a", "!b", "!c", "!d", "!e", "!f"], &folders);
        // Onto a closed folder: last in it.
        let (order, after) = go(
            &entries,
            &folders,
            Held::Server("!a".into()),
            Spot::Folder("g".into()),
            Zone::Into,
        )
        .unwrap();
        assert_eq!(order, ids(&["!b", "!c", "!d", "!e", "!f", "!a"]));
        assert_eq!(after[1], folder("g", &["!e", "!f", "!a"]));
        // Between an open folder's servers.
        let (order, after) = go(
            &entries,
            &folders,
            Held::Server("!d".into()),
            inside("!c", "f"),
            Zone::Above,
        )
        .unwrap();
        assert_eq!(order, ids(&["!a", "!b", "!d", "!c", "!e", "!f"]));
        assert_eq!(after[0], folder("f", &["!b", "!c", "!d"]));
        // Under an open folder's icon: first in it.
        let (order, _) = go(
            &entries,
            &folders,
            Held::Server("!a".into()),
            Spot::Folder("f".into()),
            Zone::Below,
        )
        .unwrap();
        assert_eq!(order, ids(&["!a", "!b", "!c", "!d", "!e", "!f"]));
        // Out of its folder, above it: the order stays, the folder changes.
        let (order, after) = go(
            &entries,
            &folders,
            Held::Server("!b".into()),
            Spot::Folder("f".into()),
            Zone::Above,
        )
        .unwrap();
        assert_eq!(order, ids(&["!a", "!b", "!c", "!d", "!e", "!f"]));
        assert_eq!(after[0], folder("f", &["!c"]));
        assert_eq!(
            layout(&order, &after)[1],
            Entry::Server("!b".into()),
            "{after:?}"
        );
        // Into the middle of its own folder: nothing.
        assert_eq!(
            go(
                &entries,
                &folders,
                Held::Server("!b".into()),
                Spot::Folder("f".into()),
                Zone::Into,
            ),
            None
        );
    }

    #[test]
    fn a_folder_left_empty_goes() {
        let folders = [folder("g", &["!b"])];
        let entries = rail(&["!a", "!b", "!c"], &folders);
        let (order, after) = go(
            &entries,
            &folders,
            Held::Server("!b".into()),
            server("!c"),
            Zone::Below,
        )
        .unwrap();
        assert_eq!(order, ids(&["!a", "!c", "!b"]));
        assert!(after.is_empty(), "{after:?}");
        // Above its own closed folder: out of it, where it was.
        let (order, after) = go(
            &entries,
            &folders,
            Held::Server("!b".into()),
            Spot::Folder("g".into()),
            Zone::Above,
        )
        .unwrap();
        assert_eq!(order, ids(&["!a", "!b", "!c"]));
        assert!(after.is_empty());
    }

    #[test]
    fn a_folder_moves_with_its_servers() {
        let folders = [folder("f", &["!b", "!c"]), folder("g", &["!d", "!e"])];
        let entries = rail(&["!a", "!b", "!c", "!d", "!e"], &folders);
        let held = || Held::Folder("g".into());
        assert_eq!(
            go(&entries, &folders, held(), server("!a"), Zone::Above).map(|p| p.0),
            Some(ids(&["!d", "!e", "!a", "!b", "!c"]))
        );
        // Onto another folder's server: next to that folder, not in it.
        let plan = plan(
            &entries,
            &folders,
            &BTreeSet::from(["f".to_owned()]),
            &held(),
            &inside("!b", "f"),
            Zone::Above,
            "",
        )
        .unwrap();
        assert_eq!(plan.order, ids(&["!a", "!b", "!c", "!d", "!e"]));
        assert_eq!(plan.mark, Mark::Line(inside("!c", "f"), true));
        assert_eq!(
            go(
                &entries,
                &folders,
                held(),
                Spot::Folder("f".into()),
                Zone::Above
            )
            .map(|p| p.0),
            Some(ids(&["!a", "!d", "!e", "!b", "!c"]))
        );
        // Onto itself or its own servers: nothing.
        assert_eq!(
            go(
                &entries,
                &folders,
                held(),
                Spot::Folder("g".into()),
                Zone::Below
            ),
            None
        );
        assert_eq!(
            go(&entries, &folders, held(), inside("!d", "g"), Zone::Below),
            None
        );
    }

    /// The orders a drop writes, as `rail_drop` works them out.
    fn orders(order: &[ChildEdge], plan_order: &[&str], block: &[&str]) -> Vec<(String, String)> {
        let siblings: Vec<ChildEdge> = order
            .iter()
            .filter(|e| !block.contains(&e.child.as_str()))
            .cloned()
            .collect();
        let last = plan_order.iter().position(|s| s == block.last().unwrap());
        let before = last.and_then(|at| plan_order.get(at + 1)).copied();
        channel_order::reorder_block(&siblings, block, before)
    }

    #[test]
    fn a_drop_writes_what_element_reads_back_in_that_order() {
        // As Element makes them: no orders at all, sorted by room ID.
        let order = [edge("!a", None), edge("!b", None), edge("!c", None)];
        // !c to the top: only !c needs an order.
        let written = orders(&order, &["!c", "!a", "!b"], &["!c"]);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "!c");
        // !a below !b: !b, without an order, ahead of it gets one too.
        let written = orders(&order, &["!b", "!a", "!c"], &["!a"]);
        let mut after: Vec<ChildEdge> = order.to_vec();
        for (space, o) in &written {
            after.iter_mut().find(|e| e.child == *space).unwrap().order = Some(o.clone());
        }
        channel_order::sort(&mut after);
        let read: Vec<&str> = after.iter().map(|e| e.child.as_str()).collect();
        assert_eq!(read, ["!b", "!a", "!c"]);
        assert!(written.iter().all(|(_, o)| channel_order::valid(o)));
        // A folder's two servers move together between ordered neighbours.
        let order = [
            edge("!a", Some("a")),
            edge("!b", Some("b")),
            edge("!c", Some("c")),
            edge("!d", Some("d")),
        ];
        let written = orders(&order, &["!c", "!d", "!a", "!b"], &["!c", "!d"]);
        let mut after = order.to_vec();
        for (space, o) in &written {
            after.iter_mut().find(|e| e.child == *space).unwrap().order = Some(o.clone());
        }
        channel_order::sort(&mut after);
        let read: Vec<&str> = after.iter().map(|e| e.child.as_str()).collect();
        assert_eq!(read, ["!c", "!d", "!a", "!b"]);
        assert_eq!(written.len(), 2, "only the folder's servers: {written:?}");
    }

    mod live {
        use super::super::*;
        use crate::engine::{Event, RoomList, RoomSummary};

        /// Three servers as Element leaves them (no orders) and a channel.
        fn list(orders: &[(&str, &str)]) -> RoomList {
            let space = |id: &str, name: &str| RoomSummary {
                id: id.into(),
                name: name.into(),
                space: true,
                space_order: orders
                    .iter()
                    .find(|(s, _)| *s == id)
                    .map(|(_, o)| o.to_string()),
                ..RoomSummary::default()
            };
            RoomList {
                rooms: vec![
                    space("!a", "Alpha"),
                    space("!b", "Bravo"),
                    space("!c", "Charlie"),
                    RoomSummary {
                        id: "!r".into(),
                        name: "general".into(),
                        ..RoomSummary::default()
                    },
                ],
                ..RoomList::default()
            }
        }

        fn app() -> (App, async_channel::Receiver<Command>) {
            let (gomuks, _) = async_channel::bounded(1);
            let (tx, rx) = async_channel::bounded(64);
            let mut app = App::new(gomuks);
            app.sdk = Some(tx);
            app.logged_in = true;
            app.verified = true;
            app.connected = true;
            app.matrix_id = "@me:h".into();
            app.engine_event(Event::Rooms(list(&[])));
            (app, rx)
        }

        type Written = (i64, Vec<(String, String)>, Option<Vec<Folder>>);

        fn written(rx: &async_channel::Receiver<Command>) -> Option<Written> {
            std::iter::from_fn(|| rx.try_recv().ok()).find_map(|c| match c {
                Command::ArrangeRail {
                    id,
                    orders,
                    folders,
                } => Some((id, orders, folders)),
                _ => None,
            })
        }

        fn drag(app: &mut App, held: Held, spot: Spot, zone: Zone) {
            let _ = app.update(Msg::Rail(Input::Lift(held)));
            let _ = app.update(Msg::Rail(Input::Over(spot, zone)));
            let _ = app.update(Msg::Rail(Input::Drop));
        }

        fn shown(app: &App) -> Vec<Entry> {
            app.rail_entries()
        }

        fn server(space: &str) -> Entry {
            Entry::Server(space.into())
        }

        #[test]
        fn the_rail_follows_the_order_another_client_writes() {
            let (mut app, _rx) = app();
            assert_eq!(shown(&app), [server("!a"), server("!b"), server("!c")]);
            app.engine_event(Event::Rooms(list(&[("!c", "a"), ("!b", "b")])));
            assert_eq!(shown(&app), [server("!c"), server("!b"), server("!a")]);
        }

        #[test]
        fn a_folder_made_here_shows_at_once_and_stays_once_synced() {
            let (mut app, rx) = app();
            drag(
                &mut app,
                Held::Server("!c".into()),
                Spot::Server {
                    space: "!a".into(),
                    folder: None,
                },
                Zone::Into,
            );
            let (id, orders, folders) = written(&rx).expect("a rail write");
            // !a had no order, so it gets one ahead of !c's.
            let spaces: Vec<&str> = orders.iter().map(|(s, _)| s.as_str()).collect();
            assert_eq!(spaces, ["!a", "!c"]);
            assert!(orders[0].1 < orders[1].1);
            let folders = folders.expect("a new folder");
            assert_eq!(folders.len(), 1);
            assert_eq!(folders[0].spaces, ["!a", "!c"]);
            let folder = Entry::Folder {
                id: folders[0].id.clone(),
                servers: vec!["!a".into(), "!c".into()],
            };
            assert_eq!(shown(&app), [folder.clone(), server("!b")]);
            // The answer, then sync, bring it back: still shown.
            app.engine_event(Event::Done { id, result: Ok(()) });
            assert_eq!(shown(&app), [folder.clone(), server("!b")]);
            let orders: Vec<(&str, &str)> = orders
                .iter()
                .map(|(s, o)| (s.as_str(), o.as_str()))
                .collect();
            app.engine_event(Event::Rooms(list(&orders)));
            app.engine_event(Event::Folders(folders));
            assert!(app.rail.written.is_empty() && app.rail.writing.is_none());
            assert_eq!(shown(&app), [folder, server("!b")]);
        }

        #[test]
        fn a_failed_write_goes_back() {
            let (mut app, rx) = app();
            drag(
                &mut app,
                Held::Server("!c".into()),
                Spot::Server {
                    space: "!a".into(),
                    folder: None,
                },
                Zone::Above,
            );
            assert_eq!(shown(&app), [server("!c"), server("!a"), server("!b")]);
            let (id, _, folders) = written(&rx).unwrap();
            assert!(folders.is_none(), "only the order changes");
            app.engine_event(Event::Done {
                id,
                result: Err(crate::engine::Failure::Failed),
            });
            assert_eq!(shown(&app), [server("!a"), server("!b"), server("!c")]);
            assert!(!app.status.is_empty());
        }

        #[test]
        fn folder_settings_rename_and_recolour() {
            let (mut app, rx) = app();
            app.engine_event(Event::Folders(vec![Folder {
                id: "f".into(),
                name: String::new(),
                color: DEFAULT_COLOR,
                spaces: vec!["!b".into(), "!c".into()],
            }]));
            let _ = app.update(Msg::Rail(Input::Settings(Some("f".into()))));
            let _ = app.update(Msg::Rail(Input::Name(" Games ".into())));
            let _ = app.update(Msg::Rail(Input::Color(COLORS[3])));
            let _ = app.update(Msg::Rail(Input::Save));
            let (_, orders, folders) = written(&rx).unwrap();
            assert!(orders.is_empty());
            let folders = folders.unwrap();
            assert_eq!(
                (folders[0].name.as_str(), folders[0].color),
                ("Games", COLORS[3])
            );
            assert_eq!(app.rail_folders(), folders);
            assert!(app.rail.settings.is_none());
        }

        #[test]
        fn rearranging_needs_a_session() {
            let (mut app, rx) = app();
            app.connected = false;
            drag(
                &mut app,
                Held::Server("!c".into()),
                Spot::Server {
                    space: "!a".into(),
                    folder: None,
                },
                Zone::Above,
            );
            assert!(written(&rx).is_none());
            assert_eq!(shown(&app), [server("!a"), server("!b"), server("!c")]);
        }
    }

    #[test]
    fn zones_by_halves_or_thirds() {
        assert_eq!(Zone::of(0, 3), Zone::Above);
        assert_eq!(Zone::of(1, 3), Zone::Into);
        assert_eq!(Zone::of(2, 3), Zone::Below);
        assert_eq!(Zone::of(1, 2), Zone::Below);
    }
}
