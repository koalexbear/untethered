//! Message text you can select and copy, as on Discord: iced's rich text
//! (`iced_widget` 0.14 `text::Rich`, MIT) with a mouse selection drawn under
//! the glyphs, a word on double click and the whole paragraph on triple
//! click, and Ctrl+C (Cmd+C on macOS) to copy it. A selection stays within
//! one paragraph, and clicking anywhere else clears it; links still open on
//! a click.
use iced::advanced::graphics::text::cosmic_text::{Buffer, Cursor};
use iced::advanced::layout;
use iced::advanced::mouse::{self, click};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::{Paragraph as _, Renderer as _, Span};
use iced::advanced::widget::text::{self as text_widget, LineHeight, Shaping, Wrapping};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, clipboard};
use iced::{
    Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Theme, Vector, alignment,
    keyboard,
};

type Renderer = iced::Renderer;
type Paragraph = <Renderer as iced::advanced::text::Renderer>::Paragraph;

/// Discord's selection colour over the dark theme.
const SELECTION: Color = Color::from_rgba(0.34, 0.4, 0.95, 0.45);

pub fn rich<'a, Link: Clone + 'static, Message>(
    spans: impl AsRef<[Span<'a, Link, Font>]> + 'a,
) -> Selectable<'a, Link, Message> {
    Selectable {
        spans: Box::new(spans),
        size: None,
        line_height: LineHeight::default(),
        font: None,
        color: None,
        wrapping: Wrapping::default(),
        hovered_link: None,
        on_link_click: None,
        images: Vec::new(),
    }
}

pub struct Selectable<'a, Link: Clone + 'static, Message> {
    spans: Box<dyn AsRef<[Span<'a, Link, Font>]> + 'a>,
    size: Option<Pixels>,
    line_height: LineHeight,
    font: Option<Font>,
    color: Option<Color>,
    wrapping: Wrapping,
    hovered_link: Option<usize>,
    on_link_click: Option<Box<dyn Fn(Link) -> Message + 'a>>,
    /// Pictures drawn over spans that make room for them: custom emoji.
    images: Vec<(usize, iced::widget::image::Handle)>,
}

impl<'a, Link: Clone + 'static, Message> Selectable<'a, Link, Message> {
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }
    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }
    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }
    pub fn on_link_click(mut self, on_link_click: impl Fn(Link) -> Message + 'a) -> Self {
        self.on_link_click = Some(Box::new(on_link_click));
        self
    }
    /// Draw `handle` over span `index`, whose text only makes room for it.
    pub fn images(mut self, images: Vec<(usize, iced::widget::image::Handle)>) -> Self {
        self.images = images;
        self
    }
    fn spans(&self) -> &[Span<'a, Link, Font>] {
        self.spans.as_ref().as_ref()
    }
}

/// What a drag extends by: characters after a single click, words after a
/// double click.
#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Char,
    Word,
}

struct State<Link> {
    spans: Vec<Span<'static, Link, Font>>,
    span_pressed: Option<usize>,
    paragraph: Paragraph,
    /// Where the selection started and where it ends now; for a word drag,
    /// the start word's two ends.
    selection: Option<(Cursor, Cursor)>,
    origin: Option<(Cursor, Cursor)>,
    dragging: Option<Unit>,
    last_click: Option<mouse::Click>,
}

