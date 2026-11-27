//! Right-click menus on screen: an area that opens one on a right click (or
//! Shift+F10 / the Menu key while hovered), and the open menu, drawn as an
//! overlay where it opened and kept inside the window. While open, the menu
//! covers the window, so a click outside only closes it, as on Discord, and
//! nothing beneath reacts to the pointer. Which entries a menu has is
//! `crate::context_menu`'s business.
use super::{
    App, BODY, BOLD, CHAT, El, FLOAT, Icon, Msg, RED, ROW_HOVER, SIDE, TEXT, icon, icons,
    line_rule, plain, surface,
};
use crate::context_menu::{Entry, Event as Menu, Item, Open, Target};
use iced::{
    Alignment::Center,
    Border, Color, Element, Event, Fill, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    color, keyboard,
    widget::{Column, Space, button, column, container, mouse_area, opaque, row, stack, text},
};

/// Space kept between a menu and the window's edges.
const MARGIN: f32 = 8.;
/// A keyboard-opened menu with no pointer position sits this far in from
/// its area's right edge, where the hover toolbar would be.
const INSET: f32 = 16.;

impl App {
    /// `content`, with a right-click menu about `target`. `place` tells it
    /// apart from other places showing the same target (see `Open::place`).
    /// `highlight` fills the area while its menu is open, so the message
    /// it's about stays marked.
    pub(super) fn menu_area<'a>(
        &'a self,
        content: impl Into<El<'a>>,
        target: Target,
        place: String,
        highlight: Option<Color>,
    ) -> El<'a> {
        let popup = self
            .context_menu
            .open
            .as_ref()
            .filter(|open| open.target == target && open.place == place)
            .and_then(|open| {
                let items = self.menu_items(&target);
                (!items.is_empty()).then(|| Popup::new(open.clone(), items))
            });
        Element::new(Area {
            content: content.into(),
            target,
            place,
            popup,
            highlight,
        })
    }

    /// A message row in the open room. Unsent messages have no menu yet.
    pub(super) fn message_area<'a>(
        &'a self,
        p: &crate::rooms::Post,
        content: impl Into<El<'a>>,
    ) -> El<'a> {
        match &p.event_id {
            Some(event) => self.menu_area(
                content,
                Target::Message {
                    room: self.selected.clone(),
                    event: event.clone(),
                },
                String::new(),
                Some(ROW_HOVER),
            ),
            None => content.into(),
        }
    }

    /// A person as the open room shows them, at `place`: their picture or
    /// name on a given message, or their member list row.
    pub(super) fn user_area<'a>(
        &'a self,
        user: &str,
        place: String,
        content: impl Into<El<'a>>,
    ) -> El<'a> {
        self.menu_area(
            content,
            Target::User {
                room: self.selected.clone(),
                user: user.to_owned(),
            },
            place,
            None,
        )
    }

    /// Asks before leaving a room, as Discord does before leaving a server.
    pub(super) fn delete_dialog(&self) -> Option<El<'_>> {
        let (room, event) = self.context_menu.deleting.as_ref()?;
        Some(self.confirm_destructive(
            "Delete Message".into(),
            "Delete this message for everyone in the room?".into(),
            "This cannot be undone.",
            "Delete",
            self.can_delete(room, event).then_some(Menu::ConfirmDelete),
        ))
    }

    pub(super) fn leave_dialog(&self) -> Option<El<'_>> {
        let room = self.context_menu.leaving.as_ref()?;
        let r = self.live.list.get(room)?;
        let (title, question) = if r.dm {
            (
                "Leave Conversation".to_owned(),
                format!("Leave your conversation with {}?", r.name),
            )
        } else {
            (format!("Leave #{}", r.name), format!("Leave #{}?", r.name))
        };
        Some(self.confirm_destructive(
            title,
            question,
            "Rejoining may need a new invite.",
            "Leave",
            (self.connected && !self.leaving_room(room)).then_some(Menu::ConfirmLeave),
        ))
    }

    /// Asks before blocking someone, as Discord does.
    pub(super) fn block_dialog(&self) -> Option<El<'_>> {
        let user = self.context_menu.blocking.as_ref()?;
        let name = self.someone(user);
        Some(self.confirm_destructive(
            format!("Block {name}"),
            format!("Block {name}? Their messages, invites and calls will stay away."),
            "Other Matrix apps you use hide them too. They aren't told.",
            "Block",
            (self.connected && !self.blocking(user)).then_some(Menu::ConfirmBlock),
        ))
    }

    fn confirm_destructive(
        &self,
        title: String,
        question: String,
        hint: &'static str,
        action: &'static str,
        confirm: Option<Menu>,
    ) -> El<'_> {
        let body = column![
            text(title).size(20).font(BOLD).color(TEXT),
            text(question).size(15).color(BODY),
            text(hint).size(14).color(super::CHANNEL),
        ]
        .spacing(8);
        let cancel = button(text("Cancel").size(14))
            .padding([10, 16])
            .style(plain(BODY, Color::WHITE))
            .on_press(Msg::ContextMenu(Menu::Close));
        let leave = button(text(action).size(14).font(BOLD))
            .padding([10, 18])
            .on_press_maybe(confirm.map(Msg::ContextMenu))
            .style(|_, s| button::Style {
                background: Some(
                    match s {
                        button::Status::Hovered | button::Status::Pressed => color!(0xa12828),
                        button::Status::Disabled => color!(0x7a2a2c),
                        button::Status::Active => color!(0xda373c),
                    }
                    .into(),
                ),
                text_color: Color::WHITE,
                border: Border::default().rounded(4),
                ..Default::default()
            });
        let footer = container(
            row![Space::new().width(Fill), cancel, leave]
                .spacing(8)
                .align_y(Center),
        )
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(SIDE.into()),
            border: Border::default().rounded(iced::border::bottom(8)),
            ..Default::default()
        });
        let dialog = container(column![container(body).padding([20, 16]), footer])
            .width(440)
            .style(|_| surface(CHAT, 8.));
        let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
            container::Style {
                background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                ..Default::default()
            }
        }))
        .on_press(Msg::ContextMenu(Menu::Close));
        stack![opaque(backdrop), container(dialog).center(Fill)].into()
    }
}

