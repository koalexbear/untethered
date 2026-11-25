//! Bounded room-local image previews. Keep downloads and raster decoding off
//! the render loop; old rooms and failed previews cannot queue infinite work.
use crate::{App, Msg, backend::Command, media::PreviewRef};
use iced::{Task, widget::image::Handle};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Default)]
pub struct InlineImages {
    epoch: u64,
    room: String,
    cache: BTreeMap<PreviewRef, Option<Handle>>,
    inflight: BTreeSet<(u64, PreviewRef)>,
    manual: VecDeque<PreviewRef>,
}
impl InlineImages {
    pub fn reset(&mut self) {
        self.epoch += 1;
        self.room.clear();
        self.cache.clear();
        self.manual.clear();
        // Count outstanding reads against the global concurrency limit until
        // they return, even across rapid room switches.
    }
    pub fn ready(&self, source: &PreviewRef) -> Option<&Handle> {
        self.cache.get(source).and_then(Option::as_ref)
    }
    pub fn pending(&self, source: &PreviewRef) -> bool {
        self.inflight.contains(&(self.epoch, source.clone()))
    }
    pub fn failed(&self, source: &PreviewRef) -> bool {
        matches!(self.cache.get(source), Some(None))
    }
    pub fn request(&mut self, source: PreviewRef) {
        self.cache.remove(&source);
        self.manual.retain(|old| old != &source);
        self.manual.push_front(source);
        self.manual.truncate(8);
    }
    pub fn complete(&mut self, epoch: u64, source: PreviewRef, image: Option<Handle>) {
        if self.inflight.remove(&(epoch, source.clone())) && epoch == self.epoch {
            self.cache.insert(source, image);
        }
    }
}

impl App {
    pub fn schedule_inline(&mut self) -> Task<Msg> {
        if !self.logged_in
            || !self.verified
            || self.account
            || self.mentions.is_some()
            || !self.connected
        {
            return Task::none();
        }
        let Some(room) = self.live.list.get(&self.selected) else {
            return Task::none();
        };
        let images = &mut self.inline;
        if images.room != self.selected {
            images.reset();
            images.room.clone_from(&self.selected);
        }
        let mut wanted: Vec<_> = images.manual.iter().cloned().collect();
        for key in room
            .posts()
            .rev()
            .take(48)
            .filter_map(|p| p.media.as_ref()?.preview.as_ref())
            .take(16)
        {
            if !wanted.contains(key) {
                wanted.push(key.clone());
            }
        }
        images.cache.retain(|k, _| wanted.contains(k));
        let mut tasks = Vec::new();
        let epoch = images.epoch;
        for key in wanted {
            if images.inflight.len() >= 2 {
                break;
            }
            if images.cache.contains_key(&key) || images.pending(&key) {
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
