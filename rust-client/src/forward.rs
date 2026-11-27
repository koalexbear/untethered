//! Forward (ROADMAP M8), as on Discord: Forward in a message's toolbar or
//! right-click menu opens Forward To, a search of your channels and DMs.
//! Pick up to five, add a message if you like, and Send.
//!
//! Matrix has no forwarded marker, so each arrives as an ordinary message
//! from you, as Element forwards: the engine sends the content again
//! (engine/timeline.rs), and an attachment keeps its file and key, so
//! nothing is uploaded again. Nobody it mentions is notified again.
use crate::{App, Msg, Request, engine::Command, rooms::Post};
use iced::{Task, widget::operation};

pub const SEARCH: &str = "forward-search";
/// As many places as Discord forwards to at once.
pub const MAX_ROOMS: usize = 5;
/// How many rooms the list shows.
const LIMIT: usize = 50;

pub struct Form {
    /// The room the message shows in, and the message.
    pub room: String,
    pub event: String,
    pub query: String,
    /// The rooms picked, in the order picked.
    pub picked: Vec<String>,
    /// An optional message, sent after the forwarded one.
    pub note: String,
    /// What went wrong last: too many picked, or rooms it couldn't reach.
    pub problem: Option<String>,
    /// Where it went while a send is answered, for the status line.
    sent: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Input {
    Open {
        room: String,
        event: String,
    },
    Close,
    Query(String),
    Toggle(String),
    /// Enter in the search: pick the best match, or with nothing typed,
    /// send.
    Enter,
    Note(String),
    Send,
}

/// A room the list offers.
pub struct Target<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub dm: bool,
    /// The space a channel is in.
    pub place: Option<&'a str>,
}