/// The hover toolbar's icon for an entry it shows.
pub(super) fn tool_glyph(entry: Entry) -> Option<Icon> {
    match entry {
        Entry::Report => Some(Icon::Flag),
        _ => glyph(entry),
    }
}

fn glyph(entry: Entry) -> Option<Icon> {
    match entry {
        Entry::AddReaction => Some(Icon::Smile),
        Entry::Reply => Some(Icon::Reply),
        Entry::Edit => Some(Icon::Pencil),
        Entry::CopyText | Entry::CopyImage => Some(Icon::Copy),
        Entry::Mention => Some(Icon::At),
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        Entry::Notifications => Some(Icon::Bell),
        _ => None,
    }
}

/// One entry, styled like the server menu's.
fn entry<'a>(e: Entry, msg: Option<Msg>) -> El<'a> {
    let enabled = msg.is_some();
    let accent = if e.danger() { RED } else { BODY };
    let mut label = row![text(e.label()).size(14).width(Fill)].align_y(Center);
    // A red glyph would vanish on the red hover, so danger entries have none.
    if let Some(g) = glyph(e).filter(|_| !e.danger()) {
        label = label.push(icon(g, 18., if enabled { BODY } else { icons::ICON_OFF }));
    }
    button(label)
        .width(Fill)
        .padding([7, 8])
        .on_press_maybe(msg)
        .style(move |_, s| button::Style {
            background: (s == button::Status::Hovered || s == button::Status::Pressed).then(|| {
                if e.danger() {
                    color!(0xda373c)
                } else {
                    color!(0x4752c4)
                }
                .into()
            }),
            text_color: match s {
                button::Status::Hovered | button::Status::Pressed => Color::WHITE,
                button::Status::Disabled => icons::ICON_OFF,
                button::Status::Active => accent,
            },
            border: Border::default().rounded(3),
            ..Default::default()
        })
        .into()
}

struct Popup<'a> {
    open: Open,
    content: El<'a>,
    /// One per row of the menu: what Enter on it sends. None for the rules
    /// between groups and for disabled entries, which keyboard focus skips.
    rows: Vec<Option<Msg>>,
}
impl Popup<'_> {
    fn new(open: Open, items: Vec<Item>) -> Self {
        let mut list = Column::new().spacing(2);
        let mut rows = Vec::new();
        let mut group = None;
        for Item { entry: e, msg } in items {
            if group.is_some_and(|g| g != e.group()) {
                list = list.push(container(line_rule(color!(0x2e2f34))).padding([4, 8]));
                rows.push(None);
            }
            group = Some(e.group());
            let msg = msg.map(|m| Msg::ContextMenu(Menu::Pick(Box::new(m))));
            list = list.push(entry(e, msg.clone()));
            rows.push(msg);
        }
        let content = container(list)
            .width(220)
            .padding(6)
            // No shadow: tiny-skia leaves its edge behind when the menu closes.
            .style(|_| container::Style {
                border: Border {
                    color: color!(0x2b2d31),
                    width: 1.,
                    radius: 6.0.into(),
                },
                ..surface(FLOAT, 6.)
            })
            .into();
        Self {
            open,
            content,
            rows,
        }
    }
}

