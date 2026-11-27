//! The media viewer: an image or video over the window, as on Discord, with
//! the room's other images and videos one arrow away, and a strip of them
//! along the bottom. The pictures either side load ahead, so stepping through
//! doesn't wait; zooming into a large one loads it sharper (ROADMAP P3).
use crate::{
    App, Msg,
    backend::Command,
    media::{Kind, Media, PreviewRef},
    preview::{Frames, Picture},
    rooms::{Post, Room},
};
use iced::{Task, widget::image::Handle};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

/// Decoded originals kept while the viewer is open: the one shown, those
/// either side, and the last few seen.
const KEEP: usize = 5;
/// Thumbnails either side of the shown one in the strip.
pub const STRIP: usize = 7;

/// Originals the viewer has decoded or is decoding.
#[derive(Default)]
pub struct Originals {
    kept: VecDeque<(PreviewRef, Picture)>,
    loading: BTreeSet<PreviewRef>,
    failed: BTreeSet<PreviewRef>,
}
impl Originals {
    pub fn get(&self, source: &PreviewRef) -> Option<&Picture> {
        self.kept.iter().find(|(s, _)| s == source).map(|(_, h)| h)
    }
    fn put(&mut self, source: PreviewRef, handle: Picture) {
        self.kept.retain(|(s, _)| s != &source);
        self.kept.push_back((source, handle));
        while self.kept.len() > KEEP {
            self.kept.pop_front();
        }
    }
    fn wanted(&self, source: &PreviewRef) -> bool {
        self.get(source).is_none()
            && !self.loading.contains(source)
            && !self.failed.contains(source)
    }
}

pub struct Viewer {
    pub room: String,
    pub event: String,
    /// The full-size image, once loaded, and sharper once zoomed into; an
    /// animated one's frames (ROADMAP P1).
    pub image: Option<Handle>,
    pub sharp: Option<Handle>,
    pub moving: Option<Arc<Frames>>,
    pub failed: bool,
    epoch: u64,
    /// The browser: the video plays in the page's own player, over the
    /// stage (ROADMAP P4).
    #[cfg(target_arch = "wasm32")]
    pub web_video: bool,
    /// Linux: the playing video and its decrypted copy, removed on close.
    #[cfg(target_os = "linux")]
    pub video: Option<crate::player::Player>,
    #[cfg(target_os = "linux")]
    file: Option<tempfile::TempPath>,
}

/// The room's viewable media in timeline order: images and videos that
/// have been sent.
pub fn gallery(r: &Room) -> impl DoubleEndedIterator<Item = (&Post, &Media)> {
    r.posts().filter_map(|p| {
        let media = p.media.as_ref().filter(|m| m.viewable())?;
        p.event_id.as_ref()?;
        Some((p, media))
    })
}

impl Viewer {
    /// What the strip shows: the gallery's items either side of this one.
    pub fn strip<'a>(&self, r: &'a Room) -> Vec<(&'a Post, &'a Media)> {
        let items: Vec<_> = gallery(r).collect();
        let at = items
            .iter()
            .position(|(p, _)| p.event_id.as_deref() == Some(self.event.as_str()))
            .unwrap_or(0);
        let start = at.saturating_sub(STRIP);
        let end = (at + STRIP + 1).min(items.len());
        items[start..end].to_vec()
    }
    /// The shown item and its place in the gallery (1-based) and size.
    pub fn current<'a>(&self, r: &'a Room) -> Option<(&'a Post, &'a Media, usize, usize)> {
        let mut found = None;
        let mut total = 0;
        for (p, m) in gallery(r) {
            total += 1;
            if p.event_id.as_deref() == Some(self.event.as_str()) {
                found = Some((p, m, total));
            }
        }
        found.map(|(p, m, i)| (p, m, i, total))
    }
}

