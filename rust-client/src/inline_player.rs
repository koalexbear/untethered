//! Playing in the timeline (ROADMAP P2), as Discord does: a video plays where
//! it is, and an audio file or voice message gets a player with seek. One
//! plays at a time; opening the viewer, another room, or another file stops
//! it. The file is fetched (decrypted) into the private runtime directory and
//! played through GStreamer (player.rs); Linux only, like the viewer's video.
use crate::{
    App, Msg,
    backend::Command,
    media::Media,
    player::{Control, Event, Player},
};
use iced::Task;

pub struct InlinePlayer {
    pub room: String,
    pub event: String,
    pub player: Option<Player>,
    pub failed: bool,
    epoch: u64,
    file: Option<tempfile::TempPath>,
}

impl App {
    /// Play `media` (the message `event`'s) in place, or pause and resume it.
    pub fn play_inline(&mut self, event: String, media: Media) -> Task<Msg> {
        if let Some(p) = self
            .inline_player
            .as_mut()
            .filter(|p| p.event == event && p.room == self.selected)
        {
            if let Some(player) = &mut p.player {
                player.control(Control::Toggle);
            }
            return Task::none();
        }
        crate::player::warm_up();
        let epoch = self.inline_player.as_ref().map_or(0, |p| p.epoch) + 1;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let file = tempfile::Builder::new()
            .prefix("untethered-play-")
            .tempfile_in(dir)
            .ok()
            .map(tempfile::NamedTempFile::into_temp_path);
        let destination = file.as_ref().map(|f| f.to_path_buf());
        // Dropping the old one stops it and removes its file.
        self.inline_player = Some(InlinePlayer {
            room: self.selected.clone(),
            event,
            player: None,
            failed: destination.is_none(),
            epoch,
            file,
        });
        let Some(destination) = destination else {
            return Task::none();
        };
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
            return Task::done(Msg::InlineFetched(epoch, false));
        }
        Task::perform(async move { rx.recv().await.unwrap_or(false) }, move |ok| {
            Msg::InlineFetched(epoch, ok)
        })
    }

    pub fn inline_fetched(&mut self, epoch: u64, ok: bool) {
        let Some(p) = self.inline_player.as_mut().filter(|p| p.epoch == epoch) else {
            return;
        };
        match p.file.as_ref().filter(|_| ok).map(|f| Player::open(f)) {
            Some(Ok(player)) => p.player = Some(player),
            Some(Err(reason)) => {
                eprintln!("Inline player: {reason}");
                p.failed = true;
            }
            None => p.failed = true,
        }
    }

    pub fn inline_event(&mut self, event: Event) {
        let Some(p) = &mut self.inline_player else {
            return;
        };
        let Some(player) = &mut p.player else {
            return;
        };
        match event {
            Event::Frame(frame) => {
                player.frame = Some(frame);
                player.poll();
            }
            Event::Ended => player.finished(),
            Event::Failed(reason) => {
                eprintln!("Inline player: {reason}");
                p.player = None;
                p.failed = true;
            }
        }
    }

    pub fn inline_control(&mut self, control: Control) {
        if let Some(player) = self.inline_player.as_mut().and_then(|p| p.player.as_mut()) {
            player.control(control);
        }
    }

    pub fn inline_scrub(&mut self, at: f32) {
        if let Some(player) = self.inline_player.as_mut().and_then(|p| p.player.as_mut()) {
            player.scrub = Some(at);
        }
    }

    /// While something plays, its position moves on; audio has no frames
    /// to read it from.
    pub fn inline_tick(&mut self) {
        if let Some(player) = self.inline_player.as_mut().and_then(|p| p.player.as_mut()) {
            player.poll();
        }
    }

    /// The player for the message `event` in the open room, if it's that one.
    pub fn inline_playing(&self, event: Option<&str>) -> Option<&InlinePlayer> {
        self.inline_player
            .as_ref()
            .filter(|p| p.room == self.selected && Some(p.event.as_str()) == event)
    }

    /// Whether a tick should move the shown position on.
    pub fn inline_running(&self) -> bool {
        self.inline_player
            .as_ref()
            .and_then(|p| p.player.as_ref())
            .is_some_and(|p| p.playing)
    }

    pub fn stop_inline(&mut self) {
        self.inline_player = None;
    }
}
