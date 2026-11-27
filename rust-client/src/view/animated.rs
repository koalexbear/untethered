//! An animated GIF or WebP (ROADMAP P1): its frames in turn, each for its own
//! delay. It asks for the next frame only while it's on screen, so a picture
//! scrolled away (or behind another view) costs nothing until it's back.
use crate::preview::Frames;
use iced::{
    ContentFit, Element, Event, Length, Radians, Rectangle, Size,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, renderer,
        widget::{Tree, tree},
    },
    border,
    time::Instant,
    widget::image::FilterMethod,
    window,
};
use std::sync::Arc;

pub struct Animated {
    frames: Arc<Frames>,
    width: Length,
    height: Length,
    fit: ContentFit,
    radius: f32,
}

pub fn animated(frames: Arc<Frames>) -> Animated {
    Animated {
        frames,
        width: Length::Shrink,
        height: Length::Shrink,
        fit: ContentFit::Contain,
        radius: 0.,
    }
}

impl Animated {
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
    pub fn content_fit(mut self, fit: ContentFit) -> Self {
        self.fit = fit;
        self
    }
    pub fn border_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
    fn natural(&self) -> Size {
        match &self.frames.frames[0].0 {
            iced::widget::image::Handle::Rgba { width, height, .. } => {
                Size::new(*width as f32, *height as f32)
            }
            _ => Size::new(1., 1.),
        }
    }
}

struct State {
    /// Which `frames` this is, so new frames start over.
    key: usize,
    index: usize,
    next: Option<Instant>,
}

impl State {
    fn new(frames: &Arc<Frames>) -> Self {
        Self {
            key: Arc::as_ptr(frames) as usize,
            index: 0,
            next: None,
        }
    }
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for Animated {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(&self.frames))
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let natural = self.natural();
        let room = limits.resolve(self.width, self.height, natural);
        let shown = self.fit.fit(natural, room);
        layout::Node::new(Size::new(
            match self.width {
                Length::Shrink => shown.width,
                _ => room.width,
            },
            match self.height {
                Length::Shrink => shown.height,
                _ => room.height,
            },
        ))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &iced::Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let state = tree.state.downcast_mut::<State>();
        if state.key != Arc::as_ptr(&self.frames) as usize {
            *state = State::new(&self.frames);
        }
        // Off screen: no next frame, so no work, until it's drawn again.
        if !viewport.intersects(&layout.bounds()) {
            state.next = None;
            return;
        }
        let frames = &self.frames.frames;
        match state.next {
            Some(at) if *now >= at => {
                state.index = (state.index + 1) % frames.len();
                // Late frames don't pile up: the next waits from now.
                state.next = Some(*now + frames[state.index].1);
                shell.request_redraw();
            }
            Some(_) => {}
            None => state.next = Some(*now + frames[state.index].1),
        }
        if let Some(at) = state.next {
            shell.request_redraw_at(window::RedrawRequest::At(at));
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
        viewport: &Rectangle,
    ) {
        use iced::advanced::{Renderer as _, image::Renderer as _};
        let state = tree.state.downcast_ref::<State>();
        let index = if state.key == Arc::as_ptr(&self.frames) as usize {
            state.index.min(self.frames.frames.len() - 1)
        } else {
            0
        };
        let bounds = layout.bounds();
        let shown = self.fit.fit(self.natural(), bounds.size());
        let at = Rectangle::new(
            bounds.center() - iced::Vector::new(shown.width, shown.height) * 0.5,
            shown,
        );
        let image = iced::advanced::image::Image {
            handle: self.frames.frames[index].0.clone(),
            border_radius: border::Radius::from(self.radius),
            filter_method: FilterMethod::Linear,
            rotation: Radians(0.),
            opacity: 1.,
            snap: true,
        };
        renderer.with_layer(
            bounds.intersection(viewport).unwrap_or(bounds),
            |renderer| {
                renderer.draw_image(image, at, bounds);
            },
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<Animated> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(animated: Animated) -> Self {
        Element::new(animated)
    }
}
