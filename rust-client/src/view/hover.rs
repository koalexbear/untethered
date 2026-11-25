//! Hover behavior for timeline rows.
use iced::{
    Background, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
};

/// A message row: highlighted while hovered, with an optional `layer` drawn
/// over it (such as the gutter timestamp) and a `float` toolbar that rises
/// above its top-right edge, as on Discord. The toolbar is an overlay, so a
/// short row never clips it and hovering it keeps the row active.
pub(super) struct Row<'a, Message> {
    base: Element<'a, Message>,
    layer: Element<'a, Message>,
    float: Element<'a, Message>,
    background: Color,
}

#[derive(Default)]
struct RowState {
    over_base: bool,
    over_float: bool,
    /// Where this row last raised its toolbar, in timeline content coordinates.
    raised: Option<Rectangle>,
}
impl RowState {
    fn active(&self) -> bool {
        self.over_base || self.over_float
    }
    fn lower(&mut self) {
        if let Some(area) = self.raised.take() {
            RAISED.with(|r| {
                if r.get() == Some(area) {
                    r.set(None);
                }
            });
        }
    }
}

thread_local! {
    /// The toolbar currently raised over the timeline. It overhangs the row
    /// above, and while one of its tooltips is open the runtime passes the
    /// pointer through to that row; rows under this area ignore the pointer.
    static RAISED: std::cell::Cell<Option<Rectangle>> = const { std::cell::Cell::new(None) };
}

const LIFT: f32 = 16.;
const INSET: f32 = 16.;

pub(super) fn row<'a, Message: 'a>(
    base: impl Into<Element<'a, Message>>,
    layer: impl Into<Element<'a, Message>>,
    float: Option<Element<'a, Message>>,
    background: Color,
) -> Element<'a, Message> {
    Element::new(Row {
        base: base.into(),
        layer: layer.into(),
        float: float.unwrap_or_else(|| iced::widget::Space::new().into()),
        background,
    })
}

impl<Message> Widget<Message, Theme, Renderer> for Row<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RowState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(RowState::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![
            Tree::new(&self.base),
            Tree::new(&self.layer),
            Tree::new(&self.float),
        ]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.base, &self.layer, &self.float]);
    }
    fn size(&self) -> Size<Length> {
        self.base.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.base.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let base = self
            .base
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let size = base.size();
        let layer = self.layer.as_widget_mut().layout(
            &mut tree.children[1],
            renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let float = self.float.as_widget_mut().layout(
            &mut tree.children[2],
            renderer,
            &layout::Limits::new(Size::ZERO, Size::new(size.width, f32::INFINITY)),
        );
        let x = (size.width - float.size().width - INSET).max(0.);
        layout::Node::with_children(
            size,
            vec![base, layer, float.move_to(iced::Point::new(x, -LIFT))],
        )
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
        let bounds = layout.bounds();
        let active = tree.state.downcast_ref::<RowState>().active();
        let mut children = layout.children();
        let (base, layer) = (children.next().unwrap(), children.next().unwrap());
        if active {
            iced::advanced::Renderer::fill_quad(
                renderer,
                renderer::Quad {
                    bounds,
                    ..Default::default()
                },
                Background::Color(self.background),
            );
        }
        self.base.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            base,
            cursor,
            viewport,
        );
        if active {
            iced::advanced::Renderer::with_layer(renderer, bounds, |renderer| {
                self.layer.as_widget().draw(
                    &tree.children[1],
                    renderer,
                    theme,
                    style,
                    layer,
                    cursor,
                    viewport,
                );
            });
        }
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let float = layout.children().nth(2).unwrap().bounds();
        let state = tree.state.downcast_mut::<RowState>();
        let was = state.active();
        let under_toolbar = RAISED
            .with(std::cell::Cell::get)
            .is_some_and(|area| cursor.is_over(area));
        state.over_base = cursor.is_over(layout.bounds()) && !under_toolbar;
        // While the toolbar itself is under the pointer, the overlay tracks it.
        if cursor.position().is_some() && !cursor.is_over(state.raised.unwrap_or(float)) {
            state.over_float = false;
        }
        if !state.active() {
            state.lower();
        }
        if was != state.active() {
            shell.request_redraw();
        }
        self.base.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
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
        self.base.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
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
        self.base.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let mut children = layout.children();
        let base_layout = children.next().unwrap();
        let float_layout = children.nth(1).unwrap();
        let Tree {
            state,
            children: trees,
            ..
        } = tree;
        let state = state.downcast_mut::<RowState>();
        let [base_tree, _, float_tree] = trees.as_mut_slice() else {
            return None;
        };
        let base = self.base.as_widget_mut().overlay(
            base_tree,
            base_layout,
            renderer,
            viewport,
            translation,
        );
        if !state.active() {
            state.over_float = false;
            state.lower();
            return base;
        }
        // Keep the toolbar inside the visible timeline near its top edge.
        let top = float_layout.bounds().y + translation.y;
        let drop = Vector::new(0., (viewport.y - top).max(0.));
        let area = float_layout.bounds() + drop;
        RAISED.with(|r| r.set(Some(area)));
        state.raised = Some(area);
        let offset = translation + drop;
        let float = overlay::Element::new(Box::new(Toolbar {
            content: &mut self.float,
            tree: float_tree,
            state,
            layout: float_layout,
            transformation: iced::Transformation::translate(offset.x, offset.y),
            viewport: *viewport,
        }));
        Some(match base {
            Some(base) => overlay::Group::with_children(vec![base, float]).overlay(),
            None => float,
        })
    }
}

struct Toolbar<'a, 'b, Message> {
    content: &'a mut Element<'b, Message>,
    tree: &'a mut Tree,
    state: &'a mut RowState,
    layout: Layout<'a>,
    transformation: iced::Transformation,
    viewport: Rectangle,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Toolbar<'_, '_, Message> {
    fn layout(&mut self, _: &Renderer, _: Size) -> layout::Node {
        let bounds = self.layout.bounds() * self.transformation;
        layout::Node::new(bounds.size()).move_to(bounds.position())
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let over = cursor.is_over(layout.bounds());
        if over != self.state.over_float {
            self.state.over_float = over;
            shell.request_redraw();
        }
        let inverse = self.transformation.inverse();
        self.content.as_widget_mut().update(
            self.tree,
            event,
            self.layout,
            cursor * inverse,
            renderer,
            clipboard,
            shell,
            &(self.viewport * inverse),
        );
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        _: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let inverse = self.transformation.inverse();
        iced::advanced::Renderer::with_layer(renderer, self.viewport, |renderer| {
            iced::advanced::Renderer::with_transformation(
                renderer,
                self.transformation,
                |renderer| {
                    self.content.as_widget().draw(
                        self.tree,
                        renderer,
                        theme,
                        style,
                        self.layout,
                        cursor * inverse,
                        &(self.viewport * inverse),
                    );
                },
            );
        });
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        let inverse = self.transformation.inverse();
        match self.content.as_widget().mouse_interaction(
            self.tree,
            self.layout,
            cursor * inverse,
            &(self.viewport * inverse),
            renderer,
        ) {
            // Claim the whole toolbar so the row beneath it does not react.
            mouse::Interaction::None => mouse::Interaction::Idle,
            other => other,
        }
    }
    fn overlay<'c>(
        &'c mut self,
        _: Layout<'c>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            self.tree,
            self.layout,
            renderer,
            &(self.viewport * self.transformation.inverse()),
            self.transformation.translation(),
        )
    }
}
