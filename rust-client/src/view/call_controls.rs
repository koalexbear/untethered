//! Vector call controls stay legible at fractional desktop scale.
use super::{El, Msg, RAIL, surface};
use iced::{
    Border, Color, Point, Rectangle, Renderer, Size, Theme, color, mouse,
    widget::{
        button,
        canvas::{self, Canvas, Frame, Geometry, Path, Stroke},
        container, text, tooltip,
    },
};

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Mic,
    Camera,
    Screen,
    Hangup,
}
struct Glyph(Icon, bool);
impl canvas::Program<Msg> for Glyph {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let p = Point::new;
        let stroke = Stroke::default()
            .with_color(Color::WHITE)
            .with_width(1.7)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let path = Path::new(|b| {
            match self.0 {
                Icon::Mic => {
                    b.rounded_rectangle(p(7., 2.), Size::new(6., 10.), 3.0.into());
                    b.move_to(p(4., 9.));
                    b.bezier_curve_to(p(4., 17.), p(16., 17.), p(16., 9.));
                    b.move_to(p(10., 15.));
                    b.line_to(p(10., 18.));
                    b.move_to(p(7., 18.));
                    b.line_to(p(13., 18.));
                }
                Icon::Camera => {
                    b.rounded_rectangle(p(2., 5.), Size::new(11., 10.), 2.0.into());
                    b.move_to(p(13., 9.));
                    b.line_to(p(18., 6.));
                    b.line_to(p(18., 14.));
                    b.line_to(p(13., 11.));
                }
                Icon::Screen => {
                    b.rounded_rectangle(p(2., 3.), Size::new(16., 12.), 2.0.into());
                    b.move_to(p(10., 15.));
                    b.line_to(p(10., 18.));
                    b.move_to(p(6., 18.));
                    b.line_to(p(14., 18.));
                }
                Icon::Hangup => {
                    b.move_to(p(2., 12.));
                    b.bezier_curve_to(p(1., 5.), p(19., 5.), p(18., 12.));
                    b.line_to(p(14., 13.));
                    b.line_to(p(13., 10.));
                    b.line_to(p(7., 10.));
                    b.line_to(p(6., 13.));
                    b.close();
                }
            }
            if self.1 {
                b.move_to(p(2., 2.));
                b.line_to(p(18., 18.));
            }
        });
        f.stroke(&path, stroke);
        vec![f.into_geometry()]
    }
}
pub(super) fn control(
    icon: Icon,
    label: &str,
    on: bool,
    slash: bool,
    action: Option<Msg>,
) -> El<'static> {
    let fill = if matches!(icon, Icon::Hangup) {
        color!(0xb53345)
    } else if on && matches!(icon, Icon::Screen | Icon::Camera) {
        color!(0x23764e)
    } else if on {
        color!(0x515468)
    } else {
        color!(0x383a40)
    };
    let control =
        button(container(Canvas::new(Glyph(icon, slash)).width(20).height(20)).center(34))
            .padding(0)
            .style(move |_, status| {
                let background = match status {
                    button::Status::Hovered | button::Status::Pressed => {
                        Color::from_rgb(fill.r + 0.1, fill.g + 0.1, fill.b + 0.1)
                    }
                    button::Status::Disabled => Color { a: 0.45, ..fill },
                    _ => fill,
                };
                button::Style {
                    background: Some(background.into()),
                    border: Border::default().rounded(8),
                    ..Default::default()
                }
            })
            .on_press_maybe(action);
    tooltip(
        control,
        container(text(label.to_owned()).size(13)).padding([4, 7]),
        tooltip::Position::Top,
    )
    .style(|_| surface(RAIL, 5.))
    .gap(6)
    .into()
}
