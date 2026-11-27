//! Link previews under messages (ROADMAP M7), as Discord shows embeds: the
//! web links in the messages the timeline shows, previewed by the homeserver
//! (engine/previews.rs) a few at a time, newest first, in rooms that show
//! them. Each room's switch syncs with Element's: unencrypted rooms show them
//! unless turned off, encrypted rooms only once turned on, because asking
//! tells the homeserver the link.
use crate::{
    App, Request,
    engine::{Command, LinkPreview},
    rich::{Block, Link},
    rooms::Post,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Links previewed per message.
pub const PER_MESSAGE: usize = 2;
/// Previews asked for at once.
const IN_FLIGHT: usize = 3;
/// Previews kept, the oldest going first.
const KEEP: usize = 300;

#[derive(Default)]
pub struct LinkPreviews {
    /// None: the homeserver had no preview.
    cache: BTreeMap<String, Option<LinkPreview>>,
    order: VecDeque<String>,
    asked: BTreeSet<String>,
}
impl LinkPreviews {
    pub fn get(&self, url: &str) -> Option<&LinkPreview> {
        self.cache.get(url)?.as_ref()
    }
    pub fn arrived(&mut self, url: String, preview: Option<LinkPreview>) {
        self.asked.remove(&url);
        self.order.retain(|u| u != &url);
        self.order.push_back(url.clone());
        self.cache.insert(url, preview);
        while self.order.len() > KEEP {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
            }
        }
    }
}

/// The links in `post` worth a preview, in order: web pages, not Matrix
/// links or mail addresses, nor anything hidden in a spoiler or code.
pub fn links(post: &Post) -> Vec<String> {
    post.rich.as_deref().map(doc_links).unwrap_or_default()
}

fn doc_links(doc: &crate::rich::Doc) -> Vec<String> {
    let mut found = Vec::new();
    collect(&doc.blocks, &mut found);
    found
}

fn collect(blocks: &[Block], found: &mut Vec<String>) {
    for block in blocks {
        if found.len() >= PER_MESSAGE {
            return;
        }
        match block {
            Block::Paragraph(runs) | Block::Heading(_, runs) => {
                for run in runs {
                    let Some(Link::Web(url)) = &run.style.link else {
                        continue;
                    };
                    let page = url.starts_with("https://") || url.starts_with("http://");
                    let matrix = url.starts_with("https://matrix.to/");
                    if page
                        && !matrix
                        && !run.style.code
                        && run.style.spoiler.is_none()
                        && !found.contains(url)
                        && found.len() < PER_MESSAGE
                    {
                        found.push(url.clone());
                    }
                }
            }
            Block::Quote(inner) => collect(inner, found),
            Block::List { items, .. } => items.iter().for_each(|item| collect(item, found)),
            Block::Code { .. } | Block::Rule => {}
        }
    }
}

impl App {
    /// Ask for the previews of the links the open room shows.
    pub fn schedule_link_previews(&mut self) {
        if !self.connected {
            // Answers to questions asked before won't come now.
            self.link_previews.asked.clear();
            return;
        }
        if !self.logged_in || !self.verified || self.account || self.mentions.is_some() {
            return;
        }
        let Some(room) = self
            .live
            .list
            .get(&self.selected)
            .filter(|r| r.link_previews)
        else {
            return;
        };
        let original = self.original.as_ref().filter(|c| c.room == self.selected);
        let history = original.and_then(|c| c.slice.as_ref()).unwrap_or(room);
        let range = self.timeline_range(history);
        let previews = &self.link_previews;
        let mut wanted: Vec<String> = Vec::new();
        for post in history.posts_in(range).rev() {
            for url in links(post) {
                if !previews.cache.contains_key(&url)
                    && !previews.asked.contains(&url)
                    && !wanted.contains(&url)
                {
                    wanted.push(url);
                }
            }
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        for url in wanted {
            if self.link_previews.asked.len() >= IN_FLIGHT {
                break;
            }
            if sdk
                .try_send(Command::LinkPreview { url: url.clone() })
                .is_err()
            {
                break;
            }
            self.link_previews.asked.insert(url);
        }
    }

    /// Turn the room's link previews on or off, for this account.
    pub fn set_link_previews(&mut self, room: String, on: bool) {
        let Some(r) = self.live.list.get_mut(&room) else {
            return;
        };
        if r.link_previews == on {
            return;
        }
        r.link_previews = on;
        if on && r.encrypted {
            self.status = "Link previews are on here: your homeserver fetches the links in this \
                encrypted room."
                .into();
        }
        let target = room.clone();
        self.command(
            |id| Command::SetLinkPreviews {
                id,
                room: target,
                on,
            },
            Request::LinkPreviews(room, on),
        );
    }

    /// A room list from the engine may predate a switch still being saved.
    pub fn keep_link_preview_switches(&mut self) {
        let pending: Vec<(String, bool)> = self
            .pending
            .values()
            .filter_map(|r| match r {
                Request::LinkPreviews(room, on) => Some((room.clone(), *on)),
                _ => None,
            })
            .collect();
        for (room, on) in pending {
            if let Some(r) = self.live.list.get_mut(&room) {
                r.link_previews = on;
            }
        }
    }

    /// Saving the switch failed: show what the room has again.
    pub fn link_previews_failed(&mut self, room: &str, on: bool) {
        if let Some(r) = self.live.list.get_mut(room) {
            r.link_previews = !on;
        }
        self.status = "Couldn't save the link preview setting. Try again.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links_of(body: &str, formatted: Option<&str>) -> Vec<String> {
        crate::rich::Doc::message(formatted, body)
            .map(|d| doc_links(&d))
            .unwrap_or_default()
    }

    #[test]
    fn a_message_previews_its_first_two_web_links() {
        let found = links_of(
            "see https://example.org/a, https://matrix.to/#/!r:x and https://example.org/b and https://example.org/c",
            None,
        );
        assert_eq!(found, ["https://example.org/a", "https://example.org/b"]);
    }

    #[test]
    fn links_in_code_spoilers_and_mail_get_none() {
        let found = links_of(
            "x",
            Some(
                "<code>https://a.example</code> <span data-mx-spoiler>https://b.example</span> \
                  <a href=\"mailto:me@example.org\">mail</a> <pre><code>https://c.example</code></pre>",
            ),
        );
        assert!(found.is_empty(), "{found:?}");
    }
}
