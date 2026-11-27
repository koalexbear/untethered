//! Profile pictures and room icons: small cropped thumbnails, fetched once
//! per mxc URI and kept for the session. The cache is bounded and keeps the
//! pictures the current view wants; a failed fetch is not retried.
use crate::{App, Msg, backend::Command, media::PreviewRef};
use iced::{Task, widget::image::Handle};
use std::collections::{BTreeMap, BTreeSet};

/// Fetches at once; the engine's media store caches what it downloaded.
const INFLIGHT: usize = 4;
/// Pictures kept, besides those still wanted.
const KEEP: usize = 512;

#[derive(Default)]
pub struct Avatars {
    cache: BTreeMap<String, Option<Handle>>,
    inflight: BTreeSet<String>,
    /// Least recently wanted first, for eviction.
    seen: Vec<String>,
}

impl Avatars {
    pub fn get(&self, mxc: Option<&str>) -> Option<&Handle> {
        self.cache.get(mxc?).and_then(Option::as_ref)
    }
    pub fn complete(&mut self, mxc: String, image: Option<Handle>) {
        if self.inflight.remove(&mxc) {
            self.cache.insert(mxc, image.map(round));
        }
    }
    fn evict(&mut self, wanted: &BTreeSet<&str>) {
        if self.cache.len() <= KEEP {
            return;
        }
        self.seen.retain(|m| self.cache.contains_key(m));
        let extra = self.cache.len() - KEEP;
        let old: Vec<String> = self
            .seen
            .iter()
            .filter(|m| !wanted.contains(m.as_str()))
            .take(extra)
            .cloned()
            .collect();
        for m in old {
            self.cache.remove(&m);
            self.seen.retain(|s| *s != m);
        }
    }
}

/// The picture cut to a circle (antialiased) in its pixels, so it is round
/// on every renderer.
fn round(image: Handle) -> Handle {
    let Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = &image
    else {
        return image;
    };
    let (w, h) = (*width, *height);
    // A centered square of the picture, which is usually square already.
    let side = w.min(h);
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    let r = side as f32 / 2.;
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let i = (((y + y0) * w + x + x0) * 4) as usize;
            let (dx, dy) = (x as f32 + 0.5 - r, y as f32 + 0.5 - r);
            let coverage = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0., 1.);
            let px = &pixels[i..i + 4];
            out.extend_from_slice(&[px[0], px[1], px[2], (f32::from(px[3]) * coverage) as u8]);
        }
    }
    Handle::from_rgba(side, side, out)
}

impl App {
    /// The pictures on screen now: the account's, the room list's, and the
    /// senders and members of the open room.
    fn wanted_avatars(&self) -> BTreeSet<&str> {
        let mut wanted = BTreeSet::new();
        if let Some(own) = &self.own_avatar {
            wanted.insert(own.as_str());
        }
        let rooms = &self.live.list;
        for r in rooms.values() {
            if let Some(a) = &r.avatar {
                wanted.insert(a.as_str());
            }
        }
        // Invites show the room's picture and the inviter's (invites.rs).
        for i in &self.live.invites {
            wanted.extend(i.avatar.as_deref());
            wanted.extend(i.inviter_avatar.as_deref());
        }
        if let Some(r) = rooms.get(&self.selected) {
            let range = self.timeline_range(r);
            for p in r.posts().skip(range.start).take(range.end - range.start) {
                if let Some(a) = r.avatars.get(&p.sender) {
                    wanted.insert(a.as_str());
                }
            }
            #[cfg(target_os = "linux")]
            for user in r.voice_users() {
                if let Some(a) = r.avatars.get(user) {
                    wanted.insert(a.as_str());
                }
            }
            if self.people {
                wanted.extend(r.avatars.values().map(String::as_str));
                wanted.extend(
                    self.directory
                        .users
                        .values()
                        .filter_map(|m| m.avatar.as_deref()),
                );
            }
        }
        // People in the composer's autocomplete popup.
        wanted.extend(self.completion_avatars());
        // The call's people, whichever room is open.
        #[cfg(target_os = "linux")]
        if let Some(r) = self.call.as_ref().and_then(|c| rooms.get(&c.room)) {
            let mut users = r.voice_users();
            users.extend(r.dm_peer.as_deref());
            for user in users {
                if let Some(a) = r.avatars.get(user) {
                    wanted.insert(a.as_str());
                }
            }
        }
        wanted
    }

    pub fn schedule_avatars(&mut self) -> Task<Msg> {
        if !self.logged_in || !self.connected || self.avatars.inflight.len() >= INFLIGHT {
            return Task::none();
        }
        let wanted = self.wanted_avatars();
        let missing: Vec<String> = wanted
            .iter()
            .filter(|m| {
                !self.avatars.cache.contains_key(**m) && !self.avatars.inflight.contains(**m)
            })
            .map(|m| (*m).to_owned())
            .collect();
        let wanted: Vec<String> = wanted.into_iter().map(str::to_owned).collect();
        let pictures = &mut self.avatars;
        pictures.seen.retain(|m| !wanted.contains(m));
        pictures.seen.extend(wanted.iter().cloned());
        pictures.evict(&wanted.iter().map(String::as_str).collect());
        let mut tasks = Vec::new();
        for mxc in missing {
            if pictures.inflight.len() >= INFLIGHT {
                break;
            }
            let (tx, rx) = async_channel::bounded(1);
            let source = PreviewRef {
                mxc: mxc.clone(),
                encrypted: false,
                fit: crate::media::Fit::Avatar,
            };
            if self
                .commands
                .try_send(Command::Inline { source, result: tx })
                .is_err()
            {
                break;
            }
            pictures.inflight.insert(mxc.clone());
            tasks.push(Task::perform(
                async move { rx.recv().await.ok().flatten() },
                move |img| Msg::AvatarReady(mxc.clone(), img),
            ));
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_cut_to_a_centered_circle() {
        // A 4:3 opaque white picture becomes a square with clear corners.
        let rounded = round(Handle::from_rgba(40, 30, vec![255; 40 * 30 * 4]));
        let Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = rounded
        else {
            panic!("not a bitmap")
        };
        assert_eq!((width, height), (30, 30));
        let alpha = |x: usize, y: usize| pixels[(y * 30 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(29, 29), 0);
        assert_eq!(alpha(15, 15), 255);
        // The edge is antialiased.
        assert!(alpha(15, 0) > 240);
    }
}
