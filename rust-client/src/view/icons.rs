//! Navigation and message icons, drawn as strokes on a 24-unit grid.
//!
//! A canvas keeps them crisp at fractional desktop scale. Each glyph caches its
//! geometry and only redraws when its icon, color, hover state or size changes.
use super::{BOLD, El, FLOAT, Msg, TEXT, surface};
use iced::{
    Color, Point, Rectangle, Renderer, Size, Theme, Vector, mouse,
    widget::{
        button,
        canvas::{self, Cache, Canvas, Path, Stroke, path::Arc},
        container, text, tooltip,
    },
};
use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Icon {
    Hash,
    Speaker,
    At,
    Members,
    #[cfg_attr(
        not(any(target_os = "linux", target_arch = "wasm32")),
        allow(dead_code)
    )]
    Bell,
    Search,
    Gear,
    Plus,
    Reply,
    Forward,
    /// Bars of a chart: Create Poll and End Poll (polls.rs).
    Poll,
    /// A tick: a picked answer in a multiple-choice poll.
    Check,
    /// A sticker with a peeled corner: the composer's sticker picker.
    Sticker,
    Smile,
    Pencil,
    Flag,
    Copy,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Close,
    Refresh,
    Pin,
    Threads,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Play,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Pause,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Muted,
    /// Four corners: open in the media viewer.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Expand,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Phone,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Video,
    Shield,
    Download,
    File,
    /// A server folder in the rail (rail_order.rs).
    Folder,
    Lock,
    UserPlus,
    Send,
    /// Explore (explore.rs).
    Compass,
    // The call controls: the Voice Connected panel and the user panel's
    // Mute and Deafen (view/voice_panel.rs).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Mic,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    MicOff,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Headphones,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    HeadphonesOff,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    ScreenShare,
    /// A handset laid down: leave the call.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    PhoneOff,
    /// Rising bars: the call's connection.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Signal,
}

/// Interactive icon color, matching channel and header controls.
pub(super) const ICON: Color = iced::color!(0xb5bac1);
pub(super) const ICON_OFF: Color = iced::color!(0x5c5f66);

struct Glyph {
    icon: Icon,
    color: Color,
    hover: Option<Color>,
    size: f32,
}

#[derive(Default)]
pub(super) struct GlyphState {
    cache: Cache,
    key: Cell<u64>,
}

fn arc(b: &mut canvas::path::Builder, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
    b.arc(Arc {
        center: Point::new(cx, cy),
        radius: r,
        start_angle: from.to_radians().into(),
        end_angle: to.to_radians().into(),
    });
}

fn poly(b: &mut canvas::path::Builder, pts: &[(f32, f32)]) {
    b.move_to(Point::new(pts[0].0, pts[0].1));
    for &(x, y) in &pts[1..] {
        b.line_to(Point::new(x, y));
    }
}