/// "#general", "#general and Mira", "#a, #b and Mira".
pub fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl App {
    pub fn forward(&mut self, input: Input) -> Task<Msg> {
        let busy = self.forward_sending();
        match input {
            Input::Open { room, event } if self.logged_in && self.verified && !busy => {
                self.forwarding = Some(Form {
                    room,
                    event,
                    query: String::new(),
                    picked: Vec::new(),
                    note: String::new(),
                    problem: None,
                    sent: Vec::new(),
                });
                self.server_menu = false;
                self.context_menu.dismiss();
                self.sdk_ui.profile.dismiss_card();
                self.reaction_picker = None;
                return operation::focus(SEARCH);
            }
            Input::Close if !busy => self.forwarding = None,
            Input::Query(query) => {
                if let Some(form) = &mut self.forwarding {
                    form.query = query;
                }
            }
            Input::Toggle(room) if !busy => {
                self.toggle_target(room);
                return operation::focus(SEARCH);
            }
            Input::Enter if !busy => {
                let typed = self
                    .forwarding
                    .as_ref()
                    .is_some_and(|f| !f.query.trim().is_empty());
                if !typed {
                    self.send_forward();
                } else if let Some(first) = self.forward_targets().first().map(|t| t.id.to_owned())
                {
                    self.toggle_target(first);
                    if let Some(form) = &mut self.forwarding {
                        form.query.clear();
                    }
                    // A new chip moves the search box along its row, which
                    // loses its focus (Iced keeps state by position).
                    return operation::focus(SEARCH);
                }
            }
            Input::Note(note) if !busy => {
                if let Some(form) = &mut self.forwarding {
                    form.note = note;
                }
            }
            Input::Send => self.send_forward(),
            _ => {}
        }
        Task::none()
    }

    fn toggle_target(&mut self, room: String) {
        let Some(form) = &mut self.forwarding else {
            return;
        };
        if let Some(at) = form.picked.iter().position(|p| *p == room) {
            form.picked.remove(at);
            form.problem = None;
        } else if form.picked.len() < MAX_ROOMS {
            form.picked.push(room);
            form.problem = None;
        } else {
            form.problem = Some(format!("You can forward to {MAX_ROOMS} places at once."));
        }
    }

    fn send_forward(&mut self) {
        if !self.connected || self.forward_sending() {
            return;
        }
        let Some(form) = &mut self.forwarding else {
            return;
        };
        if form.picked.is_empty() {
            return;
        }
        form.problem = None;
        form.sent.clear();
        let note = Some(form.note.trim().to_owned()).filter(|n| !n.is_empty());
        let (room, event, picked) = (form.room.clone(), form.event.clone(), form.picked.clone());
        for to in picked {
            let (room, event, note, target) =
                (room.clone(), event.clone(), note.clone(), to.clone());
            self.command(
                move |id| Command::Forward {
                    id,
                    room,
                    event,
                    to: target,
                    note,
                },
                Request::Forward(to),
            );
        }
    }

    pub fn forward_sending(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Forward(_)))
    }

    /// The engine's answer for one room. Once all are in, the dialog closes,
    /// or stays with the rooms it couldn't reach still picked, to try again.
    pub fn forwarded(&mut self, to: String, ok: bool) {
        let name = self.place_name(&to);
        let busy = self.forward_sending();
        let Some(form) = &mut self.forwarding else {
            if !ok {
                self.status = format!("Couldn't forward to {name}. Try again.");
            }
            return;
        };
        if ok {
            form.picked.retain(|p| *p != to);
            form.sent.push(name);
        }
        if busy {
            return;
        }
        if !form.sent.is_empty() {
            self.status = format!("Forwarded to {}.", listed(&form.sent));
        }
        if form.picked.is_empty() {
            self.forwarding = None;
            return;
        }
        let failed: Vec<String> = form.picked.clone();
        let failed: Vec<String> = failed.iter().map(|id| self.place_name(id)).collect();
        if let Some(form) = &mut self.forwarding {
            form.problem = Some(format!(
                "Couldn't forward to {}. Send tries again.",
                listed(&failed)
            ));
        }
    }

    /// A room as the status line names it: "#general", or a DM's name.
    pub fn place_name(&self, room: &str) -> String {
        match self.rooms().list.get(room) {
            Some(r) if r.dm => r.name.clone(),
            Some(r) => format!("#{}", r.name),
            None => room.to_owned(),
        }
    }

    /// The message being forwarded.
    pub fn forward_post(&self) -> Option<&Post> {
        let form = self.forwarding.as_ref()?;
        self.rooms().list.get(&form.room)?.by_id(&form.event)
    }

    /// The rooms picked that aren't end-to-end encrypted, when the message
    /// is: their server can read what's forwarded there.
    pub fn forward_exposed(&self) -> Vec<String> {
        let (Some(form), Some(post)) = (&self.forwarding, self.forward_post()) else {
            return Vec::new();
        };
        if !post.encrypted {
            return Vec::new();
        }
        form.picked
            .iter()
            .filter(|id| self.rooms().list.get(*id).is_some_and(|r| !r.encrypted))
            .map(|id| self.place_name(id))
            .collect()
    }

    /// The rooms the list offers for what the search says: channels and
    /// DMs you may post in, best match first; with nothing typed, the rooms
    /// with the newest loaded messages first.
    pub fn forward_targets(&self) -> Vec<Target<'_>> {
        let Some(form) = &self.forwarding else {
            return Vec::new();
        };
        let query = form.query.trim().trim_start_matches(['#', '@']);
        let rooms = self.rooms();
        let mut hits: Vec<(u8, i64, Target)> = rooms
            .list
            .iter()
            .filter(|(_, r)| {
                !r.space
                    && !r.voice
                    && r.successor.is_none()
                    && r.posting.can_post(&self.matrix_id, r.encrypted) != Some(false)
            })
            .filter_map(|(id, r)| {
                let score = crate::switcher::score(&r.name, query)?;
                let newest = r.order.last().map_or(0, |(time, _)| *time);
                let place = (!r.dm)
                    .then(|| self.home_space(id))
                    .flatten()
                    .and_then(|space| rooms.list.get(space))
                    .map(|space| space.name.as_str());
                Some((
                    score,
                    newest,
                    Target {
                        id,
                        name: &r.name,
                        dm: r.dm,
                        place,
                    },
                ))
            })
            .collect();
        hits.sort_by_cached_key(|(score, newest, t)| {
            (*score, -newest, t.name.to_lowercase(), t.id.to_owned())
        });
        hits.into_iter().take(LIMIT).map(|(_, _, t)| t).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Event, Failure, RoomList, RoomSummary};
    use async_channel::Receiver;

    fn app() -> (App, Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@me:h".into();
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
                    encrypted: true,
                    ..room("!general:h", "general")
                },
                room("!design:h", "design"),
                RoomSummary {
                    space: true,
                    ..room("!space:h", "Workshop")
                },
                RoomSummary {
                    voice: true,
                    ..room("!lounge:h", "lounge")
                },
                RoomSummary {
                    dm: true,
                    dm_peer: Some("@mira:h".into()),
                    encrypted: true,
                    ..room("!dm:h", "Mira")
                },
            ],
            children,
        }));
        (app, rx)
    }

    fn open(app: &mut App) {
        let _ = app.update(Msg::Forward(Input::Open {
            room: "!general:h".into(),
            event: "$m".into(),
        }));
    }

    fn ids(app: &App) -> Vec<String> {
        app.forward_targets()
            .into_iter()
            .map(|t| t.id.to_owned())
            .collect()
    }

    /// The forwards sent since last asked: (request id, room, note).
    fn sent(rx: &Receiver<Command>) -> Vec<(i64, String, Option<String>)> {
        let mut out = Vec::new();
        while let Ok(command) = rx.try_recv() {
            if let Command::Forward {
                id,
                room,
                event,
                to,
                note,
            } = command
            {
                assert_eq!((room.as_str(), event.as_str()), ("!general:h", "$m"));
                out.push((id, to, note));
            }
        }
        out
    }

    #[test]
    fn the_list_offers_channels_and_dms_but_not_spaces_or_voice() {
        let (mut app, _rx) = app();
        open(&mut app);
        assert_eq!(ids(&app), ["!design:h", "!general:h", "!dm:h"]);
        let _ = app.update(Msg::Forward(Input::Query("#des".into())));
        assert_eq!(ids(&app), ["!design:h"]);
        assert_eq!(app.forward_targets()[0].place, Some("Workshop"));
        let _ = app.update(Msg::Forward(Input::Query("@mi".into())));
        assert_eq!(ids(&app), ["!dm:h"]);
    }

    #[test]
    fn up_to_five_rooms_and_enter_picks_the_best_match() {
        let (mut app, _rx) = app();
        open(&mut app);
        let _ = app.update(Msg::Forward(Input::Query("mi".into())));
        let _ = app.update(Msg::Forward(Input::Enter));
        let form = app.forwarding.as_ref().unwrap();
        assert_eq!(form.picked, ["!dm:h"]);
        assert!(form.query.is_empty());
        let _ = app.update(Msg::Forward(Input::Toggle("!dm:h".into())));
        assert!(app.forwarding.as_ref().unwrap().picked.is_empty());
        for id in ["!a", "!b", "!c", "!d", "!e", "!f"] {
            let _ = app.update(Msg::Forward(Input::Toggle(id.into())));
        }
        let form = app.forwarding.as_ref().unwrap();
        assert_eq!(form.picked.len(), MAX_ROOMS);
        assert!(form.problem.is_some());
    }

    #[test]
    fn send_forwards_to_each_room_and_closes_once_all_arrive() {
        let (mut app, rx) = app();
        open(&mut app);
        let _ = app.update(Msg::Forward(Input::Toggle("!design:h".into())));
        let _ = app.update(Msg::Forward(Input::Toggle("!dm:h".into())));
        let _ = app.update(Msg::Forward(Input::Note("  look  ".into())));
        // Enter with nothing typed sends, the message after each forward.
        let _ = app.update(Msg::Forward(Input::Enter));
        let first = sent(&rx);
        let rooms: Vec<_> = first
            .iter()
            .map(|(_, to, note)| (to.as_str(), note.as_deref()))
            .collect();
        assert_eq!(
            rooms,
            [("!design:h", Some("look")), ("!dm:h", Some("look"))]
        );
        assert!(app.forward_sending());
        // Closing waits for the answers, and so does sending again.
        let _ = app.update(Msg::Forward(Input::Close));
        let _ = app.update(Msg::Forward(Input::Send));
        assert!(app.forwarding.is_some());
        assert!(sent(&rx).is_empty());
        app.engine_event(Event::Done {
            id: first[0].0,
            result: Ok(()),
        });
        assert!(app.forwarding.is_some());
        app.engine_event(Event::Done {
            id: first[1].0,
            result: Err(Failure::Unreachable),
        });
        // The room it couldn't reach stays picked, to try again.
        let form = app.forwarding.as_ref().unwrap();
        assert_eq!(form.picked, ["!dm:h"]);
        assert!(form.problem.as_ref().unwrap().contains("Mira"));
        assert_eq!(app.status, "Forwarded to #design.");
        let _ = app.update(Msg::Forward(Input::Send));
        let retry = sent(&rx);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].1, "!dm:h");
        app.engine_event(Event::Done {
            id: retry[0].0,
            result: Ok(()),
        });
        assert!(app.forwarding.is_none());
        assert_eq!(app.status, "Forwarded to Mira.");
    }

    #[test]
    fn names_read_as_a_list() {
        assert_eq!(listed(&["#a".into()]), "#a");
        assert_eq!(listed(&["#a".into(), "Mira".into()]), "#a and Mira");
        assert_eq!(
            listed(&["#a".into(), "#b".into(), "Mira".into()]),
            "#a, #b and Mira"
        );
    }
}
