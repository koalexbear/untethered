//! Image previews for the room on screen, every one in the timeline's shown
//! window, nearest the scroll position first. Downloads and raster decoding
//! stay off the render loop, a few at a time; the window bounds the work and
//! the decoded pictures kept, and a failed preview retries a couple of times
//! on its own, and again once the connection comes back.
use crate::{App, Msg, backend::Command, media::PreviewRef};
use iced::{Task, widget::image::Handle};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Duration,
};

/// Previews fetched and decoded at once.
const IN_FLIGHT: usize = 4;
/// Fetches before a preview shows its Retry button.
const ATTEMPTS: u8 = 3;

#[derive(Default)]
pub struct InlineImages {
    epoch: u64,
    room: String,
    cache: BTreeMap<PreviewRef, Option<Handle>>,
    inflight: BTreeSet<(u64, PreviewRef)>,
    manual: VecDeque<PreviewRef>,
    /// Failed fetches so far, and the previews waiting to try again.
    attempts: BTreeMap<PreviewRef, u8>,
    retrying: BTreeSet<PreviewRef>,
    /// How far up the timeline's shown window is scrolled: 0 at the newest
    /// message, 1 at the oldest.
    pub depth: f32,
    offline: bool,
}
impl InlineImages {
    pub fn reset(&mut self) {
        self.epoch += 1;
        self.room.clear();
        self.cache.clear();
        self.manual.clear();
        self.attempts.clear();
        self.retrying.clear();
        self.depth = 0.;
        // Count outstanding reads against the global concurrency limit until
        // they return, even across rapid room switches.
    }
    pub fn ready(&self, source: &PreviewRef) -> Option<&Handle> {
        self.cache.get(source).and_then(Option::as_ref)
    }
    /// Not shown yet, and not given up on.
    pub fn pending(&self, source: &PreviewRef) -> bool {
        !self.cache.contains_key(source)
    }
    /// Retry a preview now, with its attempts counted afresh.
    pub fn request(&mut self, source: PreviewRef) {
        self.cache.remove(&source);
        self.attempts.remove(&source);
        self.retrying.remove(&source);
        self.manual.retain(|old| old != &source);
        self.manual.push_front(source);
        self.manual.truncate(8);
    }
    pub fn complete(&mut self, epoch: u64, source: PreviewRef, image: Option<Handle>) -> Task<Msg> {
        if !self.inflight.remove(&(epoch, source.clone())) || epoch != self.epoch {
            return Task::none();
        }
        if image.is_some() {
            self.attempts.remove(&source);
            self.cache.insert(source, image);
            return Task::none();
        }
        let tries = self.attempts.entry(source.clone()).or_default();
        *tries += 1;
        if *tries >= ATTEMPTS {
            self.cache.insert(source, None);
            return Task::none();
        }
        // 4 s, then 8 s.
        let wait = Duration::from_secs(2 << *tries);
        self.retrying.insert(source.clone());
        Task::perform(crate::engine::rt::sleep(wait), move |()| {
            Msg::RetryInline(epoch, source)
        })
    }
    pub fn retry(&mut self, epoch: u64, source: &PreviewRef) {
        if epoch == self.epoch {
            self.retrying.remove(source);
        }
    }
    /// Back online: try what failed again.
    fn forget_failures(&mut self) {
        self.cache.retain(|_, image| image.is_some());
        self.attempts.clear();
        self.retrying.clear();
    }
}