fn outline(icon: Icon) -> Path {
    let p = Point::new;
    Path::new(|b| match icon {
        Icon::Hash => {
            poly(b, &[(4., 9.), (20., 9.)]);
            poly(b, &[(4., 15.), (20., 15.)]);
            poly(b, &[(10., 3.), (8., 21.)]);
            poly(b, &[(16., 3.), (14., 21.)]);
        }
        Icon::Speaker => {
            poly(
                b,
                &[
                    (11., 5.),
                    (6., 9.),
                    (2., 9.),
                    (2., 15.),
                    (6., 15.),
                    (11., 19.),
                ],
            );
            b.close();
            arc(b, 12., 12., 5., -45., 45.);
            arc(b, 12., 12., 9.5, -45., 45.);
        }
        Icon::At => {
            b.circle(p(12., 12.), 4.);
            poly(b, &[(16., 8.), (16., 13.)]);
            arc(b, 19., 13., 3., 180., 0.);
            b.line_to(p(22., 12.));
            arc(b, 12., 12., 10., 0., -306.87);
        }
        Icon::Members | Icon::UserPlus => {
            poly(b, &[(16., 21.), (16., 19.)]);
            arc(b, 12., 19., 4., 0., -90.);
            b.line_to(p(6., 15.));
            arc(b, 6., 19., 4., -90., -180.);
            b.line_to(p(2., 21.));
            b.circle(p(9., 7.), 4.);
            if icon == Icon::Members {
                poly(b, &[(22., 21.), (22., 19.)]);
                b.bezier_curve_to(p(22., 17.2), p(20.8, 15.6), p(19., 15.13));
                arc(b, 15., 7., 4., -75.5, 75.5);
            } else {
                poly(b, &[(19., 8.), (19., 14.)]);
                poly(b, &[(22., 11.), (16., 11.)]);
            }
        }
        Icon::Bell => {
            arc(b, 12., 8., 6., 180., 360.);
            b.bezier_curve_to(p(18., 15.), p(21., 17.), p(21., 17.));
            b.line_to(p(3., 17.));
            b.bezier_curve_to(p(3., 17.), p(6., 15.), p(6., 8.));
            b.move_to(p(10.3, 21.));
            b.quadratic_curve_to(p(12., 22.7), p(13.7, 21.));
        }
        Icon::Search => {
            b.circle(p(11., 11.), 7.);
            poly(b, &[(20.5, 20.5), (16., 16.)]);
        }
        Icon::Gear => {
            let mut first = true;
            for i in 0..8 {
                let t = i as f32 * 45.;
                for (r, a) in [
                    (7.4, t - 16.5),
                    (10., t - 9.),
                    (10., t + 9.),
                    (7.4, t + 16.5),
                ] {
                    let (s, c) = a.to_radians().sin_cos();
                    let q = p(12. + r * c, 12. + r * s);
                    if first {
                        b.move_to(q);
                        first = false;
                    } else {
                        b.line_to(q);
                    }
                }
            }
            b.close();
            b.circle(p(12., 12.), 3.);
        }
        Icon::Plus => {
            poly(b, &[(12., 5.), (12., 19.)]);
            poly(b, &[(5., 12.), (19., 12.)]);
        }
        Icon::Reply => {
            poly(b, &[(9., 17.), (4., 12.), (9., 7.)]);
            poly(b, &[(20., 18.), (20., 16.)]);
            arc(b, 16., 16., 4., 0., -90.);
            b.line_to(p(4., 12.));
        }
        // Reply's arrow, turned to the right.
        Icon::Forward => {
            poly(b, &[(15., 17.), (20., 12.), (15., 7.)]);
            poly(b, &[(4., 18.), (4., 16.)]);
            arc(b, 8., 16., 4., 180., 270.);
            b.line_to(p(20., 12.));
        }
        Icon::Check => poly(b, &[(5., 12.5), (10., 17.5), (19., 7.)]),
        Icon::Sticker => {
            b.move_to(p(14., 21.));
            b.line_to(p(6., 21.));
            b.quadratic_curve_to(p(3., 21.), p(3., 18.));
            b.line_to(p(3., 6.));
            b.quadratic_curve_to(p(3., 3.), p(6., 3.));
            b.line_to(p(18., 3.));
            b.quadratic_curve_to(p(21., 3.), p(21., 6.));
            b.line_to(p(21., 14.));
            b.close();
            poly(b, &[(21., 14.), (14., 14.), (14., 21.)]);
            b.move_to(p(8., 13.));
            b.quadratic_curve_to(p(10., 15.5), p(12.5, 14.5));
        }
        Icon::Poll => {
            poly(b, &[(5., 6.), (14., 6.)]);
            poly(b, &[(5., 12.), (19., 12.)]);
            poly(b, &[(5., 18.), (10., 18.)]);
        }
        Icon::Smile => {
            b.circle(p(11., 13.), 8.5);
            b.move_to(p(7.5, 15.5));
            b.bezier_curve_to(p(9., 17.6), p(13., 17.6), p(14.5, 15.5));
            poly(b, &[(20., 1.5), (20., 7.5)]);
            poly(b, &[(17., 4.5), (23., 4.5)]);
        }
        Icon::Pencil => {
            arc(b, 19., 5., 2.83, -135., 45.);
            b.line_to(p(7.5, 20.5));
            b.line_to(p(2., 22.));
            b.line_to(p(3.5, 16.5));
            b.line_to(p(17., 3.));
            poly(b, &[(15., 5.), (19., 9.)]);
        }
        Icon::Flag => {
            b.move_to(p(4., 15.));
            b.bezier_curve_to(p(6., 13.5), p(9., 13.5), p(12., 15.));
            b.bezier_curve_to(p(15., 16.5), p(18., 16.5), p(20., 15.));
            b.line_to(p(20., 3.));
            b.bezier_curve_to(p(18., 4.5), p(15., 4.5), p(12., 3.));
            b.bezier_curve_to(p(9., 1.5), p(6., 1.5), p(4., 3.));
            b.close();
            poly(b, &[(4., 22.), (4., 15.)]);
        }
        Icon::Copy => {
            b.rounded_rectangle(p(8., 8.), Size::new(14., 14.), 2.0.into());
            b.move_to(p(4., 16.));
            b.quadratic_curve_to(p(2., 16.), p(2., 14.));
            b.line_to(p(2., 4.));
            b.quadratic_curve_to(p(2., 2.), p(4., 2.));
            b.line_to(p(14., 2.));
            b.quadratic_curve_to(p(16., 2.), p(16., 4.));
        }
        Icon::ChevronDown => poly(b, &[(6., 9.), (12., 15.), (18., 9.)]),
        Icon::ChevronRight => poly(b, &[(9., 6.), (15., 12.), (9., 18.)]),
        Icon::ChevronLeft => poly(b, &[(15., 6.), (9., 12.), (15., 18.)]),
        Icon::Play => {
            poly(b, &[(6., 3.), (20., 12.), (6., 21.)]);
            b.close();
        }
        Icon::Pause => {
            b.rounded_rectangle(p(6., 4.), Size::new(4., 16.), 1.0.into());
            b.rounded_rectangle(p(14., 4.), Size::new(4., 16.), 1.0.into());
        }
        Icon::Expand => {
            poly(b, &[(4., 9.), (4., 4.), (9., 4.)]);
            poly(b, &[(15., 4.), (20., 4.), (20., 9.)]);
            poly(b, &[(20., 15.), (20., 20.), (15., 20.)]);
            poly(b, &[(9., 20.), (4., 20.), (4., 15.)]);
        }
        Icon::Muted => {
            poly(
                b,
                &[
                    (11., 5.),
                    (6., 9.),
                    (2., 9.),
                    (2., 15.),
                    (6., 15.),
                    (11., 19.),
                ],
            );
            b.close();
            poly(b, &[(22., 9.), (16., 15.)]);
            poly(b, &[(16., 9.), (22., 15.)]);
        }
        Icon::Pin => {
            poly(
                b,
                &[
                    (7., 12.),
                    (12., 7.),
                    (12., 4.),
                    (14., 2.),
                    (22., 10.),
                    (20., 12.),
                    (17., 12.),
                    (12., 17.),
                    (12., 19.),
                    (5., 12.),
                    (7., 12.),
                ],
            );
            poly(b, &[(8.5, 15.5), (3., 21.)]);
        }
        Icon::Threads => {
            poly(
                b,
                &[
                    (3., 3.),
                    (17., 3.),
                    (17., 6.),
                    (14.5, 6.),
                    (14.5, 18.),
                    (17., 18.),
                    (17., 21.),
                    (3., 21.),
                    (3., 18.),
                    (5.5, 18.),
                    (5.5, 6.),
                    (3., 6.),
                    (3., 3.),
                ],
            );
            poly(b, &[(5.5, 11.), (14.5, 7.)]);
            poly(b, &[(5.5, 16.), (14.5, 12.)]);
            b.move_to(p(14.5, 12.));
            b.bezier_curve_to(p(19.5, 10.), p(18., 17.), p(22., 17.));
        }
        Icon::Refresh => {
            arc(b, 12., 12., 8., 35., 326.);
            poly(b, &[(19., 3.), (19., 8.), (14., 8.)]);
        }
        Icon::Close => {
            poly(b, &[(18., 6.), (6., 18.)]);
            poly(b, &[(6., 6.), (18., 18.)]);
        }
        Icon::Phone => {
            b.move_to(p(4.11, 2.));
            b.line_to(p(7.11, 2.));
            b.quadratic_curve_to(p(8.95, 2.15), p(9.11, 3.72));
            b.quadratic_curve_to(p(9.3, 5.2), p(9.81, 6.53));
            b.quadratic_curve_to(p(10.1, 7.75), p(9.36, 8.64));
            b.line_to(p(8.09, 9.91));
            b.quadratic_curve_to(p(10.6, 13.4), p(14.09, 15.91));
            b.line_to(p(15.36, 14.64));
            b.quadratic_curve_to(p(16.25, 13.9), p(17.47, 14.19));
            b.quadratic_curve_to(p(18.8, 14.7), p(20.28, 14.89));
            b.quadratic_curve_to(p(21.85, 15.05), p(22., 16.92));
            b.line_to(p(22., 19.92));
            b.quadratic_curve_to(p(22.05, 22.1), p(19.82, 21.92));
            b.bezier_curve_to(p(16.6, 21.6), p(13.6, 20.5), p(11.19, 18.85));
            b.bezier_curve_to(p(8.8, 17.3), p(6.7, 15.2), p(5.19, 12.85));
            b.bezier_curve_to(p(3.5, 10.3), p(2.4, 7.3), p(2.12, 4.18));
            b.quadratic_curve_to(p(1.95, 2.1), p(4.11, 2.));
            b.close();
        }
        Icon::Video => {
            poly(b, &[(22., 8.), (16., 12.), (22., 16.)]);
            b.close();
            b.rounded_rectangle(p(2., 6.), Size::new(14., 12.), 2.0.into());
        }
        Icon::Shield => {
            b.move_to(p(12., 22.));
            b.bezier_curve_to(p(12., 22.), p(20., 18.), p(20., 12.));
            b.line_to(p(20., 5.));
            b.line_to(p(12., 2.));
            b.line_to(p(4., 5.));
            b.line_to(p(4., 12.));
            b.bezier_curve_to(p(4., 18.), p(12., 22.), p(12., 22.));
        }
        Icon::Download => {
            b.move_to(p(21., 15.));
            b.line_to(p(21., 19.));
            b.quadratic_curve_to(p(21., 21.), p(19., 21.));
            b.line_to(p(5., 21.));
            b.quadratic_curve_to(p(3., 21.), p(3., 19.));
            b.line_to(p(3., 15.));
            poly(b, &[(7., 10.), (12., 15.), (17., 10.)]);
            poly(b, &[(12., 15.), (12., 3.)]);
        }
        Icon::File => {
            b.move_to(p(14.5, 2.));
            b.line_to(p(6., 2.));
            b.quadratic_curve_to(p(4., 2.), p(4., 4.));
            b.line_to(p(4., 20.));
            b.quadratic_curve_to(p(4., 22.), p(6., 22.));
            b.line_to(p(18., 22.));
            b.quadratic_curve_to(p(20., 22.), p(20., 20.));
            b.line_to(p(20., 7.5));
            b.close();
            poly(b, &[(14., 2.), (14., 8.), (20., 8.)]);
        }
        Icon::Folder => {
            b.move_to(p(3., 6.5));
            b.quadratic_curve_to(p(3., 5.), p(4.5, 5.));
            b.line_to(p(9., 5.));
            b.line_to(p(11., 7.));
            b.line_to(p(19.5, 7.));
            b.quadratic_curve_to(p(21., 7.), p(21., 8.5));
            b.line_to(p(21., 17.5));
            b.quadratic_curve_to(p(21., 19.), p(19.5, 19.));
            b.line_to(p(4.5, 19.));
            b.quadratic_curve_to(p(3., 19.), p(3., 17.5));
            b.close();
        }
        Icon::Lock => {
            b.rounded_rectangle(p(4., 11.), Size::new(16., 10.), 2.0.into());
            poly(b, &[(7.5, 11.), (7.5, 7.5)]);
            arc(b, 12., 7.5, 4.5, 180., 360.);
            b.line_to(p(16.5, 11.));
        }
        Icon::Send => {
            poly(b, &[(22., 2.), (15., 22.), (11., 13.), (2., 9.)]);
            b.close();
            poly(b, &[(22., 2.), (11., 13.)]);
        }
        Icon::Compass => {
            b.circle(p(12., 12.), 10.);
            poly(
                b,
                &[(16.24, 7.76), (14.12, 14.12), (7.76, 16.24), (9.88, 9.88)],
            );
            b.close();
        }
        Icon::Mic | Icon::MicOff => {
            b.rounded_rectangle(p(9., 2.), Size::new(6., 12.), 3.0.into());
            poly(b, &[(19., 10.), (19., 12.)]);
            arc(b, 12., 12., 7., 0., 180.);
            b.line_to(p(5., 10.));
            poly(b, &[(12., 19.), (12., 22.)]);
            if icon == Icon::MicOff {
                poly(b, &[(3., 3.), (21., 21.)]);
            }
        }
        Icon::Headphones | Icon::HeadphonesOff => {
            poly(b, &[(3., 19.), (3., 12.)]);
            arc(b, 12., 12., 9., 180., 360.);
            b.line_to(p(21., 19.));
            b.rounded_rectangle(p(3., 14.), Size::new(5., 7.), 2.0.into());
            b.rounded_rectangle(p(16., 14.), Size::new(5., 7.), 2.0.into());
            if icon == Icon::HeadphonesOff {
                poly(b, &[(3., 3.), (21., 21.)]);
            }
        }
        Icon::ScreenShare => {
            b.rounded_rectangle(p(2., 3.), Size::new(20., 14.), 2.0.into());
            poly(b, &[(8., 21.), (16., 21.)]);
            poly(b, &[(12., 17.), (12., 21.)]);
            poly(b, &[(9., 10.), (12., 7.), (15., 10.)]);
            poly(b, &[(12., 7.), (12., 13.)]);
        }
        Icon::PhoneOff => {
            b.move_to(p(2.4, 15.));
            b.bezier_curve_to(p(1.2, 6.6), p(22.8, 6.6), p(21.6, 15.));
            b.line_to(p(16.8, 16.2));
            b.line_to(p(15.6, 12.6));
            b.line_to(p(8.4, 12.6));
            b.line_to(p(7.2, 16.2));
            b.close();
        }
        Icon::Signal => {
            poly(b, &[(6., 20.), (6., 16.)]);
            poly(b, &[(12., 20.), (12., 11.)]);
            poly(b, &[(18., 20.), (18., 5.)]);
        }
    })
}

