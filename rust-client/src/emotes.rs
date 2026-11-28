//! Custom emoji and stickers (ROADMAP M11) in the App: which image packs
//! serve a room, their pictures, and what picking one does.
//!
//! A room is served by its own packs, those of the spaces above it (as a
//! Discord server's emoji serve its channels), your own pack, and the packs
//! of rooms you use everywhere (`m.image_pack.rooms`). The engine reads
//! them (engine/packs.rs). A custom emoji picked in the composer stands as
//! `:shortcode:` and goes out as a pill (pills.rs): the picture in
//! `formatted_body`, the shortcode in `body` for clients without packs.
use crate::{
    App, Msg, Request,
    backend::Command as Backend,
    engine::{
        Command,
        packs::{Image, Pack, Packs},
    },
    media::{Fit, PreviewRef},
    pills::{Pill, Target},
    preview::Picture,
    reactions::Toggle,
};
use iced::{Task, widget::image::Handle};
use std::collections::{BTreeMap, BTreeSet};

/// Pictures fetched at once.
const INFLIGHT: usize = 6;
/// Pictures kept, besides those still wanted.
const KEEP: usize = 512;
/// A picker shows at most this many of a search's matches.
pub const FOUND: usize = 48;

#[derive(Default)]
pub struct Emotes {
    pub packs: Packs,
    cache: BTreeMap<String, Option<Picture>>,
    inflight: BTreeSet<String>,
    /// The composer's sticker picker is open.
    pub stickers: bool,
}

impl Emotes {
    pub fn get(&self, url: &str) -> Option<&Handle> {
        self.cache.get(url)?.as_ref().map(Picture::still)
    }
    pub fn complete(&mut self, url: String, picture: Option<Picture>) {
        if self.inflight.remove(&url) {
            self.cache.insert(url, picture);
        }
    }
}

#[derive(Clone, Debug)]
pub enum Input {
    /// React to `event` with a custom emoji, or take that reaction back.
    React {
        event: String,
        url: String,
        shortcode: String,
    },
    ToggleStickers,
    CloseStickers,
    Sticker(Image),
    /// Use a room's pack in every room, or stop.
    Everywhere {
        room: String,
        key: String,
        on: bool,
    },
}

/// What a pack request was, for its answer.
#[derive(Clone, Debug)]
pub enum Op {
    Sticker,
    Everywhere,
}

/// How well `query` matches a shortcode: exact, its start, a word's start
/// (after `_` or `-`), anywhere. None if not at all.
fn rank(code: &str, query: &str) -> Option<u8> {
    let code = code.to_lowercase();
    let query = query.trim_matches(':').to_lowercase();
    if query.is_empty() {
        return Some(3);
    }
    if code == query {
        Some(0)
    } else if code.starts_with(&query) {
        Some(1)
    } else if code
        .split(['_', '-'])
        .skip(1)
        .any(|w| w.starts_with(&query))
    {
        Some(2)
    } else {
        code.contains(&query).then_some(3)
    }
}

impl App {
    pub fn set_packs(&mut self, packs: Packs) {
        self.emotes.packs = packs;
    }

    /// The rooms whose packs serve `room`: itself, then the spaces above it,
    /// nearest first.
    fn pack_rooms(&self, room: &str) -> Vec<String> {
        let children = &self.rooms().children;
        let mut out = vec![room.to_owned()];
        let mut i = 0;
        while i < out.len() && out.len() < 16 {
            let below = out[i].clone();
            for (space, kids) in children {
                if kids.contains(&below) && !out.contains(space) {
                    out.push(space.clone());
                }
            }
            i += 1;
        }
        out
    }

