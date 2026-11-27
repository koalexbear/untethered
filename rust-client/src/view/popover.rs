//! A popup floating by what opened it, over everything else, as Discord's
//! reaction picker does: below its anchor, or above it when there's no room
//! below, and kept inside the window. A click or scroll outside closes it
//! and does nothing else; inside, it works like any other content.
use super::{El, Msg};
use iced::{
    Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
};

/// Space kept between the popup and the window's edges.
const MARGIN: f32 = 8.;
/// Between the popup and its anchor.
const GAP: f32 = 4.;

/// Where the popup goes, relative to its anchor's bounds.
#[derive(Clone, Copy)]
pub(super) enum Place {
    /// Below, from its left edge: under a button.
    Below,
    /// Below a point `inset` in from the anchor's top-right corner and `drop`
    /// down, its right edge there: under a message's hover toolbar, which
    /// sits over the row's top-right corner.
    UnderCorner { inset: f32, drop: f32 },
}

/// `anchor`, with `popup` floating by it while there is one; a click
/// outside the popup publishes `dismiss`.
pub(super) fn popover<'a>(
    anchor: impl Into<El<'a>>,
    popup: Option<El<'a>>,
    place: Place,
    dismiss: Msg,
) -> El<'a> {
    El::new(Popover {
        anchor: anchor.into(),
        popup,
        place,
        dismiss,
    })
}

struct Popover<'a> {
    anchor: El<'a>,
    popup: Option<El<'a>>,
    place: Place,
    dismiss: Msg,
}

impl Widget<Msg, Theme, Renderer> for Popover<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
    }
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.anchor)
            .chain(&self.popup)
            .map(Tree::new)
            .collect()
    }
    fn diff(&self, tree: &mut Tree) {
        // The anchor stays the first child, so it keeps its state when the
        // popup comes and goes.
        let children: Vec<&El<'_>> = std::iter::once(&self.anchor).chain(&self.popup).collect();
        tree.diff_children(&children);
    }
    fn size(&self) -> Size<Length> {
        self.anchor.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.anchor.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.anchor
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
        self.anchor.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
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
        self.anchor.as_widget_mut().update(
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
        self.anchor.as_widget().mouse_interaction(
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
        self.anchor
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
        let (anchor_tree, popup_tree) = match tree.children.as_mut_slice() {
            [anchor, popup] => (anchor, Some(popup)),
            [anchor] => (anchor, None),
            _ => return None,
        };
        let base = self.anchor.as_widget_mut().overlay(
            anchor_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let (Some(popup), Some(popup_tree)) = (self.popup.as_mut(), popup_tree) else {
            return base;
        };
        let floating = overlay::Element::new(Box::new(Floating {
            content: popup,
            tree: popup_tree,
            anchor: layout.bounds() + translation,
            place: self.place,
            dismiss: self.dismiss.clone(),
        }));
        Some(match base {
            Some(base) => overlay::Group::with_children(vec![base, floating]).overlay(),
            None => floating,
        })
    }
}

struct Floating<'a, 'b> {
    content: &'a mut El<'b>,
    tree: &'a mut Tree,
    anchor: Rectangle,
    place: Place,
    dismiss: Msg,
}

/// The popup's top-left corner for `size`, by `anchor` and inside `window`.
fn position(anchor: Rectangle, place: Place, size: Size, window: Size) -> Point {
    let (x, below, above) = match place {
        Place::Below => (anchor.x, anchor.y + anchor.height + GAP, anchor.y - GAP),
        Place::UnderCorner { inset, drop } => {
            let corner = anchor.y + drop;
            (
                anchor.x + anchor.width - inset - size.width,
                corner + GAP,
                corner - GAP,
            )
        }
    };
    let y = if below + size.height <= window.height - MARGIN {
        below
    } else {
        above - size.height
    };
    Point::new(
        x.min(window.width - MARGIN - size.width).max(MARGIN),
        y.min(window.height - MARGIN - size.height).max(MARGIN),
    )
}

impl overlay::Overlay<Msg, Theme, Renderer> for Floating<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.content.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let at = position(self.anchor, self.place, node.size(), bounds);
        layout::Node::with_children(bounds, vec![node.move_to(at)])
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
    ) {
        let popup = layout.children().next().expect("the popup is laid out");
        let inside = cursor.is_over(popup.bounds());
        if !inside
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. })
                    | Event::Touch(iced::touch::Event::FingerPressed { .. })
            )
        {
            shell.publish(self.dismiss.clone());
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            self.tree,
            event,
            popup,
            cursor,
            renderer,
            clipboard,
            shell,
            &popup.bounds(),
        );
        if inside && matches!(event, Event::Mouse(_)) {
            shell.capture_event();
        }
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let popup = layout.children().next().expect("the popup is laid out");
        self.content.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            popup,
            cursor,
            &popup.bounds(),
        );
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let popup = layout.children().next().expect("the popup is laid out");
        if !cursor.is_over(popup.bounds()) {
            return mouse::Interaction::None;
        }
        match self.content.as_widget().mouse_interaction(
            self.tree,
            popup,
            cursor,
            &popup.bounds(),
            renderer,
        ) {
            // Nothing beneath the popup reacts to the pointer over it.
            mouse::Interaction::None => mouse::Interaction::Idle,
            other => other,
        }
    }
    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        let popup = layout.children().next().expect("the popup is laid out");
        self.content
            .as_widget_mut()
            .operate(self.tree, popup, renderer, operation);
    }
    fn index(&self) -> f32 {
        // Above the hover toolbar it was opened from.
        2.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Size = Size::new(1000., 800.);

    #[test]
    fn it_opens_below_and_flips_above_near_the_bottom() {
        let button = Rectangle::new(Point::new(100., 200.), Size::new(30., 24.));
        let size = Size::new(300., 250.);
        assert_eq!(
            position(button, Place::Below, size, WINDOW),
            Point::new(100., 228.)
        );
        let low = Rectangle::new(Point::new(100., 700.), Size::new(30., 24.));
        assert_eq!(
            position(low, Place::Below, size, WINDOW),
            Point::new(100., 446.)
        );
    }

    #[test]
    fn under_a_corner_it_lines_up_its_right_edge_and_stays_inside() {
        let row = Rectangle::new(Point::new(300., 100.), Size::new(680., 60.));
        let size = Size::new(300., 250.);
        let place = Place::UnderCorner {
            inset: 16.,
            drop: 18.,
        };
        assert_eq!(position(row, place, size, WINDOW), Point::new(664., 122.));
        let narrow = Rectangle::new(Point::new(0., 100.), Size::new(200., 60.));
        assert_eq!(position(narrow, place, size, WINDOW).x, MARGIN);
    }
}