/// Small filled details (eyes) that a zero-length stroke would not render.
fn dots(icon: Icon) -> Option<Path> {
    (icon == Icon::Smile).then(|| {
        Path::new(|b| {
            b.circle(Point::new(8., 11.), 1.1);
            b.circle(Point::new(14., 11.), 1.1);
        })
    })
}

impl canvas::Program<Msg> for Glyph {
    type State = GlyphState;
    fn draw(
        &self,
        state: &GlyphState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let color = match self.hover {
            Some(hover) if cursor.is_over(bounds) => hover,
            _ => self.color,
        };
        let [r, g, b, a] = color.into_rgba8();
        let key = (self.icon as u64) << 40
            | u64::from(u32::from_be_bytes([r, g, b, a])) << 8
            | (self.size as u64 & 0xff);
        let key = key ^ (bounds.width as u64) << 56 ^ (bounds.height as u64) << 48;
        if state.key.get() != key {
            state.cache.clear();
            state.key.set(key);
        }
        vec![state.cache.draw(renderer, bounds.size(), |f| {
            f.translate(Vector::new(
                ((bounds.width - self.size) / 2.).round(),
                ((bounds.height - self.size) / 2.).round(),
            ));
            f.scale(self.size / 24.);
            f.stroke(
                &outline(self.icon),
                Stroke::default()
                    .with_color(color)
                    .with_width(2.)
                    .with_line_cap(canvas::LineCap::Round)
                    .with_line_join(canvas::LineJoin::Round),
            );
            if let Some(d) = dots(self.icon) {
                f.fill(&d, color);
            }
        })]
    }
}
use canvas::Geometry;

