//! The media viewer: an image or video over the window, as on Discord, with
//! the room's other images and videos one arrow away.
use crate::{
    App, Msg,
    backend::Command,
    media::{Kind, Media},
    rooms::{Post, Room},
};
use iced::{Task, widget::image::Handle};

pub struct Viewer {
    pub room: String,
    pub event: String,
    /// The full-size image, once loaded.
    pub image: Option<Handle>,
    pub failed: bool,
    epoch: u64,
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
        // Dropping the old viewer stops its video and removes its file.
        self.viewer = Some(Viewer {
            room,
            event,
            image: None,
            failed: false,
            epoch,
            #[cfg(target_os = "linux")]
            video: None,
            #[cfg(target_os = "linux")]
            file: None,
        });
        // Arrow keys and Space go to the viewer, not the composer.
        let unfocus = iced::widget::operation::focus("media-viewer");
        let load = match media.kind {
            Kind::Image => self.load_full_image(&media, epoch),
            #[cfg(target_os = "linux")]
            Kind::Video => self.load_video(media, epoch),
            _ => Task::none(),
        };
        Task::batch([unfocus, load])
    }

    fn load_full_image(&mut self, media: &Media, epoch: u64) -> Task<Msg> {
        let Some(source) = media.full() else {
            return Task::none();
        };
        let (tx, rx) = async_channel::bounded(1);
        if self
            .commands
            .try_send(Command::Inline { source, result: tx })
            .is_err()
        {
            return Task::none();
        }
        Task::perform(async move { rx.recv().await.ok().flatten() }, move |img| {
            Msg::ViewerImage(epoch, img)
        })
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
        self.viewer = None;
    }

    pub fn viewer_image(&mut self, epoch: u64, image: Option<Handle>) {
        if let Some(v) = self.viewer.as_mut().filter(|v| v.epoch == epoch) {
            v.failed = image.is_none();
            v.image = image;
        }
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
