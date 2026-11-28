//! Channel order (ROADMAP N5): a space's sidebar sorted the way the spec says
//! (and Element shows a space), and dragging channels and categories to
//! reorder them, as Discord's Manage Channels allows.
//!
//! A server is a space; its categories are the joined spaces it lists, each
//! with its own channels. Siblings sort by their `m.space.child` edge's
//! `order` (printable ASCII, compared byte by byte), those without one after
//! those with, then by the edge event's `origin_server_ts`, then room ID.
//! Dropping a channel gives its edge an `order` between its new neighbours',
//! renumbering them only when there's no room; dropping it in another
//! category moves the edge there (engine/arrange.rs). Dragging is offered
//! only where the power levels let this account send `m.space.child`.
//!
//! What's written shows at once: the sidebar keeps each edit until sync
//! brings that edge back changed, or the edit fails.
use crate::{
    App, Msg, Request, Section,
    engine::{ChildEdge, Command, arrange::Change},
    rooms::Rooms,
};
use iced::Task;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The longest `order` the spec allows.
const LONGEST: usize = 50;
/// Orders are digits in base 95: the printable ASCII characters.
const FIRST: u8 = 0x20;
const BASE: u8 = 95;

/// An `order` the spec allows: at most 50 printable ASCII characters.
pub fn valid(order: &str) -> bool {
    order.len() <= LONGEST && order.bytes().all(|b| (FIRST..FIRST + BASE).contains(&b))
}

/// The spec's sort key: a valid `order` first, by its bytes; then the edge
/// event's timestamp; then the room ID.
fn key(edge: &ChildEdge) -> (bool, &str, i64, &str) {
    let order = edge.order.as_deref().filter(|o| valid(o));
    (order.is_none(), order.unwrap_or(""), edge.ts, &edge.child)
}

/// Edges in the spec's order.
pub fn sort(edges: &mut [ChildEdge]) {
    edges.sort_by(|a, b| key(a).cmp(&key(b)));
}

/// Digits of an order, without trailing zeros (spaces): a string sorts as
/// the base-95 fraction they make, and trailing spaces don't change it.
fn digits(order: &str) -> Vec<u8> {
    let mut d: Vec<u8> = order.bytes().map(|b| b - FIRST).collect();
    while d.last() == Some(&0) {
        d.pop();
    }
    d
}

/// The middle of two fractions `a < b` (`None`: 1), as short as it can be,
/// without a trailing zero. After rocicorp's fractional indexing.
fn midpoint(mut a: &[u8], mut b: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        if let Some(hi) = b {
            // Copy the common prefix, reading `a` as padded with zeros.
            let n = hi
                .iter()
                .enumerate()
                .take_while(|&(i, d)| a.get(i).copied().unwrap_or(0) == *d)
                .count();
            if n > 0 {
                out.extend_from_slice(&hi[..n]);
                a = a.get(n..).unwrap_or_default();
                b = Some(&hi[n..]);
                continue;
            }
        }
        let low = a.first().copied().unwrap_or(0);
        let high = b.map_or(BASE, |hi| hi[0]);
        if high - low > 1 {
            out.push((low + high).div_ceil(2));
            return out;
        }
        // Consecutive digits: take `b`'s first if more of it follows, or
        // go one digit deeper above `a`.
        if let Some(hi) = b.filter(|hi| hi.len() > 1) {
            out.push(hi[0]);
            return out;
        }
        out.push(low);
        a = a.get(1..).unwrap_or_default();
        b = None;
    }
}

/// An order strictly between `lo` and `hi` (`None`: no bound), if there's
/// room for one within the spec's 50 characters.
pub fn between(lo: Option<&str>, hi: Option<&str>) -> Option<String> {
    let a = lo.map(digits).unwrap_or_default();
    let b = hi.map(digits);
    if b.as_ref().is_some_and(|b| *b <= a) {
        return None;
    }
    let mid = midpoint(&a, b.as_deref());
    (mid.len() <= LONGEST).then(|| mid.into_iter().map(|d| (d + FIRST) as char).collect())
}

/// `n` increasing orders strictly between `lo` and `hi`, spread out so the
/// next drop between any two still has room.
pub fn spread(lo: Option<&str>, hi: Option<&str>, n: usize) -> Option<Vec<String>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let mid = between(lo, hi)?;
    let mut orders = spread(lo, Some(&mid), n / 2)?;
    let after = spread(Some(&mid), hi, n - n / 2 - 1)?;
    orders.push(mid);
    orders.extend(after);
    Some(orders)
}

