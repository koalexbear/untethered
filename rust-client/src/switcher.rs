//! The quick switcher (ROADMAP N1): Ctrl+K, a few letters and Enter opens a
//! room, DM or space without the mouse, as on Discord. Unread rooms come
//! first, mentions before them; a leading `#`, `!`, `@` or `*` narrows the
//! list to text channels, voice channels, DMs or spaces.
use crate::{App, Msg, Section};
use iced::{Task, widget::operation};

pub const SEARCH: &str = "quick-switcher";
/// How many rooms the list shows.
pub const LIMIT: usize = 12;

#[derive(Default)]
pub struct Switcher {
    pub query: String,
    /// The highlighted row, which Enter opens.
    pub chosen: usize,
}

#[derive(Clone, Debug)]
pub enum Input {
    /// Ctrl+K opens it, and closes it again.
    Toggle,
    Close,
    Query(String),
    /// The arrow keys move the highlight.
    Step(i32),
    /// Enter opens the highlighted row; a click opens that room.
    Go(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Channel,
    Voice,
    Dm,
    Space,
}

/// One row: a room the query matches.
#[derive(Debug)]
pub struct Hit<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: Kind,
    /// The space a channel is in.
    pub place: Option<&'a str>,
    pub unread: bool,
    pub mentions: u64,
}

/// What a leading `#`, `!`, `@` or `*` narrows to, and the rest.
pub fn narrow(query: &str) -> (Option<Kind>, &str) {
    let query = query.trim();
    let kind = match query.chars().next() {
        Some('#') => Kind::Channel,
        Some('!') => Kind::Voice,
        Some('@') => Kind::Dm,
        Some('*') => Kind::Space,
        _ => return (None, query),
    };
    (Some(kind), query[1..].trim_start())
}

/// How well `name` matches `query`, lower being better: the start of the
/// name, the start of a word in it, anywhere in it, or its letters in order.
pub fn score(name: &str, query: &str) -> Option<u8> {
    let name = name.to_lowercase();
    let query = query.to_lowercase();
    if name.starts_with(&query) {
        return Some(0);
    }
    if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(&query))
    {
        return Some(1);
    }
    if name.contains(&query) {
        return Some(2);
    }
    let mut letters = name.chars();
    query.chars().all(|q| letters.any(|n| n == q)).then_some(3)
}