impl App {
    pub fn open_viewer(&mut self, event: String) -> Task<Msg> {
        let room = self.selected.clone();
        let Some(media) = self
            .live
            .list
            .get(&room)
            .and_then(|r| gallery(r).find(|(p, _)| p.event_id.as_deref() == Some(&event)))
            .map(|(_, m)| m.clone())
        else {
            return Task::none();
        };
        let epoch = self.viewer.as_ref().map_or(0, |v| v.epoch) + 1;
        self.copy_note = None;
        // One thing plays at a time.
        #[cfg(target_os = "linux")]
        self.stop_inline();
        #[cfg(target_arch = "wasm32")]
        crate::web_files::hide_video();
        let full = media.full();
        let cached = full.as_ref().and_then(|s| self.originals.get(s)).cloned();
        let image = cached.as_ref().map(|p| p.still().clone());
        let moving = match cached {
            Some(Picture::Moving(frames)) => Some(frames),
            _ => None,
        };
        let failed = full
            .as_ref()
            .is_some_and(|s| self.originals.failed.contains(s));
        // Dropping the old viewer stops its video and removes its file.
        self.viewer = Some(Viewer {
            room,
            event,
            image,
            sharp: None,
            moving,
            failed,
            epoch,
            #[cfg(target_arch = "wasm32")]
            web_video: false,
            #[cfg(target_os = "linux")]
            video: None,
            #[cfg(target_os = "linux")]
            file: None,
        });
        // Arrow keys and Space go to the viewer, not the composer.
        let unfocus = iced::widget::operation::focus("media-viewer");
        let load = match media.kind {
            Kind::Image => full.map_or_else(Task::none, |s| self.load_original(s)),
            #[cfg(target_os = "linux")]
            Kind::Video => self.load_video(media, epoch),
            #[cfg(target_arch = "wasm32")]
            Kind::Video => self.play_web_video(media),
            _ => Task::none(),
        };
        Task::batch([unfocus, load, self.load_neighbours()])
    }