impl App {
    pub fn schedule_inline(&mut self) -> Task<Msg> {
        if !self.connected {
            self.inline.offline = true;
            return Task::none();
        }
        if std::mem::take(&mut self.inline.offline) {
            self.inline.forget_failures();
        }
        if !self.logged_in || !self.verified || self.account || self.mentions.is_some() {
            return Task::none();
        }
        let Some(room) = self.live.list.get(&self.selected) else {
            return Task::none();
        };
        if self.inline.room != self.selected {
            self.inline.reset();
            self.inline.room.clone_from(&self.selected);
        }
        // The messages the timeline shows: the window of the room's history,
        // or of the context around a message jumped to.
        let original = self.original.as_ref().filter(|c| c.room == self.selected);
        let history = original.and_then(|c| c.slice.as_ref()).unwrap_or(room);
        let range = self.timeline_range(history);
        // What's on screen loads first: around the message jumped to, or as
        // far up the window as the timeline is scrolled.
        let focus = original
            .and_then(|c| {
                history
                    .posts()
                    .position(|p| p.event_id.as_deref() == Some(c.event.as_str()))
            })
            .unwrap_or_else(|| {
                let span = range.len().saturating_sub(1) as f32;
                range.end.saturating_sub(1) - (self.inline.depth.clamp(0., 1.) * span) as usize
            });
        let mut shown: Vec<(usize, PreviewRef)> = history
            .posts_in(range.clone())
            .enumerate()
            .filter_map(|(i, p)| Some((range.start + i, p.media.as_ref()?.preview.clone()?)))
            .collect();
        // Link previews' images too, where the room shows them.
        if room.link_previews {
            for (i, post) in history.posts_in(range.clone()).enumerate() {
                for url in crate::link_previews::links(post) {
                    if let Some(source) = self
                        .link_previews
                        .get(&url)
                        .and_then(crate::view::link_preview_image)
                    {
                        shown.push((range.start + i, source));
                    }
                }
            }
        }
        shown.sort_by_key(|(i, _)| i.abs_diff(focus));
        let images = &mut self.inline;
        let mut wanted: Vec<PreviewRef> = images.manual.iter().cloned().collect();
        for (_, key) in shown {
            if !wanted.contains(&key) {
                wanted.push(key);
            }
        }
        images.cache.retain(|k, _| wanted.contains(k));
        images.attempts.retain(|k, _| wanted.contains(k));
        images.retrying.retain(|k| wanted.contains(k));
        let mut tasks = Vec::new();
        let epoch = images.epoch;
        for key in wanted {
            if images.inflight.len() >= IN_FLIGHT {
                break;
            }
            if images.cache.contains_key(&key)
                || images.retrying.contains(&key)
                || images.inflight.contains(&(epoch, key.clone()))
            {
                continue;
            }
            let (tx, rx) = async_channel::bounded(1);
            if self
                .commands
                .try_send(Command::Inline {
                    source: key.clone(),
                    result: tx,
                })
                .is_err()
            {
                break;
            }
            images.inflight.insert((epoch, key.clone()));
            tasks.push(Task::perform(
                async move { rx.recv().await.ok().flatten() },
                move |img| Msg::InlineReady(epoch, key, img),
            ));
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Fit;

    fn key() -> PreviewRef {
        PreviewRef {
            mxc: "mxc://s/picture".into(),
            encrypted: false,
            fit: Fit::Inline,
        }
    }
    fn finish(images: &mut InlineImages, image: Option<Handle>) {
        images.inflight.insert((images.epoch, key()));
        let _ = images.complete(images.epoch, key(), image);
    }

    #[test]
    fn a_failed_preview_tries_three_times_then_offers_retry() {
        let mut images = InlineImages::default();
        for _ in 0..2 {
            finish(&mut images, None);
            assert!(images.pending(&key()), "still loading while it waits");
            assert!(images.retrying.contains(&key()));
            images.retry(images.epoch, &key());
            assert!(!images.retrying.contains(&key()));
        }
        finish(&mut images, None);
        assert!(!images.pending(&key()) && images.ready(&key()).is_none());
        // Back online, it loads again; then it shows.
        images.forget_failures();
        assert!(images.pending(&key()) && images.attempts.is_empty());
        finish(&mut images, Some(Handle::from_rgba(1, 1, vec![0; 4])));
        assert!(images.ready(&key()).is_some());
    }

    #[test]
    fn a_late_result_for_another_room_is_dropped() {
        let mut images = InlineImages::default();
        images.inflight.insert((images.epoch, key()));
        let old = images.epoch;
        images.reset();
        let _ = images.complete(old, key(), Some(Handle::from_rgba(1, 1, vec![0; 4])));
        assert!(images.ready(&key()).is_none() && images.inflight.is_empty());
    }
}