/// The curved line that joins a reply to the message it quotes.
struct Spine;
impl canvas::Program<Msg> for Spine {
    type State = Cache;
    fn draw(
        &self,
        cache: &Cache,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        vec![cache.draw(renderer, bounds.size(), |f| {
            let p = Point::new;
            let path = Path::new(|b| {
                b.move_to(p(20., bounds.height));
                b.line_to(p(20., 11.));
                b.quadratic_curve_to(p(20., 7.), p(24., 7.));
                b.line_to(p(bounds.width - 2., 7.));
            });
            f.stroke(
                &path,
                Stroke::default()
                    .with_color(iced::color!(0x4e5058))
                    .with_width(2.)
                    .with_line_cap(canvas::LineCap::Round)
                    .with_line_join(canvas::LineJoin::Round),
            );
        })]
    }
}
pub(super) fn spine() -> El<'static> {
    Canvas::new(Spine).width(52).height(18).into()
}

/// A fixed-color icon of `size` pixels.
pub(super) fn icon(icon: Icon, size: f32, color: Color) -> El<'static> {
    Canvas::new(Glyph {
        icon,
        color,
        hover: None,
        size,
    })
    .width(size)
    .height(size)
    .into()
}

/// An icon centered in a `area`-pixel square that brightens while hovered.
/// Filling the whole button lets the glyph follow the button's hover state.
pub(super) fn hover_icon(
    icon: Icon,
    size: f32,
    area: f32,
    color: Color,
    hover: Color,
) -> El<'static> {
    Canvas::new(Glyph {
        icon,
        color,
        hover: Some(hover),
        size,
    })
    .width(area)
    .height(area)
    .into()
}

pub(super) fn tip<'a>(hint: impl text::IntoFragment<'a>) -> El<'a> {
    container(text(hint).size(14).font(BOLD).color(TEXT))
        .padding([6, 10])
        .into()
}

pub(super) fn with_tip<'a>(
    content: impl Into<El<'a>>,
    hint: impl text::IntoFragment<'a>,
    position: tooltip::Position,
) -> El<'a> {
    tooltip(content, tip(hint), position)
        .gap(6)
        .style(|_| surface(FLOAT, 5.))
        .into()
}

/// A borderless icon button with a tooltip, like Discord's header controls.
pub(super) fn icon_button<'a>(
    glyph: Icon,
    hint: &'a str,
    active: bool,
    action: Option<Msg>,
    position: tooltip::Position,
) -> El<'a> {
    let color = if action.is_none() {
        ICON_OFF
    } else if active {
        TEXT
    } else {
        ICON
    };
    let hover = if action.is_none() { color } else { TEXT };
    let control = button(hover_icon(glyph, 22., 32., color, hover))
        .padding(0)
        .style(|_, _| button::Style::default())
        .on_press_maybe(action);
    with_tip(control, hint, position)
}