impl<Link> State<Link> {
    fn buffer(&self) -> &Buffer {
        self.paragraph.buffer()
    }
    fn ordered(&self) -> Option<(Cursor, Cursor)> {
        let (a, b) = self.selection?;
        (a != b).then(|| if a <= b { (a, b) } else { (b, a) })
    }
    fn start(&self) -> Cursor {
        Cursor::new(0, 0)
    }
    fn end(&self) -> Cursor {
        let lines = &self.buffer().lines;
        let last = lines.len().saturating_sub(1);
        Cursor::new(last, lines.get(last).map_or(0, |l| l.text().len()))
    }
    /// The text position under `point` (relative to the paragraph), held
    /// to the paragraph's start or end once the pointer leaves it.
    fn hit(&self, point: Point, bounds: Size) -> Cursor {
        if point.y < 0. {
            return self.start();
        }
        if point.y >= bounds.height {
            return self.end();
        }
        self.buffer()
            .hit(point.x.clamp(0., bounds.width), point.y)
            .unwrap_or_else(|| self.end())
    }
    /// The word around `at`: letters, digits and underscores, or else the
    /// single character there.
    fn word(&self, at: Cursor) -> (Cursor, Cursor) {
        let Some(line) = self.buffer().lines.get(at.line) else {
            return (at, at);
        };
        let text = line.text();
        let index = at.index.min(text.len());
        let wordy = |c: char| c.is_alphanumeric() || c == '_';
        let before = text[..index].char_indices().rev();
        let after = text[index..].char_indices();
        let mut start = index;
        for (i, c) in before {
            if !wordy(c) {
                break;
            }
            start = i;
        }
        let mut end = index;
        for (i, c) in after {
            if !wordy(c) {
                break;
            }
            end = index + i + c.len_utf8();
        }
        if start == end {
            // Not in a word: the character after the cursor.
            end = text[index..]
                .chars()
                .next()
                .map_or(index, |c| index + c.len_utf8());
        }
        (Cursor::new(at.line, start), Cursor::new(at.line, end))
    }
    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.ordered()?;
        let lines = &self.buffer().lines;
        let mut out = String::new();
        for line in a.line..=b.line {
            let text = lines.get(line)?.text();
            let from = if line == a.line { a.index } else { 0 }.min(text.len());
            let to = if line == b.line { b.index } else { text.len() }.min(text.len());
            if line > a.line {
                out.push('\n');
            }
            out.push_str(text.get(from..to.max(from)).unwrap_or_default());
        }
        (!out.is_empty()).then_some(out)
    }
}