/// Content with a right-click menu.
struct Area<'a> {
    content: El<'a>,
    target: Target,
    place: String,
    popup: Option<Popup<'a>>,
    highlight: Option<Color>,
}

#[derive(Default)]
struct AreaState {
    /// The menu this area last showed, to start a new one's focus afresh.
    serial: u64,
    /// The highlighted row, by pointer or arrow keys.
    focus: Option<usize>,
}

impl Area<'_> {
    fn open(&self, at: Vector, keyboard: bool) -> Msg {
        Msg::ContextMenu(Menu::Open {
            target: self.target.clone(),
            place: self.place.clone(),
            at: Some(at),
            keyboard,
        })
    }
}

/// Shift+F10 or the Menu key, the keyboard's right click.
fn menu_key(event: &Event) -> bool {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::F10),
            modifiers,
            ..
        }) => modifiers.shift(),
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::ContextMenu),
            ..
        }) => true,
        _ => false,
    }
}

impl Widget<Msg, Theme, Renderer> for Area<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<AreaState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(AreaState::default())
    }
    fn children(&self) -> Vec<Tree> {
        let mut children = vec![Tree::new(&self.content)];
        children.extend(self.popup.as_ref().map(|p| Tree::new(&p.content)));
        children
    }
    fn diff(&self, tree: &mut Tree) {
        match &self.popup {
            Some(p) => tree.diff_children(&[&self.content, &p.content]),
            None => tree.diff_children(&[&self.content]),
        }
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
        if let Some(color) = self.highlight.filter(|_| self.popup.is_some()) {
            iced::advanced::Renderer::fill_quad(
                renderer,
                renderer::Quad {
                    bounds: layout.bounds(),
                    ..Default::default()
                },
                color,
            );
        }
        self.content.as_widget().draw(
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
        // Nested areas (an author inside their message) answer first.
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
        if shell.is_event_captured() {
            return;
        }
        let bounds = layout.bounds();
        let Some(at) = cursor.position_over(bounds) else {
            return;
        };
        let keyboard = menu_key(event);
        if keyboard
            || matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
            )
        {
            shell.publish(self.open(at - bounds.position(), keyboard));
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
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
        let Tree {
            state, children, ..
        } = tree;
        let (content_tree, popup_tree) = match children.as_mut_slice() {
            [content, popup] => (content, Some(popup)),
            [content] => (content, None),
            _ => return None,
        };
        let base = self.content.as_widget_mut().overlay(
            content_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let (Some(popup), Some(popup_tree)) = (self.popup.as_mut(), popup_tree) else {
            return base;
        };
        let state = state.downcast_mut::<AreaState>();
        if state.serial != popup.open.serial {
            state.serial = popup.open.serial;
            state.focus = if popup.open.keyboard {
                popup.rows.iter().position(Option::is_some)
            } else {
                None
            };
        }
        let bounds = layout.bounds() + translation;
        let (anchor, from_right) = match popup.open.at {
            Some(at) => (bounds.position() + at, false),
            None => (Point::new(bounds.x + bounds.width - INSET, bounds.y), true),
        };
        let menu = overlay::Element::new(Box::new(Floating {
            content: &mut popup.content,
            tree: popup_tree,
            rows: &popup.rows,
            focus: &mut state.focus,
            anchor,
            from_right,
        }));
        Some(match base {
            Some(base) => overlay::Group::with_children(vec![base, menu]).overlay(),
            None => menu,
        })
    }
}

/// The open menu, over the whole window.
struct Floating<'a, 'b> {
    content: &'a mut El<'b>,
    tree: &'a mut Tree,
    rows: &'a [Option<Msg>],
    focus: &'a mut Option<usize>,
    anchor: Point,
    /// Its right edge goes at the anchor (opened without a pointer position).
    from_right: bool,
}

impl Floating<'_, '_> {
    /// The rows' layouts: the menu is a container around a column of rows.
    fn rows<'l>(menu: Layout<'l>) -> impl Iterator<Item = Layout<'l>> {
        menu.children()
            .next()
            .into_iter()
            .flat_map(|list| list.children())
    }
    /// The row under the pointer: `Some(None)` over a rule or a disabled
    /// entry, None between rows, on the padding or outside the menu.
    fn row_at(&self, menu: Layout<'_>, cursor: mouse::Cursor) -> Option<Option<usize>> {
        let i = Self::rows(menu).position(|row| cursor.is_over(row.bounds()))?;
        Some(self.rows.get(i).is_some_and(Option::is_some).then_some(i))
    }
    /// Keyboard focus looks like hover: the rows see the pointer over the
    /// focused one. The pointer moving onto a row focuses that row.
    fn effective(&self, menu: Layout<'_>, cursor: mouse::Cursor) -> mouse::Cursor {
        self.focus
            .and_then(|i| Self::rows(menu).nth(i))
            .map_or(cursor, |row| {
                mouse::Cursor::Available(row.bounds().center())
            })
    }
    fn step(&mut self, forward: bool) {
        let enabled: Vec<usize> = (0..self.rows.len())
            .filter(|i| self.rows[*i].is_some())
            .collect();
        let (Some(first), Some(last)) = (enabled.first(), enabled.last()) else {
            return;
        };
        *self.focus = Some(match *self.focus {
            None if forward => *first,
            None => *last,
            Some(now) if forward => enabled.iter().copied().find(|i| *i > now).unwrap_or(*first),
            Some(now) => enabled
                .iter()
                .rev()
                .copied()
                .find(|i| *i < now)
                .unwrap_or(*last),
        });
    }
}