/// The new orders that put `child` just before `before` (or last) among
/// `siblings`, which are in the spec's order and don't include it. Only
/// `child` gets one where its neighbours leave room; otherwise as few of
/// them as possible are renumbered. Siblings without an order sort after
/// every one with, so those ahead of `child` get one too.
pub fn reorder(siblings: &[ChildEdge], child: &str, before: Option<&str>) -> Vec<(String, String)> {
    let at = before
        .and_then(|b| siblings.iter().position(|s| s.child == b))
        .unwrap_or(siblings.len());
    let mut list: Vec<(&str, Option<&str>)> = siblings
        .iter()
        .map(|s| (s.child.as_str(), s.order.as_deref().filter(|o| valid(o))))
        .collect();
    list.insert(at, (child, None));
    let order = |i: usize| list.get(i).and_then(|(_, o)| *o);
    // Everything from the first sibling without an order up to `child`.
    let (mut first, mut last) = (
        list[..at]
            .iter()
            .position(|(_, o)| o.is_none())
            .unwrap_or(at),
        at,
    );
    loop {
        let lo = first.checked_sub(1).and_then(order);
        let hi = order(last + 1);
        if let Some(orders) = spread(lo, hi, last - first + 1) {
            return list[first..=last]
                .iter()
                .zip(orders)
                .filter(|((_, old), new)| *old != Some(new.as_str()))
                .map(|((id, _), new)| ((*id).to_owned(), new))
                .collect();
        }
        // No room: renumber a neighbour too, the one after while it has an
        // order, then the one before.
        if hi.is_some() {
            last += 1;
        } else if first > 0 {
            first -= 1;
        } else {
            // Unreachable: with no bounds there is always room.
            return Vec::new();
        }
    }
}

/// A space's sidebar group: the server's own channels, or a category's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    /// The space whose edges list this group's channels.
    pub space: String,
    pub category: bool,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// The room shown: the newest, for an upgraded child.
    pub room: String,
    /// The child `space`'s edge names, or None for a room under a deeper
    /// space, which moves only from there.
    pub edge: Option<String>,
}

impl Rooms {
    /// A space's children in the spec's order. Children whose edge isn't
    /// known (the demo, the gomuks engine) come last, by name.
    pub fn siblings(&self, space: &str) -> Vec<ChildEdge> {
        let children = self.children.get(space);
        let mut known: Vec<ChildEdge> = self
            .edges
            .get(space)
            .into_iter()
            .flatten()
            .filter(|e| children.is_some_and(|c| c.contains(&e.child)))
            .cloned()
            .collect();
        sort(&mut known);
        let listed: BTreeSet<&str> = known.iter().map(|e| e.child.as_str()).collect();
        let mut unknown: Vec<&String> = children
            .into_iter()
            .flatten()
            .filter(|c| !listed.contains(c.as_str()))
            .collect();
        let name = |id: &str| {
            self.list
                .get(self.newest(id))
                .map(|r| r.name.to_lowercase())
        };
        unknown.sort_by_cached_key(|id| (name(id), id.to_string()));
        known.extend(unknown.into_iter().map(|child| ChildEdge {
            child: child.clone(),
            ..ChildEdge::default()
        }));
        known
    }

    /// A space's channels as its sidebar groups them: its own, then a
    /// category for each joined space it lists, each in the spec's order.
    /// A room under a deeper space joins the category it's reached through
    /// first, and a room listed twice shows once, nearest the top.
    pub fn groups(&self, space: &str) -> Vec<Group> {
        let mut seen = BTreeSet::from([space.to_owned()]);
        let mut groups = vec![Group {
            space: space.to_owned(),
            category: false,
            channels: Vec::new(),
        }];
        // Rooms and categories the server lists itself.
        for edge in self.siblings(space) {
            let id = self.newest(&edge.child);
            let Some(r) = self.list.get(id) else {
                continue;
            };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            if r.space {
                groups.push(Group {
                    space: id.to_owned(),
                    category: true,
                    channels: Vec::new(),
                });
            } else {
                groups[0].channels.push(Channel {
                    room: id.to_owned(),
                    edge: Some(edge.child),
                });
            }
        }
        // Each category's own rooms, then deeper spaces' breadth first.
        let mut deeper = VecDeque::new();
        for (index, group) in groups.iter_mut().enumerate().skip(1) {
            for edge in self.siblings(&group.space) {
                let id = self.newest(&edge.child);
                match self.list.get(id) {
                    Some(r) if r.space => deeper.push_back((index, edge.child)),
                    Some(_) if seen.insert(id.to_owned()) => group.channels.push(Channel {
                        room: id.to_owned(),
                        edge: Some(edge.child),
                    }),
                    _ => {}
                }
            }
        }
        while let Some((index, sub)) = deeper.pop_front() {
            if !seen.insert(self.newest(&sub).to_owned()) {
                continue;
            }
            for edge in self.siblings(self.newest(&sub)) {
                let id = self.newest(&edge.child);
                match self.list.get(id) {
                    Some(r) if r.space => deeper.push_back((index, edge.child)),
                    Some(_) if seen.insert(id.to_owned()) => groups[index].channels.push(Channel {
                        room: id.to_owned(),
                        edge: None,
                    }),
                    _ => {}
                }
            }
        }
        groups
    }

