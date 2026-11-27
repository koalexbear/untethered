//! A picture in the media viewer that zooms (part of ROADMAP P3), adapted
//! from Iced's image viewer: it fits the stage without being enlarged, the
//! mouse wheel zooms around the pointer, a click zooms in on that point (to
//! twice the size, or to the picture's own pixels if more) and out again, and
//! a drag pans while zoomed. Only the picture itself takes the pointer, so a
//! click beside it still reaches the backdrop and closes the viewer. Another
//! picture (`key`) starts unzoomed.
use iced::{
    Element, Event, Length, Point, Radians, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, renderer,
        widget::{Tree, tree},
    },
    border,
    widget::image::{FilterMethod, Handle},
};

/// The furthest a picture zooms in, from its fitted size.
pub const MAX: f32 = 8.;
/// One wheel notch.
const STEP: f32 = 1.25;
/// A press that moves less than this is a click, not a drag.
const CLICK: f32 = 4.;

pub struct Zoom<Message> {
    handle: Handle,
    natural: Size,
    key: u64,
    /// Sent when the picture is first zoomed into.
    on_zoom: Option<Message>,
}

pub fn zoom<Message>(handle: Handle, natural: Size, key: u64) -> Zoom<Message> {
    Zoom {
        handle,
        natural,
        key,
        on_zoom: None,
    }
}

impl<Message> Zoom<Message> {
    pub fn on_zoom(mut self, message: Message) -> Self {
        self.on_zoom = Some(message);
        self
    }
}

#[derive(Default)]
struct State {
    key: u64,
    scale: f32,
    offset: Vector,
    grabbed: Option<(Point, Vector)>,
    dragged: bool,
}

impl State {
    fn scale(&self) -> f32 {
        self.scale.max(1.)
    }
}

/// `natural` fitted into `room`, never enlarged.
pub fn fitted(natural: Size, room: Size) -> Size {
    let scale = (room.width / natural.width)
        .min(room.height / natural.height)
        .min(1.);
    Size::new(
        (natural.width * scale).max(1.),
        (natural.height * scale).max(1.),
    )
}

/// The offset kept within what lets the picture still cover the middle.
fn clamp(offset: Vector, shown: Size, bounds: Size) -> Vector {
    let room = |picture: f32, view: f32| ((picture - view) / 2.).max(0.);
    let (x, y) = (
        room(shown.width, bounds.width),
        room(shown.height, bounds.height),
    );
    Vector::new(offset.x.clamp(-x, x), offset.y.clamp(-y, y))
}

/// Zoom to `scale`, keeping the point under `at` where it is.
fn rescale(state: &mut State, scale: f32, at: Point, bounds: Rectangle, fit: Size) {
    let previous = state.scale();
    let scale = scale.clamp(1., MAX);
    let factor = scale / previous - 1.;
    let toward = at - bounds.center();
    state.offset = state.offset + (toward + state.offset) * factor;
    state.scale = scale;
    state.offset = clamp(state.offset, fit * scale, bounds.size());
}

impl<Message> Zoom<Message> {
    fn sync(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.key != self.key {
            *state = State {
                key: self.key,
                ..State::default()
            };
        }
    }

    /// Where the picture is drawn.
    fn shown(&self, state: &State, bounds: Rectangle) -> Rectangle {
        let size = fitted(self.natural, bounds.size()) * state.scale();
        let offset = if state.key == self.key {
            state.offset
        } else {
            Vector::ZERO
        };
        Rectangle::new(
            bounds.center() - Vector::new(size.width, size.height) * 0.5 - offset,
            size,
        )
    }
}

impl<Message: Clone, Theme> Widget<Message, Theme, iced::Renderer> for Zoom<Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            key: self.key,
            ..State::default()
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &iced::Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        self.sync(tree);
        let bounds = layout.bounds();
        let fit = fitted(self.natural, bounds.size());
        let state = tree.state.downcast_mut::<State>();
        let was = state.scale();
        let over = cursor
            .position_over(bounds)
            .filter(|p| self.shown(state, bounds).contains(*p));
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(at) = over else { return };
                let y = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    // A touchpad's pixels: about one step per notch's worth.
                    mouse::ScrollDelta::Pixels { y, .. } => y / 100.,
                };
                if y != 0. {
                    let scale = state.scale() * STEP.powf(y.clamp(-3., 3.));
                    rescale(state, scale, at, bounds, fit);
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(at) = over else { return };
                state.grabbed = Some((at, state.offset));
                state.dragged = false;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let Some((from, start)) = state.grabbed else {
                    return;
                };
                let moved = *position - from;
                if !state.dragged && (moved.x.abs() > CLICK || moved.y.abs() > CLICK) {
                    state.dragged = true;
                }
                if state.dragged && state.scale() > 1. {
                    state.offset = clamp(start - moved, fit * state.scale(), bounds.size());
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let Some((at, _)) = state.grabbed.take() else {
                    return;
                };
                if !state.dragged {
                    if state.scale() > 1. {
                        state.scale = 1.;
                        state.offset = Vector::ZERO;
                    } else {
                        // Its own pixels, or twice the size if that's less.
                        let own = (self.natural.width / fit.width).max(2.);
                        rescale(state, own, at, bounds, fit);
                    }
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            _ => {}
        }
        if was <= 1.
            && state.scale() > 1.
            && let Some(message) = &self.on_zoom
        {
            shell.publish(message.clone());
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Rectangle,
        _: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        if state.grabbed.is_some() && state.dragged {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_over(bounds) {
            Some(p) if self.shown(state, bounds).contains(p) => {
                if state.key == self.key && state.scale() > 1. {
                    mouse::Interaction::ZoomOut
                } else {
                    mouse::Interaction::ZoomIn
                }
            }
            _ => mouse::Interaction::None,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        use iced::advanced::{Renderer as _, image::Renderer as _};
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let shown = self.shown(state, bounds);
        renderer.with_layer(bounds, |renderer| {
            renderer.draw_image(
                iced::advanced::image::Image {
                    handle: self.handle.clone(),
                    border_radius: border::Radius::default(),
                    filter_method: FilterMethod::Linear,
                    rotation: Radians(0.),
                    opacity: 1.,
                    snap: true,
                },
                shown,
                bounds,
            );
        });
    }
}

impl<'a, Message: Clone + 'a, Theme: 'a> From<Zoom<Message>>
    for Element<'a, Message, Theme, iced::Renderer>
{
    fn from(zoom: Zoom<Message>) -> Self {
        Element::new(zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_fits_without_growing() {
        assert_eq!(
            fitted(Size::new(4000., 2000.), Size::new(1000., 800.)),
            Size::new(1000., 500.)
        );
        assert_eq!(
            fitted(Size::new(300., 200.), Size::new(1000., 800.)),
            Size::new(300., 200.)
        );
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1000., 800.));
        let fit = Size::new(1000., 500.);
        let mut state = State::default();
        // Zoom in twice at a point right of the middle.
        let at = Point::new(750., 400.);
        rescale(&mut state, 2., at, bounds, fit);
        assert_eq!(state.scale, 2.);
        assert_eq!(state.offset, Vector::new(250., 0.));
        // Not past the picture's edge, nor out past fitting.
        rescale(&mut state, 100., Point::new(1000., 400.), bounds, fit);
        assert_eq!(state.scale, MAX);
        assert!(state.offset.x <= (fit.width * MAX - bounds.width) / 2.);
        rescale(&mut state, 0.1, at, bounds, fit);
        assert_eq!((state.scale, state.offset), (1., Vector::ZERO));
    }
}