impl overlay::Overlay<Msg, Theme, Renderer> for Floating<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.content.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let size = node.size();
        // Like Discord: open down and right of the pointer, to its left when
        // the right edge is near, and shifted up to fit above the bottom.
        let mut x = if self.from_right {
            self.anchor.x - size.width
        } else {
            self.anchor.x
        };
        if x + size.width > bounds.width - MARGIN {
            x = self.anchor.x - size.width;
        }
        let x = x.min(bounds.width - MARGIN - size.width).max(MARGIN);
        let y = self
            .anchor
            .y
            .min(bounds.height - MARGIN - size.height)
            .max(MARGIN);
        layout::Node::with_children(bounds, vec![node.move_to(Point::new(x, y))])
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
        let menu = layout.children().next().expect("the menu is laid out");
        let inside = cursor.is_over(menu.bounds());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. })
            | Event::Touch(iced::touch::Event::FingerPressed { .. })
                if !inside =>
            {
                shell.publish(Msg::ContextMenu(Menu::Close));
                shell.capture_event();
                return;
            }
            // Only moving onto a row moves the highlight. A keyboard-opened
            // menu leaves the pointer on its corner, and a nudge there or
            // outside the menu mustn't take the keyboard's place away.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(hovered) = self.row_at(menu, cursor)
                    && *self.focus != hovered
                {
                    *self.focus = hovered;
                    shell.request_redraw();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key),
                modifiers,
                ..
            }) => {
                use keyboard::key::Named;
                let handled = match key {
                    Named::ArrowDown => {
                        self.step(true);
                        true
                    }
                    Named::ArrowUp => {
                        self.step(false);
                        true
                    }
                    Named::Tab => {
                        self.step(!modifiers.shift());
                        true
                    }
                    Named::Home | Named::End => {
                        *self.focus = None;
                        self.step(*key == Named::Home);
                        true
                    }
                    Named::Enter | Named::Space => {
                        if let Some(msg) = self.focus.and_then(|i| self.rows[i].clone()) {
                            shell.publish(msg);
                        }
                        true
                    }
                    // The same keys again close it, like a second right click
                    // outside would.
                    _ if menu_key(event) => {
                        shell.publish(Msg::ContextMenu(Menu::Close));
                        true
                    }
                    // Escape goes to the App, which closes the topmost layer.
                    _ => false,
                };
                if handled {
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }
            }
            _ => {}
        }
        // Presses land where the pointer is; everything else (hover styles)
        // follows the focused row.
        let rows_cursor = if matches!(event, Event::Mouse(_) | Event::Touch(_))
            && !matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }))
        {
            cursor
        } else {
            self.effective(menu, cursor)
        };
        self.content.as_widget_mut().update(
            self.tree,
            event,
            menu,
            rows_cursor,
            renderer,
            clipboard,
            shell,
            &menu.bounds(),
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
        let menu = layout.children().next().expect("the menu is laid out");
        self.content.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            menu,
            self.effective(menu, cursor),
            &menu.bounds(),
        );
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let menu = layout.children().next().expect("the menu is laid out");
        match self.content.as_widget().mouse_interaction(
            self.tree,
            menu,
            cursor,
            &menu.bounds(),
            renderer,
        ) {
            // Claim the whole window so nothing beneath reacts while open.
            mouse::Interaction::None => mouse::Interaction::Idle,
            other => other,
        }
    }
    fn index(&self) -> f32 {
        // Above the overlays of what it was opened from.
        2.
    }
}