    /// The packs usable in `room`, in picker order: its own and its spaces',
    /// yours, then those of rooms you use everywhere. Each once.
    pub fn room_packs(&self, room: &str) -> Vec<&Pack> {
        let packs = &self.emotes.packs;
        let mut out: Vec<&Pack> = Vec::new();
        for r in self.pack_rooms(room) {
            out.extend(
                packs
                    .packs
                    .iter()
                    .filter(|p| p.room.as_deref() == Some(r.as_str())),
            );
        }
        out.extend(packs.packs.iter().filter(|p| p.room.is_none()));
        for p in &packs.packs {
            if let Some(r) = &p.room
                && packs.everywhere.contains(&(r.clone(), p.key.clone()))
                && !out.iter().any(|o| std::ptr::eq(*o, p))
            {
                out.push(p);
            }
        }
        out
    }

    /// The custom emoji usable in `room`, each shortcode once: the first
    /// pack to have it wins.
    pub fn room_emotes(&self, room: &str) -> Vec<(&Pack, &Image)> {
        let mut seen = BTreeSet::new();
        self.room_packs(room)
            .into_iter()
            .flat_map(|p| p.images.iter().filter(|i| i.emoticon).map(move |i| (p, i)))
            .filter(|(_, i)| seen.insert(i.shortcode.as_str()))
            .collect()
    }

    /// The stickers usable in `room`, by pack.
    pub fn room_stickers(&self, room: &str) -> Vec<(&Pack, Vec<&Image>)> {
        self.room_packs(room)
            .into_iter()
            .map(|p| (p, p.images.iter().filter(|i| i.sticker).collect::<Vec<_>>()))
            .filter(|(_, images)| !images.is_empty())
            .collect()
    }

