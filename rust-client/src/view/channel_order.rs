//! A space's sidebar with its categories, and dragging channels and
//! categories to reorder them (ROADMAP N5, channel_order.rs).
//!
//! Each row sits in a [`Slot`] that notices a press moving far enough to
//! lift it, and, while something's held, which half of the row the pointer
//! is over. The App's subscription drops it wherever the button comes up.
//! The drop line is drawn inside the row it touches, and rows are neither
//! added nor removed while dragging (an empty heading keeps its slot as an
//! empty space), so Iced keeps every row's state, the pressed button's
//! included, where it was (Iced keeps widget state by position).
use super::{App, BLURPLE, El, Msg, SIDE};
use crate::{
    channel_order::{Group, Held, Input, Line, Spot},
    rooms::Room,
};
use iced::{
    Background, Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    widget::{Column, Space},
};

/// How far a press moves before it lifts the row, so a click still opens.
const LIFT: f32 = 6.;

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
        Element::new(Slot {
            content,
            lift,
            spot,
            dragging: held.is_some(),
            lifted,
            line,
        })
    }
}

pub(super) struct Slot<'a> {
    content: El<'a>,
    lift: Option<Held>,
    spot: Spot,
    /// Something is held.
    dragging: bool,
    /// This row is what's held: drawn dimmed.
    lifted: bool,
    /// Draw the drop line here: on the bottom edge, or the top.
    line: Option<bool>,
}

#[derive(Default)]
struct SlotState {
    /// Where a press on this row started, while the button is down.
    pressed: Option<Point>,
    lifted: bool,
    /// The half of this row under the pointer, last told: lower or not.
    over: Option<bool>,
}

impl Widget<Msg, Theme, Renderer> for Slot<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SlotState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(SlotState::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        let bounds = layout.bounds();
        if !self.lifted && self.line.is_none() {
            return;
        }
        let area = bounds.expand(2.);
        iced::advanced::Renderer::with_layer(renderer, area, |renderer| {
            if self.lifted {
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    renderer::Quad {
                        bounds,
                        ..Default::default()
                    },
                    Background::Color(Color { a: 0.6, ..SIDE }),
                );
            }
            if let Some(below) = self.line {
                let y = if below {
                    bounds.y + bounds.height - 1.
                } else {
                    bounds.y - 1.
                };
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x + 8.,
                            y,
                            width: (bounds.width - 8.).max(0.),
                            height: 2.,
                        },
                        border: iced::Border::default().rounded(1),
                        ..Default::default()
                    },
                    Background::Color(BLURPLE),
                );
            }
        });
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<SlotState>();
        let mut cursor = cursor;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.pressed = cursor.position_over(bounds).filter(|_| self.lift.is_some());
                state.lifted = false;
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(from), Some(at), Some(held)) =
                    (state.pressed, cursor.position(), &self.lift)
                    && !state.lifted
                    && from.distance(at) > LIFT
                {
                    state.lifted = true;
                    shell.publish(Msg::Order(Input::Lift(held.clone())));
                }
                let over = if self.dragging {
                    cursor
                        .position_over(bounds)
                        .map(|at| at.y > bounds.center_y())
                } else {
                    None
                };
                if over != state.over {
                    if let Some(lower) = over {
                        shell.publish(Msg::Order(Input::Over(self.spot.clone(), lower)));
                    } else if self.dragging {
                        shell.publish(Msg::Order(Input::Leave(self.spot.clone())));
                    }
                    state.over = over;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // A drag, not a click: the button forgets its press unclicked.
                // The App drops what's held from its own subscription.
                if std::mem::take(&mut state.lifted) {
                    cursor = mouse::Cursor::Unavailable;
                }
                state.pressed = None;
                state.over = None;
            }
            _ => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.dragging && cursor.is_over(layout.bounds()) {
            return mouse::Interaction::Grabbing;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Msg, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