    /// The sidebar's order of a space's channels: each group's text
    /// channels, then its voice channels.
    pub fn flatten(&self, groups: &[Group]) -> Vec<String> {
        let voice = |id: &str| self.list.get(id).is_some_and(|r| r.voice);
        groups
            .iter()
            .flat_map(|g| {
                let text = g.channels.iter().filter(|c| !voice(&c.room));
                let calls = g.channels.iter().filter(|c| voice(&c.room));
                text.chain(calls).map(|c| c.room.clone())
            })
            .collect()
    }

    /// Show these edits until sync brings their edges back changed.
    pub fn arrange(&mut self, request: i64, changes: &[Change]) {
        self.arranging
            .start(request, changes, &self.edges, &self.children);
        self.show_arranged();
    }

    /// An edit finished; one that failed stops showing.
    pub fn arranged(&mut self, request: i64, ok: bool) {
        if !ok && self.arranging.forget(request) {
            self.show_arranged();
        }
    }

    fn show_arranged(&mut self) {
        let (children, edges) = self.arranging.shown();
        self.children = children;
        self.edges = edges;
        self.hierarchy_revision += 1;
        self.sort_rooms();
    }
}

/// Edges written here and not yet back from sync.
#[derive(Default)]
pub struct Arranging {
    /// The engine's latest children and edges, without what's pending.
    synced_children: BTreeMap<String, BTreeSet<String>>,
    synced_edges: BTreeMap<String, Vec<ChildEdge>>,
    pending: Vec<Pending>,
}

struct Pending {
    request: i64,
    space: String,
    child: String,
    /// The order it gets, or None: it leaves `space`.
    order: Option<String>,
    /// The edge's timestamp when written (None: there was none). Any
    /// change to the edge since, ours or anyone's, ends this.
    was: Option<i64>,
}

fn edge_ts(edges: &BTreeMap<String, Vec<ChildEdge>>, space: &str, child: &str) -> Option<i64> {
    edges
        .get(space)?
        .iter()
        .find(|e| e.child == child)
        .map(|e| e.ts)
}

impl Arranging {
    /// The engine's children and edges, with what's pending shown.
    pub fn synced(
        &mut self,
        children: BTreeMap<String, BTreeSet<String>>,
        edges: BTreeMap<String, Vec<ChildEdge>>,
    ) -> (
        BTreeMap<String, BTreeSet<String>>,
        BTreeMap<String, Vec<ChildEdge>>,
    ) {
        self.pending
            .retain(|p| edge_ts(&edges, &p.space, &p.child) == p.was);
        self.synced_children = children;
        self.synced_edges = edges;
        self.shown()
    }

    fn start(
        &mut self,
        request: i64,
        changes: &[Change],
        edges: &BTreeMap<String, Vec<ChildEdge>>,
        children: &BTreeMap<String, BTreeSet<String>>,
    ) {
        // Before the first room list (the demo), what's shown is all there is.
        if self.synced_children.is_empty() && self.synced_edges.is_empty() {
            self.synced_children = children.clone();
            self.synced_edges = edges.clone();
        }
        let mut add = |space: &str, child: &str, order: Option<&str>| {
            self.pending.push(Pending {
                request,
                space: space.to_owned(),
                child: child.to_owned(),
                order: order.map(str::to_owned),
                was: edge_ts(&self.synced_edges, space, child),
            })
        };
        for change in changes {
            match change {
                Change::Order {
                    space,
                    child,
                    order,
                } => add(space, child, Some(order)),
                Change::Move {
                    from,
                    to,
                    child,
                    order,
                } => {
                    add(to, child, Some(order));
                    add(from, child, None);
                }
            }
        }
    }

    /// Stop showing a request's edits; true if it had any.
    fn forget(&mut self, request: i64) -> bool {
        let before = self.pending.len();
        self.pending.retain(|p| p.request != request);
        self.pending.len() != before
    }

    fn shown(
        &self,
    ) -> (
        BTreeMap<String, BTreeSet<String>>,
        BTreeMap<String, Vec<ChildEdge>>,
    ) {
        let mut children = self.synced_children.clone();
        let mut edges = self.synced_edges.clone();
        for p in &self.pending {
            let list = edges.entry(p.space.clone()).or_default();
            let at = list.iter().position(|e| e.child == p.child);
            match (&p.order, at) {
                (Some(order), Some(at)) => list[at].order = Some(order.clone()),
                (Some(order), None) => list.push(ChildEdge {
                    child: p.child.clone(),
                    order: Some(order.clone()),
                    ts: i64::MAX,
                }),
                (None, Some(at)) => {
                    list.remove(at);
                }
                (None, None) => {}
            }
            let set = children.entry(p.space.clone()).or_default();
            if p.order.is_some() {
                set.insert(p.child.clone());
            } else {
                set.remove(&p.child);
            }
        }
        (children, edges)
    }
}

/// What's being dragged.
#[derive(Clone, Debug, PartialEq)]
pub enum Held {
    /// A channel, by the edge in `space` that lists it.
    Channel {
        room: String,
        space: String,
        edge: String,
        voice: bool,
    },
    /// A category: a space listed by the server `parent`.
    Category { space: String, parent: String },
}