impl<Link: Clone + 'static, Message> Widget<Message, Theme, Renderer>
    for Selectable<'_, Link, Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Link>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Link> {
            spans: Vec::new(),
            span_pressed: None,
            paragraph: Paragraph::default(),
            selection: None,
            origin: None,
            dragging: None,
            last_click: None,
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Link>>();
        let spans = self.spans.as_ref().as_ref();
        layout::sized(limits, Length::Shrink, Length::Shrink, |limits| {
            let bounds = limits.max();
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let font = self.font.unwrap_or_else(|| renderer.default_font());
            let (line_height, wrapping) = (self.line_height, self.wrapping);
            let text = |content| text(content, bounds, size, line_height, font, wrapping);
            if state.spans != spans {
                state.paragraph = Paragraph::with_spans(text(spans));
                state.spans = spans.iter().cloned().map(Span::to_static).collect();
                // The text changed: an old selection no longer fits it.
                state.selection = None;
                state.dragging = None;
            } else {
                match state.paragraph.compare(self::text(
                    (),
                    bounds,
                    size,
                    line_height,
                    font,
                    wrapping,
                )) {
                    iced::advanced::text::Difference::None => {}
                    iced::advanced::text::Difference::Bounds => state.paragraph.resize(bounds),
                    iced::advanced::text::Difference::Shape => {
                        state.paragraph = Paragraph::with_spans(text(spans));
                    }
                }
            }
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if !layout.bounds().intersects(viewport) {
            return;
        }
        let state = tree.state.downcast_ref::<State<Link>>();
        let translation = layout.position() - Point::ORIGIN;
        if let Some((start, end)) = state.ordered() {
            for run in state.buffer().layout_runs() {
                if let Some((x, width)) = run.highlight(start, end) {
                    // An empty line inside the selection still shows a sliver.
                    let width = if width < 1. && run.line_i > start.line && run.line_i < end.line {
                        4.
                    } else {
                        width
                    };
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(x, run.line_top),
                                Size::new(width, run.line_height),
                            ) + translation,
                            ..Default::default()
                        },
                        SELECTION,
                    );
                }
            }
        }
        let style = text_widget::Style { color: self.color };
        for (index, span) in self.spans().iter().enumerate() {
            let hovered = self.on_link_click.is_some() && Some(index) == self.hovered_link;
            if span.highlight.is_none() && !span.underline && !span.strikethrough && !hovered {
                continue;
            }
            let regions = state.paragraph.span_bounds(index);
            if let Some(highlight) = span.highlight {
                for bounds in &regions {
                    let bounds = Rectangle::new(
                        bounds.position() - Vector::new(span.padding.left, span.padding.top),
                        bounds.size() + Size::new(span.padding.x(), span.padding.y()),
                    );
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: bounds + translation,
                            border: highlight.border,
                            ..Default::default()
                        },
                        highlight.background,
                    );
                }
            }
            if span.underline || span.strikethrough || hovered {
                let size = span.size.or(self.size).unwrap_or(renderer.default_size());
                let line_height = span
                    .line_height
                    .unwrap_or(self.line_height)
                    .to_absolute(size);
                let color = span.color.or(style.color).unwrap_or(defaults.text_color);
                let baseline =
                    translation + Vector::new(0.0, size.0 + (line_height.0 - size.0) / 2.0);
                let mut line = |drop: f32| {
                    for bounds in &regions {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle::new(
                                    bounds.position() + baseline - Vector::new(0.0, drop),
                                    Size::new(bounds.width, 1.0),
                                ),
                                ..Default::default()
                            },
                            color,
                        );
                    }
                };
                if span.underline || hovered {
                    line(size.0 * 0.08);
                }
                if span.strikethrough {
                    line(size.0 / 2.0);
                }
            }
        }
        text_widget::draw(
            renderer,
            defaults,
            layout.bounds(),
            &state.paragraph,
            style,
            viewport,
        );
        // Each picture fills the square its span makes, centred on the line.
        for (index, handle) in &self.images {
            for bounds in state.paragraph.span_bounds(*index) {
                let side = bounds.width.min(bounds.height);
                if side < 1. {
                    continue;
                }
                let square = Rectangle::new(
                    Point::new(
                        bounds.x + (bounds.width - side) / 2.,
                        bounds.y + (bounds.height - side) / 2.,
                    ),
                    Size::new(side, side),
                ) + translation;
                iced::advanced::image::Renderer::draw_image(
                    renderer,
                    iced::advanced::image::Image::new(handle.clone()),
                    square,
                    *viewport,
                );
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State<Link>>();
        let was_hovered = self.hovered_link.is_some();
        self.hovered_link = cursor.position_in(bounds).and_then(|at| {
            let span = state.paragraph.hit_span(at)?;
            self.spans.as_ref().as_ref().get(span)?.link.as_ref()?;
            self.on_link_click.as_ref().map(|_| span)
        });
        if was_hovered != self.hovered_link.is_some() {
            shell.request_redraw();
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if self.hovered_link.is_some() {
                    state.span_pressed = self.hovered_link;
                    shell.capture_event();
                    return;
                }
                let Some(at) = cursor.position_over(bounds) else {
                    // A click anywhere else ends this paragraph's selection.
                    if state.selection.take().is_some() {
                        shell.request_redraw();
                    }
                    state.last_click = None;
                    return;
                };
                let local = at - (bounds.position() - Point::ORIGIN);
                let clicked = mouse::Click::new(at, mouse::Button::Left, state.last_click);
                state.last_click = Some(clicked);
                let here = state.hit(local, bounds.size());
                match clicked.kind() {
                    click::Kind::Single => {
                        state.selection = Some((here, here));
                        state.origin = None;
                        state.dragging = Some(Unit::Char);
                    }
                    click::Kind::Double => {
                        let word = state.word(here);
                        state.selection = Some(word);
                        state.origin = Some(word);
                        state.dragging = Some(Unit::Word);
                    }
                    click::Kind::Triple => {
                        state.selection = Some((state.start(), state.end()));
                        state.dragging = None;
                    }
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(unit) = state.dragging else {
                    return;
                };
                // The event's position is the window's; a scrollable shifts
                // the cursor it passes on, also while the pointer is outside
                // it (levitating), to the paragraph's own coordinates.
                let Some(position) = cursor.land().position() else {
                    return;
                };
                let local = position - (bounds.position() - Point::ORIGIN);
                let here = state.hit(local, bounds.size());
                state.selection = match (unit, state.origin, state.selection) {
                    (Unit::Word, Some((first, last)), _) => {
                        let (start, end) = state.word(here);
                        Some(if start < first {
                            (last, start)
                        } else {
                            (first, end.max(last))
                        })
                    }
                    (_, _, Some((anchor, _))) => Some((anchor, here)),
                    _ => None,
                };
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = None;
                if let (Some(span), Some(on_click)) =
                    (state.span_pressed.take(), &self.on_link_click)
                    && Some(span) == self.hovered_link
                    && let Some(link) = self
                        .spans
                        .as_ref()
                        .as_ref()
                        .get(span)
                        .and_then(|s| s.link.clone())
                {
                    shell.publish(on_click(link));
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if modifiers.command() && key.as_ref() == keyboard::Key::Character("c") =>
            {
                if let Some(text) = state.selected_text() {
                    clipboard.write(clipboard::Kind::Standard, text);
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State<Link>>();
        if self.hovered_link.is_some() {
            mouse::Interaction::Pointer
        } else if state.dragging.is_some() || cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

fn text<C>(
    content: C,
    bounds: Size,
    size: Pixels,
    line_height: LineHeight,
    font: Font,
    wrapping: Wrapping,
) -> iced::advanced::Text<C, Font> {
    iced::advanced::Text {
        content,
        bounds,
        size,
        line_height,
        font,
        align_x: text_widget::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping,
    }
}

impl<'a, Link: Clone + 'static, Message: 'a> From<Selectable<'a, Link, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(text: Selectable<'a, Link, Message>) -> Self {
        Element::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(content: &str) -> State<()> {
        let spans = [Span::<(), Font>::new(content)];
        State {
            spans: Vec::new(),
            span_pressed: None,
            paragraph: Paragraph::with_spans(text(
                &spans[..],
                Size::new(400., 400.),
                Pixels(16.),
                LineHeight::default(),
                Font::DEFAULT,
                Wrapping::Word,
            )),
            selection: None,
            origin: None,
            dragging: None,
            last_click: None,
        }
    }

    #[test]
    fn a_double_click_takes_the_word_and_a_drag_copies_across_lines() {
        let mut s = state("hello brave_new world, ok?\nsecond line");
        let (a, b) = s.word(Cursor::new(0, 9));
        assert_eq!((a.index, b.index), (6, 15), "brave_new");
        // Just after a word: that word; off one: the character there.
        let (a, b) = s.word(Cursor::new(0, 21));
        assert_eq!((a.index, b.index), (16, 21), "world");
        let (a, b) = s.word(Cursor::new(0, 22));
        assert_eq!((a.index, b.index), (22, 23), "the space");
        assert_eq!(s.selected_text(), None);
        s.selection = Some((Cursor::new(1, 6), Cursor::new(0, 6)));
        assert_eq!(
            s.selected_text().as_deref(),
            Some("brave_new world, ok?\nsecond")
        );
        s.selection = Some((s.start(), s.end()));
        assert_eq!(
            s.selected_text().as_deref(),
            Some("hello brave_new world, ok?\nsecond line")
        );
    }

    #[test]
    fn a_drag_past_the_paragraph_stops_at_its_ends() {
        let s = state("one line");
        let size = Size::new(400., 22.);
        assert_eq!(s.hit(Point::new(50., -30.), size), Cursor::new(0, 0));
        assert_eq!(s.hit(Point::new(5., 90.), size), Cursor::new(0, 8));
    }
}
