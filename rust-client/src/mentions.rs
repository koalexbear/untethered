//! The mentions inbox: messages from others that mention us, in every joined
//! room, from the engine's stored events. Desktop notification decisions
//! (including user-defined highlight words) are not mention data.
use crate::{App, Request, Section};

pub struct Hit {
    pub room: String,
    pub event: String,
    pub sender: String,
    pub preview: String,
    pub time: i64,
}
/// A request the engine is answering; `dirty` asks for another when it ends.
#[derive(Default)]
struct Scan {
    rooms: Vec<String>,
    dirty: bool,
}
pub struct Inbox {
    pub hits: Vec<Hit>,
    pub shown: usize,
    pub failed: bool,
    scan: Option<Scan>,
}
/// The same excerpt as `Post::quote`.
fn preview(text: &str) -> String {
    let mut chars = text.trim().chars();
    let mut excerpt: String = chars
        .by_ref()
        .take(72)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        excerpt.push('…');
    }
    excerpt
}
impl Inbox {
    pub fn loading(&self) -> bool {
        self.scan.is_some()
    }
}
impl App {
    /// Leave the mentions inbox, and Explore (explore.rs): every way to a
    /// room, a space or another page calls this.
    pub fn close_mentions(&mut self) {
        self.mentions = None;
        self.explore.page = None;
        self.pending.retain(|_, p| !matches!(p, Request::Mentions));
    }
    pub fn open_mentions(&mut self) {
        if !self.logged_in || !self.verified || !self.connected {
            return;
        }
        self.close_original();
        self.clear_search();
        self.mentions.get_or_insert(Inbox {
            hits: vec![],
            shown: 50,
            failed: false,
            scan: None,
        });
        self.refresh_mentions();
    }
    pub fn refresh_mentions(&mut self) {
        let Some(i) = &mut self.mentions else { return };
        i.hits.retain(|h| self.live.list.contains_key(&h.room));
        if let Some(s) = &mut i.scan {
            s.dirty = true;
            return;
        }
        i.failed = false;
        let rooms: Vec<_> = self
            .live
            .list
            .iter()
            .filter(|(_, r)| !r.space)
            .map(|(k, _)| k.clone())
            .collect();
        if rooms.is_empty() {
            i.hits.clear();
            return;
        }
        i.scan = Some(Scan {
            rooms,
            ..Default::default()
        });
        self.request_mentions();
    }
    fn request_mentions(&mut self) {
        let Some(rooms) = self
            .mentions
            .as_ref()
            .and_then(|i| i.scan.as_ref())
            .map(|s| s.rooms.clone())
        else {
            return;
        };
        self.command(
            |id| crate::engine::Command::Mentions { id, rooms },
            Request::Mentions,
        );
        if !self.pending.contains_key(&self.seq) {
            self.mentions_failed();
        }
    }
    pub fn mentions_failed(&mut self) {
        if let Some(i) = &mut self.mentions {
            i.scan = None;
            i.failed = true;
        }
    }
    /// Messages mentioning us, from the engine.
    pub fn mentions_found(&mut self, found: Vec<crate::engine::Found>) {
        let Some(i) = &mut self.mentions else { return };
        let Some(scan) = i.scan.take() else { return };
        let joined = &self.live.list;
        i.hits = found
            .into_iter()
            .filter(|f| joined.get(&f.room).is_some_and(|r| !r.space))
            .map(|f| Hit {
                preview: preview(&f.body),
                room: f.room,
                event: f.event_id,
                sender: f.sender,
                time: f.time,
            })
            .collect();
        i.hits.sort_by(|a, b| {
            b.time
                .cmp(&a.time)
                .then(a.room.cmp(&b.room))
                .then(a.event.cmp(&b.event))
        });
        if scan.dirty {
            self.refresh_mentions();
        }
    }
    pub fn open_mention(&mut self, room: String, id: String) -> bool {
        if !self.logged_in
            || !self.verified
            || !self.live.list.contains_key(&room)
            || !self
                .mentions
                .as_ref()
                .is_some_and(|i| i.hits.iter().any(|h| h.room == room && h.event == id))
        {
            return false;
        }
        self.close_mentions();
        self.clear_search();
        self.section = Section::All;
        self.open(room);
        self.jump(id);
        true
    }
}