/// A sidebar row a drag can be over.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    /// A channel row in `space`'s group (`edge` as in [`Channel`]).
    Channel {
        room: String,
        space: String,
        edge: Option<String>,
        voice: bool,
    },
    /// A category's heading.
    Category { space: String },
    /// The server's own TEXT CHANNELS or VOICE CHANNELS heading.
    Heading { space: String, voice: bool },
}

/// Where the sidebar draws the drop line.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Above(Spot),
    Below(Spot),
    /// Under a category's last row.
    AfterCategory(String),
}

/// Where a drop would put what's held: in `space` just before the edge
/// `before`, or last.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub space: String,
    pub before: Option<String>,
    pub line: Line,
}

#[derive(Clone, Debug)]
pub enum Input {
    /// A row was pressed and moved far enough to drag it.
    Lift(Held),
    /// The pointer is over a row, in its lower half or not.
    Over(Spot, bool),
    /// The pointer left a row.
    Leave(Spot),
    /// The pointer left the window.
    Away,
    /// The mouse button came up.
    Drop,
}

/// The drag in progress.
#[derive(Clone, Debug, Default)]
pub struct Drag {
    pub held: Option<Held>,
    pub over: Option<(Spot, bool)>,
}

impl App {
    /// This account may send `m.space.child` in `space`.
    pub fn may_arrange(&self, space: &str) -> bool {
        self.logged_in
            && self.connected
            && self
                .live
                .list
                .get(space)
                .is_some_and(|r| r.space && r.posting.can_link_channel(&self.matrix_id))
    }

    /// The drag in progress, if any.
    pub fn held(&self) -> Option<&Held> {
        self.order.held.as_ref()
    }

