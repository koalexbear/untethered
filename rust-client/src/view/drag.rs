//! A row that can be dragged and dropped on: the channel sidebar's (N5,
//! channel_order.rs) and the server rail's (N8, rail_order.rs).
//!
//! A [`Slot`] notices a press moving far enough to lift its row, and, while
//! something's held, which part of the row the pointer is over. It never
//! captures events: its content works as before, except that a lifted
//! button gets its release with no cursor, so a drop doesn't click it. The
//! App drops what's held from a subscription, wherever the button comes up.
//! The drop line is drawn by the row it touches, so rows needn't be added
//! or removed while dragging (Iced keeps widget state by position).
use super::{El, Msg};
use iced::{
    Background, Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
};

/// How far a press moves before it lifts the row, so a click still clicks.
const LIFT: f32 = 6.;

/// What a slot says while something's held: the zone of the row the
/// pointer entered (top to bottom, of `zones`), or None when it left.
pub(super) type Report<'a> = Box<dyn Fn(Option<u8>) -> Msg + 'a>;

/// How a slot draws what's dragged.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Look {
    /// Laid over the row while it's what's held.
    pub dim: Option<Color>,
    /// The drop line: on the bottom edge, or the top.
    pub line: Option<bool>,
    /// The line starts this far in from the row's left edge…
    pub inset: f32,
    /// …and its middle is this far outside the edge (in a gap between rows).
    pub gap: f32,
}

/// `content`, which publishes `lift` once a press on it moves far enough,
/// and, while `report` is given (something's held), which of its `zones`
/// the pointer is over.
pub(super) fn slot<'a>(
    content: El<'a>,
    lift: Option<Msg>,
    report: Option<(u8, Report<'a>)>,
    look: Look,
) -> El<'a> {
    Element::new(Slot {
        content,
        lift,
        report,
        look,
    })
}

struct Slot<'a> {
    content: El<'a>,
    lift: Option<Msg>,
    report: Option<(u8, Report<'a>)>,
    look: Look,
}

#[derive(Default)]
struct SlotState {
    /// Where a press on this row started, while the button is down.
    pressed: Option<Point>,
    lifted: bool,
    /// The zone under the pointer, last told.
    over: Option<u8>,
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
        let Look {
            dim,
            line,
            inset,
            gap,
        } = self.look;
        if dim.is_none() && line.is_none() {
            return;
        }
        let bounds = layout.bounds();
        let area = bounds.expand(2. + gap);
        iced::advanced::Renderer::with_layer(renderer, area, |renderer| {
            if let Some(dim) = dim {
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    renderer::Quad {
                        bounds,
                        ..Default::default()
                    },
                    Background::Color(dim),
                );
            }
            if let Some(below) = line {
                let y = if below {
                    bounds.y + bounds.height + gap - 1.
                } else {
                    bounds.y - gap - 1.
                };
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x + inset,
                            y,
                            width: (bounds.width - inset).max(0.),
                            height: 2.,
                        },
                        border: iced::Border::default().rounded(1),
                        ..Default::default()
                    },
                    Background::Color(super::BLURPLE),
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
                if let (Some(from), Some(at), Some(lift)) =
                    (state.pressed, cursor.position(), &self.lift)
                    && !state.lifted
                    && from.distance(at) > LIFT
                {
                    state.lifted = true;
                    shell.publish(lift.clone());
                }
                if let Some((zones, report)) = &self.report {
                    let over = cursor.position_over(bounds).map(|at| {
                        let part = (at.y - bounds.y) / bounds.height.max(1.);
                        ((part * f32::from(*zones)) as u8).min(zones - 1)
                    });
                    if over != state.over {
                        shell.publish(report(over));
                        state.over = over;
                    }
                } else {
                    state.over = None;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // A drag, not a click: the button forgets its press unclicked.
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
        if self.report.is_some() && cursor.is_over(layout.bounds()) {
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
