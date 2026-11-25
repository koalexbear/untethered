//! Rebuild from this device's decrypted gomuks cache. Desktop notification
//! decisions (including user-defined highlight words) are not mention data.
use crate::{
    App, Request, Section, edits,
    rooms::{self, Room, items},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Key = (String, String); // Room and canonical message event, never an edit ID.
#[derive(Clone, Copy)]
pub enum Page {
    Candidates,
    Bases,
}
pub struct Hit {
    pub room: String,
    pub event: String,
    pub sender: String,
    pub preview: String,
    pub time: i64,
}
#[derive(Default)]
struct Scan {
    rooms: Vec<String>,
    hits: BTreeMap<Key, Hit>,
    unresolved: BTreeMap<Key, BTreeSet<String>>,
    base_rooms: BTreeSet<String>,
    senders: BTreeSet<String>,
    dirty: bool,
}
pub struct Inbox {
    pub hits: Vec<Hit>,
    pub shown: usize,
    pub failed: bool,
    scan: Option<Scan>,
}
/// The same excerpt as `Post::quote`.
#[cfg(not(target_arch = "wasm32"))]
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
fn addressed(mentions: &Value, me: &str) -> bool {
    !me.is_empty() && items(&mentions["user_ids"]).any(|id| id.as_str() == Some(me))
}
fn content(e: &Value) -> &Value {
    if e["type"] == "m.room.encrypted" {
        &e["decrypted"]
    } else {
        &e["content"]
    }
}
impl Scan {
    fn params(&self, page: Page, next: Option<&str>) -> Value {
        let mut q = json!({"sort_by_time":true,"limit":128});
        match page {
            Page::Candidates => {
                // get_mentions filters push-rule highlights, which can include
                // keywords or omit a direct mention if the user muted it.
                // A coarse JSON key prefilter is never the test for a mention.
                // ESCAPE '\' is part of gomuks's parameterized LIKE query.
                q["raw_like"] = r#""user\_ids""#.into();
                q["room_ids"] = json!(self.rooms);
            }
            Page::Bases => {
                q["room_ids"] = json!(self.base_rooms);
                q["senders"] = json!(self.senders);
            }
        }
        if let Some(t) = next {
            q["next_batch"] = t.into();
        }
        q
    }
    fn accept<'a>(
        &mut self,
        page: Page,
        data: &'a Value,
        me: &str,
    ) -> Option<(Page, Option<&'a str>)> {
        let mut rooms = BTreeMap::<&str, Room>::new();
        for e in items(&data["events"]) {
            let (Some(r), Some(id), Some(sender)) = (
                e["room_id"].as_str(),
                e["event_id"].as_str(),
                e["sender"].as_str(),
            ) else {
                continue;
            };
            if self.rooms.binary_search_by(|s| s.as_str().cmp(r)).is_err()
                || sender == me
                || !rooms::server_confirmed(e)
            {
                continue;
            }
            if matches!(page, Page::Bases) && !self.unresolved.contains_key(&(r.into(), id.into()))
            {
                continue;
            }
            if edits::is_replacement(e) && matches!(page, Page::Candidates) {
                let c = content(e);
                if addressed(&c["m.new_content"]["m.mentions"], me)
                    && let Some(base) = c["m.relates_to"]["event_id"]
                        .as_str()
                        .filter(|s| s.starts_with('$'))
                {
                    self.unresolved
                        .entry((r.into(), base.into()))
                        .or_default()
                        .insert(sender.into());
                }
            }
            if !edits::is_replacement(e) {
                rooms.entry(r).or_default().insert(e);
            }
        }
        // A latest edit can be in the query page itself instead of
        // related_events, including when looking up a previously unseen base.
        for e in items(&data["events"])
            .chain(items(&data["related_events"]))
            .filter(|e| edits::is_replacement(e))
        {
            if let Some(r) = e["room_id"].as_str().and_then(|r| rooms.get_mut(r)) {
                r.insert(e);
            }
        }
        for (room, r) in rooms {
            for p in r.history.values() {
                let Some(id) = &p.event_id else { continue };
                let key = (room.into(), id.clone());
                self.unresolved.remove(&key);
                if p.reactable
                    && p.sender != me
                    && addressed(p.current_mentions(), me)
                    && let Some(q) = p.quote()
                {
                    self.hits.insert(
                        key,
                        Hit {
                            room: room.into(),
                            event: id.clone(),
                            sender: q.sender,
                            preview: q.excerpt,
                            time: p.time,
                        },
                    );
                }
            }
        }
        let more = data["next_batch"].as_str().filter(|t| !t.is_empty());
        if let Some(t) = more
            && (matches!(page, Page::Candidates) || !self.unresolved.is_empty())
        {
            return Some((page, Some(t)));
        }
        if matches!(page, Page::Candidates) {
            self.unresolved.retain(|k, _| !self.hits.contains_key(k));
            if !self.unresolved.is_empty() {
                self.base_rooms = self.unresolved.keys().map(|(r, _)| r.clone()).collect();
                self.senders = self.unresolved.values().flatten().cloned().collect();
                return Some((Page::Bases, None));
            }
        }
        None
    }
}
impl Inbox {
    pub fn loading(&self) -> bool {
        self.scan.is_some()
    }
}
impl App {
    pub fn close_mentions(&mut self) {
        self.mentions = None;
        self.pending
            .retain(|_, p| !matches!(p, Request::Mentions(_)));
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
    pub fn mention_delta(&self, d: &Value) -> bool {
        items(&d["events"]).any(|e| {
            let c = content(e);
            addressed(&c["m.mentions"], &self.matrix_id)
                || edits::is_replacement(e)
                || e["type"] == "m.room.redaction"
                || e["redacted_by"].is_string()
        })
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
        self.request_mentions(Page::Candidates, None);
    }
    fn request_mentions(&mut self, page: Page, next: Option<&str>) {
        // The matrix-sdk engine answers with every hit at once.
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
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
                Request::Mentions(page),
            );
            if !self.pending.contains_key(&self.seq) {
                self.mentions_failed();
            }
            return;
        }
        if let Some(s) = self.mentions.as_ref().and_then(|i| i.scan.as_ref()) {
            self.rpc(
                "search_local",
                s.params(page, next),
                Request::Mentions(page),
            );
            if !self.pending.contains_key(&self.seq) {
                self.mentions_failed();
            }
        }
    }
    pub fn mentions_failed(&mut self) {
        if let Some(i) = &mut self.mentions {
            i.scan = None;
            i.failed = true;
        }
    }
    /// Messages mentioning us, from the matrix-sdk engine.
    #[cfg(not(target_arch = "wasm32"))]
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
    pub fn mentions_page(&mut self, page: Page, data: &Value) {
        let Some(i) = &mut self.mentions else { return };
        let Some(s) = &mut i.scan else { return };
        if !data.is_object() || (!data["events"].is_null() && !data["events"].is_array()) {
            self.mentions_failed();
            return;
        }
        if let Some((kind, t)) = s.accept(page, data, &self.matrix_id) {
            self.request_mentions(kind, t);
            return;
        }
        let scan = i.scan.take().unwrap();
        i.hits = scan
            .hits
            .into_values()
            .filter(|h| self.live.list.contains_key(&h.room))
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