    /// The custom emoji in `room` matching `query`, best first.
    pub fn emote_search(&self, room: &str, query: &str, limit: usize) -> Vec<(&Pack, &Image)> {
        let mut found: Vec<(u8, usize, (&Pack, &Image))> = self
            .room_emotes(room)
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| rank(&e.1.shortcode, query).map(|r| (r, i, e)))
            .collect();
        found.sort_by_key(|(r, i, _)| (*r, *i));
        found.into_iter().take(limit).map(|(_, _, e)| e).collect()
    }

    /// A custom emoji's shortcode, by its mxc URI: what a reaction with it
    /// is called.
    pub fn emote_name(&self, url: &str) -> Option<&str> {
        self.emotes
            .packs
            .packs
            .iter()
            .flat_map(|p| &p.images)
            .find(|i| i.url == url)
            .map(|i| i.shortcode.as_str())
    }

    /// The custom emoji `text` names as `:shortcode:`, as pills, so they go
    /// out as pictures however they were typed.
    pub fn emote_pills(&self, room: &str, text: &str) -> Vec<Pill> {
        let mut out: Vec<Pill> = Vec::new();
        for (_, image) in self.room_emotes(room) {
            let label = format!(":{}:", image.shortcode);
            if text.contains(&label) && !out.iter().any(|p| p.label == label) {
                out.push(Pill {
                    label,
                    target: Target::Emote {
                        url: image.url.clone(),
                    },
                });
            }
        }
        out
    }

    /// Runs inside `update`, so the pictures a picker it opens wants are
    /// fetched in the same pass.
    pub fn emote(&mut self, input: Input) {
        match input {
            Input::React {
                event,
                url,
                shortcode,
            } => {
                let room = self.selected.clone();
                let Some(op) = self
                    .visible_post(&event)
                    .and_then(|p| Toggle::for_post(&room, p, &url))
                else {
                    return;
                };
                self.react_emote(op, shortcode);
            }
            Input::ToggleStickers => {
                self.emotes.stickers = !self.emotes.stickers && self.can_send_sticker();
                if self.emotes.stickers {
                    self.completion.picker = None;
                    self.composer_menu = false;
                }
            }
            Input::CloseStickers => self.emotes.stickers = false,
            Input::Sticker(image) => {
                if !self.can_send_sticker() {
                    return;
                }
                self.emotes.stickers = false;
                let room = self.selected.clone();
                self.command(
                    move |id| Command::SendSticker { id, room, image },
                    Request::Pack(Op::Sticker),
                );
            }
            Input::Everywhere { room, key, on } => {
                if self.verified && !self.pack_pending() {
                    self.command(
                        move |id| Command::UsePack { id, room, key, on },
                        Request::Pack(Op::Everywhere),
                    );
                }
            }
        }
    }

    /// React with a custom emoji, its shortcode beside it; taking one back
    /// goes the usual way.
    pub fn react_emote(&mut self, op: Toggle, shortcode: String) {
        if !op.adds() {
            return self.toggle_reaction(op);
        }
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self.reaction_pending(op.parts().1, op.parts().2)
        {
            return;
        }
        self.reaction_picker = None;
        let (room, event, url) = {
            let (r, e, k) = op.parts();
            (r.to_owned(), e.to_owned(), k.to_owned())
        };
        self.command(
            move |id| Command::ReactEmote {
                id,
                room,
                event,
                url,
                shortcode,
            },
            Request::Reaction(op),
        );
    }

    pub fn pack_done(&mut self, op: Op, ok: bool) {
        if !ok {
            self.status = match op {
                Op::Sticker => "Couldn't send the sticker. Try again.",
                Op::Everywhere => "Couldn't save where you use that pack. Try again.",
            }
            .into();
        }
    }

    pub fn pack_pending(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Pack(Op::Everywhere)))
    }

    pub fn can_send_sticker(&self) -> bool {
        self.logged_in
            && self.verified
            && self.connected
            && self.sdk.is_some()
            && self.live.list.get(&self.selected).is_some_and(|r| {
                !r.space
                    && r.successor.is_none()
                    && r.posting.can_post(&self.matrix_id, r.encrypted) != Some(false)
            })
    }

    /// The pictures the screen shows or is about to: custom emoji in the
    /// visible messages and their reactions, and every one a picker or the
    /// autocomplete popup can show.
    fn wanted_emotes(&self) -> BTreeSet<String> {
        let mut wanted = BTreeSet::new();
        let rooms = self.rooms();
        if let Some(r) = rooms.list.get(&self.selected) {
            let range = self.timeline_range(r);
            for p in r.posts().skip(range.start).take(range.end - range.start) {
                if let Some(doc) = &p.rich {
                    wanted.extend(doc.emotes().into_iter().map(str::to_owned));
                }
                wanted.extend(p.chips.keys().filter(|k| k.starts_with("mxc://")).cloned());
            }
        }
        let picking = self.completion.picker.is_some()
            || self.reaction_picker.is_some()
            || self.completion().is_some();
        if picking {
            wanted.extend(
                self.room_emotes(&self.selected)
                    .into_iter()
                    .take(KEEP / 2)
                    .map(|(_, i)| i.url.clone()),
            );
        }
        if self.emotes.stickers {
            for (_, images) in self.room_stickers(&self.selected) {
                wanted.extend(images.into_iter().map(|i| i.url.clone()));
            }
        }
        wanted
    }

    pub fn schedule_emotes(&mut self) -> Task<Msg> {
        if !self.logged_in || !self.connected || self.emotes.inflight.len() >= INFLIGHT {
            return Task::none();
        }
        let wanted = self.wanted_emotes();
        // Forget pictures nothing wants once there are too many.
        if self.emotes.cache.len() > KEEP {
            let extra = self.emotes.cache.len() - KEEP;
            let old: Vec<String> = self
                .emotes
                .cache
                .keys()
                .filter(|k| !wanted.contains(*k))
                .take(extra)
                .cloned()
                .collect();
            for k in old {
                self.emotes.cache.remove(&k);
            }
        }
        let mut tasks = Vec::new();
        for url in wanted {
            if self.emotes.inflight.len() >= INFLIGHT {
                break;
            }
            if self.emotes.cache.contains_key(&url) || self.emotes.inflight.contains(&url) {
                continue;
            }
            let (tx, rx) = async_channel::bounded(1);
            let source = PreviewRef {
                mxc: url.clone(),
                encrypted: false,
                fit: Fit::Emote,
            };
            if self
                .commands
                .try_send(Backend::Inline { source, result: tx })
                .is_err()
            {
                break;
            }
            self.emotes.inflight.insert(url.clone());
            tasks.push(Task::perform(
                async move { rx.recv().await.ok().flatten() },
                move |picture| Msg::EmoteReady(url.clone(), picture),
            ));
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, RoomList, RoomSummary};

    fn image(code: &str, emoticon: bool, sticker: bool) -> Image {
        Image {
            shortcode: code.into(),
            url: format!("mxc://h/{code}"),
            body: None,
            info: None,
            emoticon,
            sticker,
        }
    }

    fn pack(room: Option<&str>, key: &str, images: Vec<Image>) -> Pack {
        Pack {
            room: room.map(str::to_owned),
            key: key.into(),
            stable: true,
            name: format!("{room:?}"),
            avatar: None,
            attribution: None,
            images,
        }
    }

    /// The workshop space holds #general; #elsewhere is outside it.
    fn app() -> App {
        let (gomuks, _) = async_channel::bounded(1);
        let mut app = App::new(gomuks);
        app.logged_in = true;
        let room = |id: &str, space: bool| RoomSummary {
            id: id.into(),
            name: id.into(),
            space,
            ..Default::default()
        };
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                room("!workshop:h", true),
                room("!general:h", false),
                room("!elsewhere:h", false),
                room("!lounge:h", false),
            ],
            children: [(
                "!workshop:h".to_owned(),
                BTreeSet::from(["!general:h".to_owned()]),
            )]
            .into(),
        }));
        app.set_packs(Packs {
            packs: vec![
                pack(
                    Some("!general:h"),
                    "",
                    vec![image("wave", true, false), image("big", false, true)],
                ),
                pack(
                    Some("!workshop:h"),
                    "",
                    vec![image("wave", true, false), image("hammer", true, true)],
                ),
                pack(None, "", vec![image("me", true, false)]),
                pack(Some("!lounge:h"), "cats", vec![image("cat", true, false)]),
            ],
            everywhere: BTreeSet::from([("!lounge:h".to_owned(), "cats".to_owned())]),
        });
        app
    }

    #[test]
    fn a_room_uses_its_own_its_spaces_yours_and_everywhere_packs() {
        let app = app();
        let codes = |room: &str| -> Vec<String> {
            app.room_emotes(room)
                .into_iter()
                .map(|(_, i)| i.shortcode.clone())
                .collect()
        };
        // The room's own "wave" wins over its space's.
        assert_eq!(codes("!general:h"), ["wave", "hammer", "me", "cat"]);
        assert_eq!(
            app.room_emotes("!general:h")[0].1.url,
            "mxc://h/wave",
            "from #general's pack"
        );
        // Outside the space: yours and the ones used everywhere.
        assert_eq!(codes("!elsewhere:h"), ["me", "cat"]);
        let stickers: Vec<_> = app
            .room_stickers("!general:h")
            .into_iter()
            .flat_map(|(_, i)| i.into_iter().map(|i| i.shortcode.clone()))
            .collect();
        assert_eq!(stickers, ["big", "hammer"]);
    }

    #[test]
    fn typed_shortcodes_of_usable_emoji_become_pills() {
        let app = app();
        let pills = app.emote_pills("!elsewhere:h", "hi :cat: :hammer: :cat:");
        assert_eq!(pills.len(), 1, "once each, and only usable ones");
        assert_eq!(pills[0].label, ":cat:");
        assert_eq!(app.emote_name("mxc://h/hammer"), Some("hammer"));
        let found: Vec<_> = app
            .emote_search("!general:h", "am", 5)
            .into_iter()
            .map(|(_, i)| i.shortcode.clone())
            .collect();
        assert_eq!(found, ["hammer"]);
    }

    #[test]
    fn shortcodes_rank_exact_then_prefix_then_word_then_anywhere() {
        assert_eq!(rank("blobcat", "blobcat"), Some(0));
        assert_eq!(rank("blobcat", ":blob"), Some(1));
        assert_eq!(rank("party_blob", "blob"), Some(2));
        assert_eq!(rank("bigblob", "blob"), Some(3));
        assert_eq!(rank("cat", "dog"), None);
    }
}