impl App {
    /// The rows for what the switcher says now, best first.
    pub fn switcher_hits(&self) -> Vec<Hit<'_>> {
        let Some(switcher) = &self.switcher else {
            return Vec::new();
        };
        let (only, query) = narrow(&switcher.query);
        let rooms = self.rooms();
        let mut hits: Vec<(u8, Hit)> = rooms
            .list
            .iter()
            .filter_map(|(id, r)| {
                let kind = if r.space {
                    Kind::Space
                } else if r.dm {
                    Kind::Dm
                } else if r.voice {
                    Kind::Voice
                } else {
                    Kind::Channel
                };
                if only.is_some_and(|k| k != kind) {
                    return None;
                }
                let score = score(&r.name, query)?;
                let place = matches!(kind, Kind::Channel | Kind::Voice)
                    .then(|| self.home_space(id))
                    .flatten()
                    .and_then(|space| rooms.list.get(space))
                    .map(|space| space.name.as_str());
                Some((
                    score,
                    Hit {
                        id,
                        name: &r.name,
                        kind,
                        place,
                        unread: r.unread,
                        mentions: r.read.highlights,
                    },
                ))
            })
            .collect();
        hits.sort_by_cached_key(|(score, hit)| {
            (
                *score,
                hit.mentions == 0,
                !hit.unread,
                hit.name.len(),
                hit.name.to_lowercase(),
            )
        });
        hits.into_iter().take(LIMIT).map(|(_, hit)| hit).collect()
    }

    /// The first space whose channel list has `room`.
    pub(crate) fn home_space(&self, room: &str) -> Option<&str> {
        self.rooms()
            .ordered_spaces
            .iter()
            .find(|(_, rooms)| rooms.iter().any(|id| id == room))
            .map(|(space, _)| space.as_str())
    }

    pub fn switcher(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Toggle if self.switcher.is_some() => return self.close_switcher(),
            Input::Toggle if self.logged_in && self.verified => {
                self.switcher = Some(Switcher::default());
                self.server_menu = false;
                self.account = false;
                self.context_menu.dismiss();
                self.sdk_ui.profile.dismiss_card();
                self.reaction_picker = None;
                if self.viewer.is_some() {
                    self.close_viewer();
                }
                if !self.starting_dm() {
                    self.new_message = None;
                }
                if !self.forward_sending() {
                    self.forwarding = None;
                }
                if !self.poll_sending() {
                    self.polling = None;
                }
                self.composer_menu = false;
                return operation::focus(SEARCH);
            }
            Input::Close => return self.close_switcher(),
            Input::Query(query) => {
                if let Some(s) = &mut self.switcher {
                    s.query = query;
                    s.chosen = 0;
                }
            }
            Input::Step(step) => {
                let rows = self.switcher_hits().len() as i32;
                if let Some(s) = self.switcher.as_mut().filter(|_| rows > 0) {
                    s.chosen = (s.chosen as i32 + step).rem_euclid(rows) as usize;
                }
            }
            Input::Go(id) => {
                let id = id.or_else(|| {
                    let chosen = self.switcher.as_ref()?.chosen;
                    self.switcher_hits().get(chosen).map(|h| h.id.to_owned())
                });
                if let Some(id) = id {
                    self.switcher = None;
                    let open = self.go_to(id);
                    return Task::batch([open, operation::focus("composer")]);
                }
            }
            Input::Toggle => {}
        }
        Task::none()
    }

    fn close_switcher(&mut self) -> Task<Msg> {
        self.switcher = None;
        operation::focus("composer")
    }

    /// Open a room where the sidebar shows it, as clicking it would; a
    /// space opens as its server.
    fn go_to(&mut self, id: String) -> Task<Msg> {
        let Some(r) = self.rooms().list.get(&id) else {
            return Task::none();
        };
        if r.space {
            return self.update(Msg::Section(Section::Space(id)));
        }
        let (dm, voice) = (r.dm, r.voice);
        if !self.visible().any(|(shown, _)| *shown == id) {
            let section = if dm {
                Section::Direct
            } else {
                self.home_space(&id)
                    .map_or(Section::All, |space| Section::Space(space.to_owned()))
            };
            self.close_mentions();
            self.invite_open = None;
            self.room_preview = None;
            self.section = section;
        }
        let open = self.update(if voice {
            Msg::OpenVoice(id)
        } else {
            Msg::Open(id)
        });
        self.reconcile();
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, RoomList, RoomSummary};

    #[test]
    fn names_match_best_at_their_start() {
        assert_eq!(score("general", "gen"), Some(0));
        assert_eq!(score("General", "GEN"), Some(0));
        assert_eq!(score("off-topic", "top"), Some(1));
        assert_eq!(score("Mira and Milo", "milo"), Some(1));
        assert_eq!(score("design", "sig"), Some(2));
        assert_eq!(score("announcements", "anc"), Some(3));
        assert_eq!(score("general", "xyz"), None);
        assert_eq!(score("general", ""), Some(0));
    }

    #[test]
    fn a_leading_sign_narrows_the_kind() {
        assert_eq!(narrow("#gen"), (Some(Kind::Channel), "gen"));
        assert_eq!(narrow(" ! lounge"), (Some(Kind::Voice), "lounge"));
        assert_eq!(narrow("@mira"), (Some(Kind::Dm), "mira"));
        assert_eq!(narrow("*"), (Some(Kind::Space), ""));
        assert_eq!(narrow("gen"), (None, "gen"));
    }

    fn app() -> App {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, _rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        let room = |id: &str, name: &str| RoomSummary {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let mut children = std::collections::BTreeMap::new();
        children.insert(
            "!space:h".to_owned(),
            std::collections::BTreeSet::from(["!design:h".to_owned()]),
        );
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![
                RoomSummary {
                    unread_mentions: 1,
                    ..room("!general:h", "general")
                },
                room("!gaming:h", "gaming"),
                RoomSummary {
                    unread_messages: 2,
                    ..room("!games:h", "board games")
                },
                room("!design:h", "design"),
                RoomSummary {
                    space: true,
                    ..room("!space:h", "Workshop")
                },
                RoomSummary {
                    dm: true,
                    dm_peer: Some("@mira:h".into()),
                    ..room("!dm:h", "Mira")
                },
            ],
            children,
        }));
        app
    }

    fn ids(app: &App) -> Vec<String> {
        app.switcher_hits()
            .into_iter()
            .map(|h| h.id.to_owned())
            .collect()
    }

    #[test]
    fn the_best_match_comes_first_and_unread_rooms_before_read_ones() {
        let mut app = app();
        let _ = app.update(Msg::Switcher(Input::Toggle));
        // With nothing typed, mentions lead, then unread rooms.
        assert_eq!(ids(&app)[..2], ["!general:h", "!games:h"]);
        // The start of a name, of a word in it, then its letters in order.
        let _ = app.update(Msg::Switcher(Input::Query("ga".into())));
        assert_eq!(ids(&app), ["!gaming:h", "!games:h", "!general:h"]);
        // Equally good matches: the one with a mention first.
        let _ = app.update(Msg::Switcher(Input::Query("g".into())));
        assert_eq!(ids(&app)[..3], ["!general:h", "!gaming:h", "!games:h"]);
        let _ = app.update(Msg::Switcher(Input::Query("*".into())));
        assert_eq!(ids(&app), ["!space:h"]);
        let _ = app.update(Msg::Switcher(Input::Query("des".into())));
        assert_eq!(app.switcher_hits()[0].place, Some("Workshop"));
    }

    #[test]
    fn three_letters_and_enter_open_a_room_where_the_sidebar_has_it() {
        let mut app = app();
        app.section = Section::Direct;
        let _ = app.update(Msg::Switcher(Input::Toggle));
        let _ = app.update(Msg::Switcher(Input::Query("des".into())));
        let _ = app.update(Msg::Switcher(Input::Go(None)));
        assert!(app.switcher.is_none());
        assert_eq!(app.selected, "!design:h");
        assert!(app.section == Section::Space("!space:h".into()));
        // Arrows move the highlight, wrapping around.
        let _ = app.update(Msg::Switcher(Input::Toggle));
        let _ = app.update(Msg::Switcher(Input::Query("mira".into())));
        let _ = app.update(Msg::Switcher(Input::Step(-1)));
        assert_eq!(app.switcher.as_ref().unwrap().chosen, 0);
        let _ = app.update(Msg::Switcher(Input::Go(None)));
        assert_eq!(app.selected, "!dm:h");
        assert!(app.section == Section::Direct);
        // Ctrl+K closes it again; Escape does too.
        let _ = app.update(Msg::Switcher(Input::Toggle));
        let _ = app.update(Msg::Switcher(Input::Toggle));
        assert!(app.switcher.is_none());
        let _ = app.update(Msg::Switcher(Input::Toggle));
        let _ = app.update(Msg::Escape);
        assert!(app.switcher.is_none());
    }
}
