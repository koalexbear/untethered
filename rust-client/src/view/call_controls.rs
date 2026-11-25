//! Vector call controls stay legible at fractional desktop scale.
use super::{BOLD, El, Msg, TEXT};
use iced::{
    Alignment::Center,
    Border, Color, Point, Rectangle, Renderer, Size, Theme, color, mouse,
    widget::{
        button,
        canvas::{self, Canvas, Frame, Geometry, Path, Stroke},
        row, text,
    },
};

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Mic,
    Headphones,
    Camera,
    Screen,
    Devices,
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
                Icon::Headphones => {
                    b.move_to(p(3., 13.));
                    b.bezier_curve_to(p(2., 0.), p(18., 0.), p(17., 13.));
                    for x in [3., 13.] {
                        b.rounded_rectangle(p(x, 10.), Size::new(4., 6.), 1.5.into());
                    }
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
                Icon::Devices => {
                    for (x, y) in [(4., 7.), (10., 13.), (16., 6.)] {
                        b.move_to(p(x, 3.));
                        b.line_to(p(x, y - 2.));
                        b.circle(p(x, y), 2.);
                        b.move_to(p(x, y + 2.));
                        b.line_to(p(x, 17.));
                    }
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
    button(
        row![
            Canvas::new(Glyph(icon, slash)).width(20).height(20),
            text(label.to_owned()).size(13).font(BOLD)
        ]
        .spacing(6)
        .align_y(Center),
    )
    .padding([9, 12])
    .style(move |_, s| button::Style {
        background: Some(
            (if s == button::Status::Hovered {
                Color::from_rgb(fill.r + 0.1, fill.g + 0.1, fill.b + 0.1)
            } else {
                fill
            })
            .into(),
        ),
        text_color: TEXT,
        border: Border::default().rounded(18),
        ..Default::default()
    })
    .on_press_maybe(action)
    .into()
}