    /// The browser: fetch the video and play it in the page's own player.
    #[cfg(target_arch = "wasm32")]
    fn play_web_video(&mut self, media: Media) -> Task<Msg> {
        let strip = self
            .viewer
            .as_ref()
            .and_then(|v| Some(v.strip(self.live.list.get(&v.room)?).len() > 1))
            .unwrap_or(false);
        // Above the caption, and the strip when there is one.
        crate::web_files::place_video(if strip { 108. } else { 40. });
        self.seq += 1;
        if self
            .commands
            .try_send(Command::Download {
                id: self.seq,
                media,
                play: true,
            })
            .is_ok()
        {
            self.pending.insert(self.seq, crate::Request::PlayVideo);
        }
        Task::none()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn web_video_ready(&mut self, ok: bool) {
        if let Some(v) = &mut self.viewer {
            v.web_video = ok;
            v.failed = !ok;
        }
    }

    /// Decode an original, unless it's kept, loading or failed already.
    fn load_original(&mut self, source: PreviewRef) -> Task<Msg> {
        if !self.originals.wanted(&source) {
            return Task::none();
        }
        let (tx, rx) = async_channel::bounded(1);
        if self
            .commands
            .try_send(Command::Inline {
                source: source.clone(),
                result: tx,
            })
            .is_err()
        {
            return Task::none();
        }
        self.originals.loading.insert(source.clone());
        Task::perform(async move { rx.recv().await.ok().flatten() }, move |img| {
            Msg::Original(source.clone(), img)
        })
    }

    /// The pictures either side of the shown one, loaded ahead.
    fn load_neighbours(&mut self) -> Task<Msg> {
        let Some(v) = &self.viewer else {
            return Task::none();
        };
        let Some(r) = self.live.list.get(&v.room) else {
            return Task::none();
        };
        let items: Vec<&Media> = gallery(r).map(|(_, m)| m).collect();
        let Some(at) = gallery(r).position(|(p, _)| p.event_id.as_deref() == Some(&v.event)) else {
            return Task::none();
        };
        let sources: Vec<PreviewRef> = [at.checked_sub(1), Some(at + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| items.get(i)?.full())
            .collect();
        Task::batch(sources.into_iter().map(|s| self.load_original(s)))
    }

    /// Zoomed into: load the picture sharper, if it's larger than shown.
    pub fn viewer_zoomed(&mut self) -> Task<Msg> {
        let Some(v) = self.viewer.as_ref().filter(|v| v.sharp.is_none()) else {
            return Task::none();
        };
        let sharp = self
            .live
            .list
            .get(&v.room)
            .and_then(|r| v.current(r))
            .and_then(|(_, m, ..)| m.sharp());
        sharp.map_or_else(Task::none, |s| self.load_original(s))
    }

    /// An original arrived (or didn't).
    pub fn original_loaded(&mut self, source: PreviewRef, image: Option<Picture>) {
        self.originals.loading.remove(&source);
        let Some(v) = &mut self.viewer else {
            return;
        };
        let current = self
            .live
            .list
            .get(&v.room)
            .and_then(|r| v.current(r))
            .map(|(_, m, ..)| (m.full(), m.sharp()));
        let (full, sharp) = current.unwrap_or_default();
        if source.fit == crate::media::Fit::Zoom {
            // Only the shown picture is kept sharp, and not with the others.
            if image.is_none() {
                self.originals.failed.insert(source.clone());
            }
            if sharp.as_ref() == Some(&source) {
                v.sharp = image.map(|p| p.still().clone());
            }
            return;
        }
        if full.as_ref() == Some(&source) {
            v.failed = image.is_none();
            v.image = image.as_ref().map(|p| p.still().clone());
            v.moving = match &image {
                Some(Picture::Moving(frames)) => Some(frames.clone()),
                _ => None,
            };
        }
        match image {
            Some(handle) => self.originals.put(source, handle),
            None => {
                self.originals.failed.insert(source);
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn load_video(&mut self, media: Media, epoch: u64) -> Task<Msg> {
        crate::player::warm_up();
        // The decrypted copy goes to the user's private runtime directory
        // (a tmpfs), and is removed when the viewer moves on or closes.
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let suffix = std::path::Path::new(&media.name)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let Ok(file) = tempfile::Builder::new()
            .prefix("untethered-video-")
            .suffix(&suffix)
            .tempfile_in(dir)
        else {
            return self.viewer_failed(epoch);
        };
        let path = file.into_temp_path();
        let destination = path.to_path_buf();
        if let Some(v) = &mut self.viewer {
            v.file = Some(path);
        }
        let (tx, rx) = async_channel::bounded(1);
        if self
            .commands
            .try_send(Command::Fetch {
                media,
                destination,
                result: tx,
            })
            .is_err()
        {
            return self.viewer_failed(epoch);
        }
        Task::perform(async move { rx.recv().await.unwrap_or(false) }, move |ok| {
            Msg::ViewerVideo(epoch, ok)
        })
    }

    #[cfg(target_os = "linux")]
    fn viewer_failed(&mut self, epoch: u64) -> Task<Msg> {
        Task::done(Msg::ViewerVideo(epoch, false))
    }

    /// The previous (-1) or next (+1) image or video in the room. Past the
    /// oldest one loaded, ask for older messages.
    pub fn step_viewer(&mut self, delta: i32) -> Task<Msg> {
        let Some(v) = &self.viewer else {
            return Task::none();
        };
        let Some(r) = self.live.list.get(&v.room) else {
            return Task::none();
        };
        let items: Vec<&str> = gallery(r)
            .filter_map(|(p, _)| p.event_id.as_deref())
            .collect();
        let Some(at) = items.iter().position(|e| *e == v.event) else {
            return Task::none();
        };
        let next = at as i64 + i64::from(delta);
        if next < 0 {
            if !r.end {
                self.page();
            }
            return Task::none();
        }
        match items.get(next as usize) {
            Some(event) => {
                let event = (*event).to_owned();
                self.open_viewer(event)
            }
            None => Task::none(),
        }
    }

    pub fn close_viewer(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if self.viewer.is_some() {
            crate::web_files::hide_video();
        }
        self.viewer = None;
        self.copy_note = None;
        // The decoded originals are large; they load again next time.
        self.originals = Originals::default();
    }

    #[cfg(target_os = "linux")]
    pub fn viewer_video(&mut self, epoch: u64, fetched: bool) {
        let Some(v) = self.viewer.as_mut().filter(|v| v.epoch == epoch) else {
            return;
        };
        let opened = v
            .file
            .as_ref()
            .filter(|_| fetched)
            .map(|path| crate::player::Player::open(path));
        match opened {
            Some(Ok(player)) => v.video = Some(player),
            Some(Err(reason)) => {
                eprintln!("Media viewer: {reason}");
                v.failed = true;
            }
            None => v.failed = true,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn player_event(&mut self, event: crate::player::Event) {
        use crate::player::Event;
        let Some(v) = &mut self.viewer else {
            return;
        };
        let Some(player) = &mut v.video else {
            return;
        };
        match event {
            Event::Frame(frame) => {
                player.frame = Some(frame);
                player.poll();
            }
            Event::Ended => player.finished(),
            Event::Failed(reason) => {
                eprintln!("Media viewer: {reason}");
                v.video = None;
                v.failed = true;
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub fn player_control(&mut self, control: crate::player::Control) {
        if let Some(player) = self.viewer.as_mut().and_then(|v| v.video.as_mut()) {
            player.control(control);
        }
    }

    #[cfg(target_os = "linux")]
    pub fn player_scrub(&mut self, at: f32) {
        if let Some(player) = self.viewer.as_mut().and_then(|v| v.video.as_mut()) {
            player.scrub = Some(at);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::{
        App, Msg,
        backend::Command,
        engine::{Event, Message, RoomList, RoomSummary, TimelineSnapshot},
        media::{Fit, PreviewRef},
    };
    use iced::widget::image::Handle;
    use serde_json::json;

    pub(crate) fn picture(row: i64, sender: &str, time: i64) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: sender.into(),
            sender_name: None,
            sender_avatar: None,
            time,
            body: format!("{row}.png"),
            formatted: None,
            kind: crate::engine::Kind::Attachment,
            encrypted: false,
            delivery: crate::engine::Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: false,
            editable: false,
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: Some(json!({
                "msgtype": "m.image",
                "body": format!("{row}.png"),
                "url": format!("mxc://h/{row}"),
                "info": {"w": 4000, "h": 3000, "mimetype": "image/png", "size": 1000}
            })),
            from: None,
        }
    }

    pub(crate) fn room_with(pictures: Vec<Message>) -> (App, async_channel::Receiver<Command>) {
        let (backend, commands) = async_channel::bounded(64);
        let (sdk, _) = async_channel::bounded(64);
        let mut app = App::new(backend);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![RoomSummary {
                id: "!r:h".into(),
                name: "pictures".into(),
                ..Default::default()
            }],
            children: Default::default(),
        }));
        let _ = app.update(Msg::Open("!r:h".into()));
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: "!r:h".into(),
            messages: pictures,
            start_reached: true,
            latest_event: None,
            seams: Vec::new(),
        }));
        while commands.try_recv().is_ok() {}
        (app, commands)
    }

    fn full(row: i64) -> PreviewRef {
        PreviewRef {
            mxc: format!("mxc://h/{row}"),
            encrypted: false,
            fit: Fit::Full,
        }
    }

    fn loads(commands: &async_channel::Receiver<Command>) -> Vec<PreviewRef> {
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|c| match c {
                Command::Inline { source, .. } if source.fit != Fit::Inline => Some(source),
                _ => None,
            })
            .collect()
    }

    fn pixel() -> Option<crate::preview::Picture> {
        Some(crate::preview::Picture::Still(Handle::from_rgba(
            1,
            1,
            vec![0; 4],
        )))
    }

    #[test]
    fn the_pictures_either_side_load_ahead_and_open_at_once() {
        let (mut app, commands) =
            room_with((1..=3).map(|n| picture(n, "@mira:h", n * 1000)).collect());
        let _ = app.open_viewer("$2".into());
        let mut wanted = loads(&commands);
        wanted.sort();
        assert_eq!(wanted, [full(1), full(2), full(3)]);
        let _ = app.update(Msg::Original(full(3), pixel()));
        let _ = app.update(Msg::Original(full(2), pixel()));
        assert!(app.viewer.as_ref().unwrap().image.is_some());
        // Stepping on shows the next picture at once, and loads nothing
        // it already has or is loading.
        let _ = app.step_viewer(1);
        let v = app.viewer.as_ref().unwrap();
        assert_eq!(v.event, "$3");
        assert!(v.image.is_some(), "loaded ahead");
        assert!(loads(&commands).is_empty());
        // Closing lets go of the decoded originals.
        app.close_viewer();
        assert!(app.originals.get(&full(3)).is_none());
    }

    #[test]
    fn zooming_into_a_large_picture_loads_it_sharper_once() {
        let (mut app, commands) = room_with(vec![picture(1, "@mira:h", 1000)]);
        let _ = app.open_viewer("$1".into());
        let _ = app.update(Msg::Original(full(1), pixel()));
        let _ = loads(&commands);
        let _ = app.update(Msg::ViewerZoomed);
        let sharp = PreviewRef {
            fit: Fit::Zoom,
            ..full(1)
        };
        assert_eq!(loads(&commands), std::slice::from_ref(&sharp));
        let _ = app.update(Msg::ViewerZoomed);
        assert!(loads(&commands).is_empty(), "once");
        let _ = app.update(Msg::Original(sharp, pixel()));
        assert!(app.viewer.as_ref().unwrap().sharp.is_some());
    }
}