    pub fn arrange_input(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Lift(held) => {
                let space = match &held {
                    Held::Channel { space, .. } => space,
                    Held::Category { parent, .. } => parent,
                };
                if self.may_arrange(space) && self.order.held.is_none() {
                    self.order = Drag {
                        held: Some(held),
                        over: None,
                    };
                }
            }
            Input::Over(spot, lower) if self.order.held.is_some() => {
                self.order.over = Some((spot, lower));
            }
            Input::Leave(spot) => {
                if self.order.over.as_ref().is_some_and(|(s, _)| *s == spot) {
                    self.order.over = None;
                }
            }
            Input::Drop => {
                let landing = self.landing();
                let held = std::mem::take(&mut self.order).held;
                if let (Some(held), Some(landing)) = (held, landing) {
                    self.drop_at(held, landing);
                }
            }
            Input::Away => self.order.over = None,
            Input::Over(..) => {}
        }
        Task::none()
    }

    /// Where a drop would put what's held, from the row under the pointer.
    pub fn landing(&self) -> Option<Landing> {
        let held = self.order.held.as_ref()?;
        let (spot, lower) = self.order.over.as_ref()?;
        let rooms = &self.live;
        // The edge after `edge` in `space`, skipping what's held.
        let next = |space: &str, edge: &str, skip: &str| {
            let siblings = rooms.siblings(space);
            siblings
                .iter()
                .skip_while(|s| s.child != edge)
                .skip(1)
                .find(|s| s.child != skip)
                .map(|s| s.child.clone())
        };
        let first = |space: &str, skip: &str| {
            rooms
                .siblings(space)
                .into_iter()
                .find(|s| s.child != skip)
                .map(|s| s.child)
        };
        match (held, spot) {
            (
                Held::Channel {
                    space: from,
                    edge: held,
                    voice,
                    ..
                },
                Spot::Channel {
                    space,
                    edge: Some(edge),
                    voice: kind,
                    ..
                },
            ) if voice == kind && self.may_land(from, space) => {
                if edge == held {
                    // On itself: it stays where it is.
                    return None;
                }
                let (before, line) = if *lower {
                    (next(space, edge, held), Line::Below(spot.clone()))
                } else {
                    (Some(edge.clone()), Line::Above(spot.clone()))
                };
                Some(Landing {
                    space: space.clone(),
                    before,
                    line,
                })
            }
            (
                Held::Channel {
                    space: from, edge, ..
                },
                Spot::Category { space },
            ) if self.may_land(from, space) => Some(Landing {
                space: space.clone(),
                before: first(space, edge),
                line: Line::Below(spot.clone()),
            }),
            (
                Held::Channel {
                    space: from,
                    edge,
                    voice,
                    ..
                },
                Spot::Heading { space, voice: kind },
            ) if voice == kind && self.may_land(from, space) => Some(Landing {
                space: space.clone(),
                before: first(space, edge),
                line: Line::Below(spot.clone()),
            }),
            (
                Held::Category {
                    space: held,
                    parent,
                },
                Spot::Category { space },
            ) if held != space && self.may_arrange(parent) => {
                let groups = rooms.groups(parent);
                let categories: Vec<&str> = groups
                    .iter()
                    .filter(|g| g.category)
                    .map(|g| g.space.as_str())
                    .collect();
                let at = categories.iter().position(|c| c == space)?;
                if !*lower {
                    return Some(Landing {
                        space: parent.clone(),
                        before: Some(space.clone()),
                        line: Line::Above(spot.clone()),
                    });
                }
                // After this category: before the next one, or last.
                let after = categories[at + 1..].iter().find(|c| **c != held);
                Some(Landing {
                    space: parent.clone(),
                    before: next(parent, space, held),
                    line: match after {
                        Some(c) => Line::Above(Spot::Category {
                            space: (*c).to_owned(),
                        }),
                        None => Line::AfterCategory(space.clone()),
                    },
                })
            }
            _ => None,
        }
    }

    /// A channel from `from` may land in `to`: both edges can be written.
    fn may_land(&self, from: &str, to: &str) -> bool {
        self.may_arrange(from) && self.may_arrange(to)
    }

    fn drop_at(&mut self, held: Held, landing: Landing) {
        let (from, child) = match &held {
            Held::Channel { space, edge, .. } => (space.clone(), edge.clone()),
            Held::Category { space, parent } => (parent.clone(), space.clone()),
        };
        let to = landing.space;
        let siblings: Vec<ChildEdge> = self
            .live
            .siblings(&to)
            .into_iter()
            .filter(|s| s.child != child)
            .collect();
        // Dropped back where it was: nothing to write.
        if from == to {
            let now = self.live.siblings(&to);
            let at = now.iter().position(|s| s.child == child);
            let next = at.and_then(|at| now.get(at + 1)).map(|s| &s.child);
            if at.is_some() && next == landing.before.as_ref() {
                return;
            }
        }
        let mut changes: Vec<Change> = reorder(&siblings, &child, landing.before.as_deref())
            .into_iter()
            .map(|(id, order)| {
                if id == child && from != to {
                    Change::Move {
                        from: from.clone(),
                        to: to.clone(),
                        child: id,
                        order,
                    }
                } else {
                    Change::Order {
                        space: to.clone(),
                        child: id,
                        order,
                    }
                }
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        // Neighbours first, so the dragged one never shows out of place.
        changes.sort_by_key(|c| match c {
            Change::Order { child: id, .. } | Change::Move { child: id, .. } => *id == child,
        });
        let sent = changes.clone();
        self.command(
            move |id| Command::Arrange { id, changes: sent },
            Request::Arrange,
        );
        // Queued: show it at once.
        if matches!(self.pending.get(&self.seq), Some(Request::Arrange)) {
            self.live.arrange(self.seq, &changes);
        }
    }

    pub fn arranged(&mut self, request: i64, ok: bool) {
        self.live.arranged(request, ok);
        if !ok {
            self.status = "Couldn't move it there. Try again.".into();
        }
    }

    /// While something's held, the button coming up anywhere drops it.
    pub fn order_subscription(&self) -> iced::Subscription<Msg> {
        if self.order.held.is_none() {
            return iced::Subscription::none();
        }
        iced::event::listen_with(|event, _, _| match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Msg::Order(Input::Drop))
            }
            // The pointer left the window: nothing is under it.
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Msg::Order(Input::Away)),
            _ => None,
        })
    }

    /// The open server's groups, when it's a space.
    pub fn open_groups(&self) -> Option<(&str, &[Group])> {
        let Section::Space(space) = &self.section else {
            return None;
        };
        let groups = self.rooms().groups.get(space)?;
        Some((space, groups))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(child: &str, order: Option<&str>, ts: i64) -> ChildEdge {
        ChildEdge {
            child: child.into(),
            order: order.map(str::to_owned),
            ts,
        }
    }

    fn ids(edges: &[ChildEdge]) -> Vec<&str> {
        edges.iter().map(|e| e.child.as_str()).collect()
    }

    #[test]
    fn siblings_sort_as_the_spec_says() {
        let mut edges = vec![
            edge("!late", None, 30),
            edge("!b", Some("b"), 50),
            edge("!early", None, 10),
            edge("!a2", Some("a"), 20),
            edge("!a1", Some("a"), 20),
            // Not printable ASCII, or too long: as if it had none.
            edge("!bad", Some("é"), 5),
            edge("!long", Some(&"x".repeat(51)), 40),
            edge("!space", Some(" "), 99),
            edge("!tilde", Some("~"), 1),
        ];
        sort(&mut edges);
        assert_eq!(
            ids(&edges),
            [
                "!space", "!a1", "!a2", "!b", "!tilde", "!bad", "!early", "!late", "!long"
            ]
        );
    }

    #[test]
    fn an_order_between_two_sorts_between_them() {
        let cases: &[(Option<&str>, Option<&str>)] = &[
            (None, None),
            (Some("a"), Some("b")),
            (Some("a"), Some("a!")),
            (Some("a"), Some("c")),
            (Some("a~~"), Some("b")),
            (None, Some("!")),
            (None, Some(" !")),
            (Some("~"), None),
            (Some("~~~~"), None),
            (Some("a  "), Some("a !")),
            (Some("P"), Some("P ~")),
        ];
        for &(lo, hi) in cases {
            let mid = between(lo, hi).unwrap_or_else(|| panic!("room between {lo:?} and {hi:?}"));
            assert!(valid(&mid) && !mid.ends_with(' '), "{mid:?}");
            assert!(lo.is_none_or(|lo| lo < mid.as_str()), "{lo:?} < {mid:?}");
            assert!(hi.is_none_or(|hi| mid.as_str() < hi), "{mid:?} < {hi:?}");
        }
        assert_eq!(between(Some("a"), Some("c")).as_deref(), Some("b"));
        assert_eq!(between(None, None).as_deref(), Some("P"));
    }

    #[test]
    fn no_order_fits_between_equals_or_past_fifty_characters() {
        assert_eq!(between(Some("a"), Some("a")), None);
        // "a" and "a " are different strings with nothing between them.
        assert_eq!(between(Some("a"), Some("a ")), None);
        assert_eq!(between(Some("b"), Some("a")), None);
        let lo = format!("a{}", " ".repeat(48) + "!");
        let hi = format!("a{}", " ".repeat(48) + "\"");
        assert_eq!(lo.len(), 50);
        assert_eq!(between(Some(&lo), Some(&hi)), None);
    }

    #[test]
    fn spread_orders_increase_and_stay_short() {
        let orders = spread(None, None, 40).unwrap();
        assert_eq!(orders.len(), 40);
        assert!(orders.windows(2).all(|w| w[0] < w[1]));
        assert!(orders.iter().all(|o| valid(o) && o.len() <= 2));
        let tight = spread(Some("a"), Some("b"), 5).unwrap();
        assert!(tight.windows(2).all(|w| w[0] < w[1]));
        assert!(tight.iter().all(|o| "a" < o.as_str() && o.as_str() < "b"));
    }

    /// The order after applying `changes` to `siblings` plus `child`.
    fn after(siblings: &[ChildEdge], child: &str, changes: &[(String, String)]) -> Vec<String> {
        let mut all: Vec<ChildEdge> = siblings.to_vec();
        all.push(edge(child, None, i64::MAX));
        for (id, order) in changes {
            all.iter_mut().find(|e| e.child == *id).unwrap().order = Some(order.clone());
        }
        sort(&mut all);
        all.into_iter().map(|e| e.child).collect()
    }

    #[test]
    fn a_drop_between_ordered_neighbours_moves_only_the_dropped_child() {
        let siblings = [
            edge("!a", Some("a"), 1),
            edge("!b", Some("b"), 2),
            edge("!c", Some("c"), 3),
        ];
        let changes = reorder(&siblings, "!x", Some("!b"));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "!x");
        assert_eq!(after(&siblings, "!x", &changes), ["!a", "!x", "!b", "!c"]);
        // First and last.
        let changes = reorder(&siblings, "!x", Some("!a"));
        assert_eq!(changes.len(), 1);
        assert_eq!(after(&siblings, "!x", &changes), ["!x", "!a", "!b", "!c"]);
        let changes = reorder(&siblings, "!x", None);
        assert_eq!(changes.len(), 1);
        assert_eq!(after(&siblings, "!x", &changes), ["!a", "!b", "!c", "!x"]);
    }

    #[test]
    fn siblings_without_an_order_ahead_of_the_drop_get_one() {
        // Nobody has an order yet, as in a space Element made.
        let siblings = [
            edge("!a", None, 1),
            edge("!b", None, 2),
            edge("!c", None, 3),
            edge("!d", None, 4),
        ];
        let changes = reorder(&siblings, "!x", Some("!c"));
        let moved: Vec<&str> = changes.iter().map(|(id, _)| id.as_str()).collect();
        // Only those up to the drop; the rest keep sorting by time after them.
        assert_eq!(moved, ["!a", "!b", "!x"]);
        assert_eq!(
            after(&siblings, "!x", &changes),
            ["!a", "!b", "!x", "!c", "!d"]
        );
        // Last, after ordered and unordered siblings alike.
        let siblings = [edge("!a", Some("m"), 1), edge("!b", None, 2)];
        let changes = reorder(&siblings, "!x", None);
        assert_eq!(after(&siblings, "!x", &changes), ["!a", "!b", "!x"]);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn without_room_the_neighbours_are_renumbered() {
        // Equal orders, sorted by time: nothing fits between them.
        let siblings = [
            edge("!a", Some("a"), 1),
            edge("!b", Some("a"), 2),
            edge("!c", Some("a"), 3),
        ];
        let changes = reorder(&siblings, "!x", Some("!b"));
        assert_eq!(after(&siblings, "!x", &changes), ["!a", "!x", "!b", "!c"]);
        assert!(changes.iter().all(|(_, o)| valid(o)));
        // "a" and "a " leave no room either.
        let siblings = [edge("!a", Some("a"), 1), edge("!b", Some("a "), 2)];
        let changes = reorder(&siblings, "!x", Some("!b"));
        assert_eq!(after(&siblings, "!x", &changes), ["!a", "!x", "!b"]);
    }

    use crate::engine::{Event, RoomList, RoomSummary};
    use crate::permissions::StateDoc;

    fn doc(content: serde_json::Value) -> Option<StateDoc> {
        Some(StateDoc {
            event_id: "$doc".into(),
            sender: "@owner:h".into(),
            content,
            redacted: false,
        })
    }

    /// A server `!s` with `#b`, `#a` and a voice channel, ordered by edge,
    /// and a category `!cat` with `#d` and `#c`.
    fn list(power: i64) -> RoomList {
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            create: doc(serde_json::json!({"room_version": "11"})),
            power: doc(serde_json::json!({"users": {"@me:h": power}, "state_default": 50})),
            ..RoomSummary::default()
        };
        let space = |id: &str, name: &str, edges: Vec<ChildEdge>| RoomSummary {
            space: true,
            child_order: edges,
            ..room(id, name)
        };
        let rooms = vec![
            space(
                "!s",
                "Server",
                vec![
                    edge("!a", Some("m"), 5),
                    edge("!b", Some("c"), 6),
                    edge("!v", None, 1),
                    edge("!cat", Some("x"), 2),
                ],
            ),
            space(
                "!cat",
                "Later",
                vec![edge("!c", None, 9), edge("!d", None, 8)],
            ),
            room("!a", "alpha"),
            room("!b", "bravo"),
            room("!c", "charlie"),
            room("!d", "delta"),
            RoomSummary {
                voice: true,
                ..room("!v", "lounge")
            },
        ];
        let children = rooms
            .iter()
            .filter(|r| r.space)
            .map(|r| {
                (
                    r.id.clone(),
                    r.child_order.iter().map(|e| e.child.clone()).collect(),
                )
            })
            .collect();
        RoomList {
            rooms,
            children,
            invites: Vec::new(),
        }
    }

    fn app(power: i64) -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@me:h".into();
        app.engine_event(Event::Rooms(list(power)));
        app.section = Section::Space("!s".into());
        (app, rx)
    }

    fn arranged(rx: &async_channel::Receiver<Command>) -> Option<(i64, Vec<Change>)> {
        std::iter::from_fn(|| rx.try_recv().ok()).find_map(|c| match c {
            Command::Arrange { id, changes } => Some((id, changes)),
            _ => None,
        })
    }

    fn channel(room: &str, space: &str) -> Held {
        Held::Channel {
            room: room.into(),
            space: space.into(),
            edge: room.into(),
            voice: false,
        }
    }

    fn spot(room: &str, space: &str) -> Spot {
        Spot::Channel {
            room: room.into(),
            space: space.into(),
            edge: Some(room.into()),
            voice: false,
        }
    }

    fn drag(app: &mut App, held: Held, over: Spot, lower: bool) {
        let _ = app.update(Msg::Order(Input::Lift(held)));
        let _ = app.update(Msg::Order(Input::Over(over, lower)));
        let _ = app.update(Msg::Order(Input::Drop));
    }

    #[test]
    fn a_space_sorts_by_order_with_its_categories_after_its_own_channels() {
        let (app, _rx) = app(100);
        let groups = &app.live.groups["!s"];
        let shape: Vec<(&str, bool, Vec<&str>)> = groups
            .iter()
            .map(|g| {
                let rooms = g.channels.iter().map(|c| c.room.as_str()).collect();
                (g.space.as_str(), g.category, rooms)
            })
            .collect();
        // `#b` has the lower order; `#d`'s edge is older than `#c`'s.
        assert_eq!(
            shape,
            [
                ("!s", false, vec!["!b", "!a", "!v"]),
                ("!cat", true, vec!["!d", "!c"]),
            ]
        );
        // Text channels, then voice, group by group.
        assert_eq!(
            app.live.ordered_spaces["!s"],
            ["!b", "!a", "!v", "!d", "!c"]
        );
    }

    #[test]
    fn dropping_a_channel_above_another_writes_only_its_order() {
        let (mut app, rx) = app(100);
        drag(&mut app, channel("!a", "!s"), spot("!b", "!s"), false);
        let (id, changes) = arranged(&rx).expect("an arrange command");
        assert_eq!(changes.len(), 1);
        let Change::Order {
            space,
            child,
            order,
        } = &changes[0]
        else {
            panic!("{changes:?}");
        };
        assert_eq!((space.as_str(), child.as_str()), ("!s", "!a"));
        assert!(valid(order) && order.as_str() < "c");
        // Shown at once, and still while sync hasn't brought it back.
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!a", "!b"]);
        app.engine_event(Event::Rooms(list(100)));
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!a", "!b"]);
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!a", "!b"]);
        // Sync brings the edge back: the engine's state is shown.
        let mut synced = list(100);
        synced.rooms[0].child_order[0] = edge("!a", Some(order), 50);
        app.engine_event(Event::Rooms(synced));
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!a", "!b"]);
        assert!(app.live.arranging.pending.is_empty());
    }

    #[test]
    fn a_failed_move_goes_back() {
        let (mut app, rx) = app(100);
        drag(&mut app, channel("!a", "!s"), spot("!b", "!s"), false);
        let (id, _) = arranged(&rx).unwrap();
        app.engine_event(Event::Done {
            id,
            result: Err(crate::engine::Failure::Failed),
        });
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!b", "!a"]);
        assert!(!app.status.is_empty());
    }

    #[test]
    fn dropping_a_channel_in_another_category_moves_its_edge() {
        let (mut app, rx) = app(100);
        // Below `#d`, the category's first channel.
        drag(&mut app, channel("!a", "!s"), spot("!d", "!cat"), true);
        let (_, changes) = arranged(&rx).unwrap();
        // `#d` and `#c` had no order: the one ahead of the drop gets one.
        let Some(Change::Move {
            from,
            to,
            child,
            order,
        }) = changes.last()
        else {
            panic!("{changes:?}");
        };
        assert_eq!(
            (from.as_str(), to.as_str(), child.as_str()),
            ("!s", "!cat", "!a")
        );
        assert!(valid(order));
        assert!(matches!(&changes[..changes.len() - 1],
            [Change::Order { space, child, .. }] if space == "!cat" && child == "!d"));
        let groups = &app.live.groups["!s"];
        assert_eq!(groups[0].channels.len(), 2);
        let cat: Vec<&str> = groups[1].channels.iter().map(|c| c.room.as_str()).collect();
        assert_eq!(cat, ["!d", "!a", "!c"]);
    }

    #[test]
    fn categories_reorder_among_themselves() {
        let (mut app, rx) = app(100);
        let mut rooms = list(100);
        rooms.rooms.push(RoomSummary {
            space: true,
            ..rooms.rooms[1].clone()
        });
        let last = rooms.rooms.len() - 1;
        rooms.rooms[last].id = "!cat2".into();
        rooms.rooms[last].child_order.clear();
        rooms.rooms[0].child_order.push(edge("!cat2", Some("y"), 3));
        rooms.children.get_mut("!s").unwrap().insert("!cat2".into());
        app.engine_event(Event::Rooms(rooms));
        let held = Held::Category {
            space: "!cat2".into(),
            parent: "!s".into(),
        };
        let over = Spot::Category {
            space: "!cat".into(),
        };
        let _ = app.update(Msg::Order(Input::Lift(held.clone())));
        let _ = app.update(Msg::Order(Input::Over(over.clone(), false)));
        assert_eq!(app.landing().unwrap().line, Line::Above(over.clone()));
        let _ = app.update(Msg::Order(Input::Drop));
        let (_, changes) = arranged(&rx).unwrap();
        assert!(matches!(&changes[..],
            [Change::Order { space, child, order }] if space == "!s" && child == "!cat2" && order.as_str() < "x"));
        let order: Vec<&str> = app.live.groups["!s"]
            .iter()
            .map(|g| g.space.as_str())
            .collect();
        assert_eq!(order, ["!s", "!cat2", "!cat"]);
        // Below the last category: the line goes under its last row.
        let _ = app.update(Msg::Order(Input::Lift(held)));
        let _ = app.update(Msg::Order(Input::Over(over, true)));
        assert_eq!(
            app.landing().unwrap().line,
            Line::AfterCategory("!cat".into())
        );
    }

    #[test]
    fn dragging_needs_the_power_to_send_space_children() {
        let (mut app, rx) = app(0);
        drag(&mut app, channel("!a", "!s"), spot("!b", "!s"), false);
        assert!(app.order.held.is_none());
        assert!(arranged(&rx).is_none());
        assert_eq!(app.live.ordered_spaces["!s"][..2], ["!b", "!a"]);
    }

    #[test]
    fn a_drop_where_it_already_is_writes_nothing() {
        let (mut app, rx) = app(100);
        // Above `#a`, which `#b` is already just above; and on itself.
        drag(&mut app, channel("!b", "!s"), spot("!a", "!s"), false);
        drag(&mut app, channel("!b", "!s"), spot("!b", "!s"), true);
        assert!(arranged(&rx).is_none());
        // A text channel isn't dropped among voice channels.
        let voice = Spot::Channel {
            room: "!v".into(),
            space: "!s".into(),
            edge: Some("!v".into()),
            voice: true,
        };
        drag(&mut app, channel("!b", "!s"), voice, false);
        assert!(arranged(&rx).is_none());
    }

    #[test]
    fn repeated_drops_at_the_same_place_keep_working() {
        let mut siblings = vec![edge("!a", Some("a"), 1), edge("!b", Some("b"), 2)];
        for i in 0..200i64 {
            let id = format!("!n{i}");
            let changes = reorder(&siblings, &id, Some("!b"));
            assert!(changes.iter().all(|(_, o)| valid(o)), "drop {i}");
            siblings.push(edge(&id, None, 10 + i));
            for (child, order) in changes {
                siblings
                    .iter_mut()
                    .find(|e| e.child == child)
                    .unwrap()
                    .order = Some(order);
            }
            sort(&mut siblings);
            let at = siblings.iter().position(|e| e.child == id).unwrap();
            assert_eq!(siblings[at + 1].child, "!b", "drop {i}");
        }
    }
}
