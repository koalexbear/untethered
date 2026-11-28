//! A space's sidebar with its categories, and dragging channels and
//! categories to reorder them (ROADMAP N5, channel_order.rs).
//!
//! Each row sits in a drag [`slot`](super::drag::slot) that notices a press
//! moving far enough to lift it, and, while something's held, which half of
//! the row the pointer is over. The App's subscription drops it wherever
//! the button comes up. Rows are neither added nor removed while dragging
//! (an empty heading keeps its slot as an empty space), so Iced keeps every
//! row's state, the pressed button's included, where it was (Iced keeps
//! widget state by position).
use super::{
    App, El, Msg, SIDE,
    drag::{self, Look, Report},
};
use crate::{
    channel_order::{Group, Held, Input, Line, Spot},
    rooms::Room,
};
use iced::{
    Color,
    widget::{Column, Space},
};

impl App {
    /// A space's channels: its own under TEXT CHANNELS and VOICE CHANNELS,
    /// then a category for each joined space it lists, each with its text
    /// channels, then its voice channels.
    pub(super) fn space_channel_list<'a>(
        &'a self,
        mut list: Column<'a, Msg>,
        groups: &'a [Group],
        create: Option<Msg>,
    ) -> Column<'a, Msg> {
        let rooms = self.rooms();
        let held = self.held();
        let landing = self.landing();
        let line = |spot: &Spot| match landing.as_ref().map(|l| &l.line) {
            Some(Line::Above(s)) if s == spot => Some(false),
            Some(Line::Below(s)) if s == spot => Some(true),
            _ => None,
        };
        let section = groups.first().map_or("", |g| g.space.as_str());
        let (unjoined_text, unjoined_voice) = self.unjoined_channels();
        for group in groups {
            let mine = self.may_arrange(&group.space);
            let collapsed = |kind: &str| self.collapsed.contains(&format!("{section}/{kind}"));
            // What the group shows, text channels first: a collapsed one
            // still shows unread and selected channels.
            let shown = |voice: bool, open: bool| {
                group.channels.iter().filter_map(move |c| {
                    let r = rooms.list.get(&c.room)?;
                    let selected = self.mentions.is_none() && c.room == self.selected;
                    (r.voice == voice && (open || r.unread || selected)).then_some((c, r))
                })
            };
            if !group.category {
                // The server's own channels, under TEXT CHANNELS and
                // VOICE CHANNELS, then those it lists that aren't joined.
                for (voice, kind, title, unjoined) in [
                    (false, "text", "TEXT CHANNELS", &unjoined_text),
                    (true, "voice", "VOICE CHANNELS", &unjoined_voice),
                ] {
                    let open = !collapsed(kind);
                    let spot = Spot::Heading {
                        space: group.space.clone(),
                        voice,
                    };
                    let wanted =
                        mine && matches!(held, Some(Held::Channel { voice: v, .. }) if *v == voice);
                    let any = group
                        .channels
                        .iter()
                        .any(|c| rooms.list.get(&c.room).is_some_and(|r| r.voice == voice));
                    if !any && unjoined.is_empty() && !wanted {
                        // Shown only as a place to drop: keep its slot.
                        list = list.push(Space::new().height(0));
                        continue;
                    }
                    let plus = create.clone().map(|msg| (msg, "Create Channel"));
                    let heading = self.category(&format!("{section}/{kind}"), title, plus);
                    list = list.push(self.slot(heading, None, spot.clone(), line(&spot)));
                    for (c, r) in shown(voice, open) {
                        list = self.channel_slot(list, group, c, r, mine, line, false);
                    }
                    if open {
                        list = list.extend(unjoined.iter().map(|r| self.unjoined_row(r)));
                    }
                }
                continue;
            }
            // A category: its heading, then its channels.
            let open = !collapsed(&group.space);
            let rows: Vec<_> = shown(false, open).chain(shown(true, open)).collect();
            // A category dropped after this one: the line under its last row.
            let after = matches!(landing.as_ref().map(|l| &l.line),
                Some(Line::AfterCategory(s)) if *s == group.space);
            let spot = Spot::Category {
                space: group.space.clone(),
            };
            let name = rooms
                .list
                .get(&group.space)
                .map_or_else(String::new, |r| r.name.to_uppercase());
            let plus = self
                .create_in(&group.space)
                .map(|msg| (msg, "Create Channel"));
            let lift = self.may_arrange(section).then(|| Held::Category {
                space: group.space.clone(),
                parent: section.to_owned(),
            });
            let heading = self.category(&format!("{section}/{}", group.space), name, plus);
            let under = line(&spot).or((after && rows.is_empty()).then_some(true));
            list = list.push(self.slot(heading, lift, spot, under));
            let count = rows.len();
            for (i, (c, r)) in rows.into_iter().enumerate() {
                list = self.channel_slot(list, group, c, r, mine, line, after && i + 1 == count);
            }
        }
        list
    }

    /// A channel's row in `group`, which this account may arrange or not
    /// (`mine`), and the people in it for a voice channel.
    #[allow(clippy::too_many_arguments)]
    fn channel_slot<'a>(
        &'a self,
        list: Column<'a, Msg>,
        group: &'a Group,
        c: &'a crate::channel_order::Channel,
        r: &'a Room,
        mine: bool,
        line: impl Fn(&Spot) -> Option<bool>,
        last: bool,
    ) -> Column<'a, Msg> {
        let spot = Spot::Channel {
            room: c.room.clone(),
            space: group.space.clone(),
            edge: c.edge.clone(),
            voice: r.voice,
        };
        // Only a room the group's own space lists moves from here.
        let lift = c.edge.as_ref().filter(|_| mine).map(|edge| Held::Channel {
            room: c.room.clone(),
            space: group.space.clone(),
            edge: edge.clone(),
            voice: r.voice,
        });
        let under = line(&spot).or(last.then_some(true));
        let list = list.push(self.slot(self.channel_row(&c.room, r), lift, spot, under));
        #[cfg(target_os = "linux")]
        if r.voice {
            return list.push(self.voice_people(&c.room, r));
        }
        list
    }

    /// Create Channel in `space`, as its heading's **+** offers.
    fn create_in(&self, space: &str) -> Option<Msg> {
        (self.logged_in && self.verified && self.connected && self.setup.is_none())
            .then(|| Msg::Setup(crate::community::Input::Start(Some(space.to_owned()))))
    }

    /// A row that can be lifted (`lift`), dropped on (`spot`), and shows
    /// the drop line on its top or bottom edge (`line`: below or not).
    fn slot<'a>(
        &self,
        content: El<'a>,
        lift: Option<Held>,
        spot: Spot,
        line: Option<bool>,
    ) -> El<'a> {
        let held = self.held();
        let lifted = match (held, &spot) {
            (Some(Held::Channel { room, .. }), Spot::Channel { room: r, .. }) => room == r,
            (Some(Held::Category { space, .. }), Spot::Category { space: s }) => space == s,
            _ => false,
        };
        let report = held.is_some().then(|| {
            let report: Report<'a> = Box::new(move |zone| {
                Msg::Order(match zone {
                    Some(half) => Input::Over(spot.clone(), half == 1),
                    None => Input::Leave(spot.clone()),
                })
            });
            (2, report)
        });
        drag::slot(
            content,
            lift.map(|held| Msg::Order(Input::Lift(held))),
            report,
            Look {
                dim: lifted.then_some(Color { a: 0.6, ..SIDE }),
                line,
                inset: 8.,
                gap: 0.,
            },
        )
    }
}
