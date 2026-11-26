//! The tray icon, drawn at whatever size the tray asks for: a chat bubble in
//! the app's blurple, a red dot for mentions or a white one for other unread
//! messages at the top right, and the call at the bottom right (green while
//! you can talk, grey while muted or deafened, also green while a call rings).
//!
//! Untethered has no icon of its own yet (the desktop file names the theme's
//! `internet-chat`), and a themed icon can't carry two marks: tray hosts put
//! an overlay in one corner only. So the icon is drawn, the same on every
//! desktop and on Windows.
use super::{Badge, Snapshot};

const BLURPLE: [u8; 3] = [0x58, 0x65, 0xf2];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const RED: [u8; 3] = [0xf2, 0x3f, 0x43];
const GREEN: [u8; 3] = [0x23, 0xa5, 0x5a];
const GREY: [u8; 3] = [0x80, 0x84, 0x8e];

/// Straight (not premultiplied) RGBA, row by row.
pub struct Image {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// A mark in a corner: its centre, radius and colour, and the clear gap
/// around it that keeps it apart from the bubble.
struct Dot {
    x: f32,
    y: f32,
    r: f32,
    color: [u8; 3],
}
impl Dot {
    const GAP: f32 = 0.07;
    fn covers(&self, x: f32, y: f32, grow: f32) -> bool {
        let (dx, dy) = (x - self.x, y - self.y);
        dx * dx + dy * dy <= (self.r + grow) * (self.r + grow)
    }
}

fn marks(s: &Snapshot) -> (Option<Dot>, Option<Dot>) {
    let badge = match s.badge {
        Badge::Mentions(_) => Some(Dot {
            x: 0.78,
            y: 0.22,
            r: 0.2,
            color: RED,
        }),
        Badge::Unread => Some(Dot {
            x: 0.8,
            y: 0.2,
            r: 0.14,
            color: WHITE,
        }),
        Badge::None => None,
    };
    let call = if s.ringing.is_some() {
        Some(GREEN)
    } else {
        s.call
            .as_ref()
            .map(|c| if c.muted || c.deafened { GREY } else { GREEN })
    }
    .map(|color| Dot {
        x: 0.8,
        y: 0.8,
        r: 0.17,
        color,
    });
    (badge, call)
}

/// Inside a rounded rectangle from (x0, y0) to (x1, y1).
fn rounded(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    let dx = (x0 + r - x).max(x - (x1 - r)).max(0.);
    let dy = (y0 + r - y).max(y - (y1 - r)).max(0.);
    x >= x0 && x <= x1 && y >= y0 && y <= y1 && dx * dx + dy * dy <= r * r
}
/// Inside the triangle a, b, c.
fn triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |(ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        (bx - ax) * (p.1 - ay) - (by - ay) * (p.0 - ax)
    };
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

/// The colour at a point of the unit square, or None where it's clear.
fn sample(x: f32, y: f32, badge: Option<&Dot>, call: Option<&Dot>) -> Option<[u8; 3]> {
    for dot in [badge, call].into_iter().flatten() {
        if dot.covers(x, y, 0.) {
            return Some(dot.color);
        }
        if dot.covers(x, y, Dot::GAP) {
            return None;
        }
    }
    // Two lines of text on the bubble.
    if rounded(x, y, (0.26, 0.29, 0.74, 0.37), 0.04) || rounded(x, y, (0.26, 0.46, 0.6, 0.54), 0.04)
    {
        return Some(WHITE);
    }
    let body = rounded(x, y, (0.06, 0.1, 0.94, 0.72), 0.2);
    let tail = triangle((x, y), (0.2, 0.66), (0.44, 0.66), (0.14, 0.93));
    (body || tail).then_some(BLURPLE)
}

/// Draw the icon `size` pixels square, smoothing edges with 4×4 samples.
pub fn draw(size: u32, s: &Snapshot) -> Image {
    const N: u32 = 4;
    let (badge, call) = marks(s);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut hits) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f32 + (sx as f32 + 0.5) / N as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / N as f32) / size as f32;
                    if let Some(c) = sample(x, y, badge.as_ref(), call.as_ref()) {
                        r += c[0] as u32;
                        g += c[1] as u32;
                        b += c[2] as u32;
                        hits += 1;
                    }
                }
            }
            // Average the covered samples' colour; coverage is the alpha.
            let average = |sum: u32| sum.checked_div(hits).unwrap_or(0) as u8;
            rgba.extend([
                average(r),
                average(g),
                average(b),
                (hits * 255 / (N * N)) as u8,
            ]);
        }
    }
    Image { size, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::Call;

    fn pixel(img: &Image, x: f32, y: f32) -> [u8; 4] {
        let (px, py) = ((x * img.size as f32) as u32, (y * img.size as f32) as u32);
        let i = ((py * img.size + px) * 4) as usize;
        img.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn marks_follow_the_badge_and_the_call() {
        let plain = draw(32, &Snapshot::default());
        assert_eq!(pixel(&plain, 0.78, 0.22), [0x58, 0x65, 0xf2, 255], "bubble");
        assert_eq!(pixel(&plain, 0.02, 0.98)[3], 0, "clear outside the bubble");

        let busy = Snapshot {
            badge: Badge::Mentions(2),
            call: Some(Call {
                place: "general".into(),
                muted: false,
                deafened: false,
            }),
            ..Default::default()
        };
        let img = draw(32, &busy);
        assert_eq!(
            pixel(&img, 0.78, 0.22),
            [0xf2, 0x3f, 0x43, 255],
            "mention dot"
        );
        assert_eq!(pixel(&img, 0.8, 0.8), [0x23, 0xa5, 0x5a, 255], "call dot");

        let muted = Snapshot {
            badge: Badge::Unread,
            call: busy.call.clone().map(|c| Call { muted: true, ..c }),
            ..Default::default()
        };
        let img = draw(32, &muted);
        assert_eq!(pixel(&img, 0.8, 0.2), [0xff, 0xff, 0xff, 255], "unread dot");
        assert_eq!(pixel(&img, 0.8, 0.8), [0x80, 0x84, 0x8e, 255], "muted call");
    }

    #[test]
    fn draws_every_size_a_tray_asks_for() {
        for size in [16, 22, 24, 32, 48, 64] {
            assert_eq!(
                draw(size, &Snapshot::default()).rgba.len(),
                (size * size * 4) as usize
            );
        }
    }
}
