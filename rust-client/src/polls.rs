//! Polls (ROADMAP M9), as Element and Discord have them: a poll shows as a
//! card in the timeline, where a click on an answer votes (and votes again,
//! to change it) until its creator ends it; Create Poll in the composer's +
//! menu starts one.
//!
//! Matrix polls are MSC3381's: the engine sends Element's unstable events
//! (engine/polls.rs) and the SDK's timeline counts the votes. A poll's
//! results show to those who voted while it's open, or only once it ends
//! when its creator chose so (`m.undisclosed`, Element's closed poll).
use crate::{App, Msg, Request, engine::Command, engine::polls::Draft, rooms::Post};
use iced::{Task, widget::operation};

pub use crate::engine::polls::{MAX_ANSWERS, Poll};

pub const QUESTION: &str = "poll-question";
/// Discord's limits on a question and an answer.
pub const QUESTION_LENGTH: usize = 300;
pub const ANSWER_LENGTH: usize = 55;

/// The field for answer `i`.
pub fn answer_field(i: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("poll-answer-{i}"))
}

/// Create a Poll, for the room it was opened in.
pub struct Form {
    pub room: String,
    pub question: String,
    pub answers: Vec<String>,
    pub multiple: bool,
    /// Results show only once the poll ends.
    pub closed: bool,
    pub problem: Option<&'static str>,
}

impl Form {
    fn draft(&self) -> Draft {
        Draft {
            question: self.question.clone(),
            answers: self.answers.clone(),
            multiple: self.multiple,
            disclosed: !self.closed,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Input {
    Open,
    Close,
    Question(String),
    /// Enter in the question: on to the first answer.
    QuestionEntered,
    Answer(usize, String),
    /// Enter in answer `i`: on to the next, adding one after the last.
    Next(usize),
    AddAnswer,
    RemoveAnswer(usize),
    Multiple(bool),
    Closed(bool),
    Post,
    /// A click on an answer of the poll at `event`.
    Pick {
        room: String,
        event: String,
        answer: String,
    },
    /// Take your vote back.
    Unvote {
        room: String,
        event: String,
    },
}

/// What a poll request was, for its answer.
#[derive(Clone, Debug)]
pub enum Op {
    Create,
    Vote,
    End,
}

/// What a poll's answers read after a click on `answer`, given what you
/// picked before: a single-choice poll moves the vote, a multiple-choice
/// one turns that answer on or off, up to as many as it allows. None when
/// nothing changes.
pub fn picking(poll: &Poll, before: &[String], answer: &str) -> Option<Vec<String>> {
    if !poll.answers.iter().any(|a| a.id == answer) {
        return None;
    }
    if poll.max_selections <= 1 {
        return (before != [answer]).then(|| vec![answer.to_owned()]);
    }
    let mut picked = before.to_vec();
    if let Some(at) = picked.iter().position(|a| a == answer) {
        picked.remove(at);
    } else if (picked.len() as u64) < poll.max_selections {
        picked.push(answer.to_owned());
    } else {
        return None;
    }
    // In the poll's order, as the card lists them.
    picked.sort_by_key(|id| poll.answers.iter().position(|a| &a.id == id));
    Some(picked)
}

impl App {
    pub fn poll(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Open => {
                if !self.can_create_poll() {
                    return Task::none();
                }
                self.composer_menu = false;
                self.polling = Some(Form {
                    room: self.selected.clone(),
                    question: String::new(),
                    answers: vec![String::new(), String::new()],
                    multiple: false,
                    closed: false,
                    problem: None,
                });
                return operation::focus(QUESTION);
            }
            Input::Close => {
                if !self.poll_sending() {
                    self.polling = None;
                }
            }
            Input::Question(q) => {
                if let Some(form) = &mut self.polling {
                    form.question = q.chars().take(QUESTION_LENGTH).collect();
                    form.problem = None;
                }
            }
            Input::QuestionEntered => return operation::focus(answer_field(0)),
            Input::Answer(i, a) => {
                if let Some(form) = &mut self.polling
                    && let Some(slot) = form.answers.get_mut(i)
                {
                    *slot = a.chars().take(ANSWER_LENGTH).collect();
                    form.problem = None;
                }
            }
            Input::Next(i) => {
                let Some(form) = &mut self.polling else {
                    return Task::none();
                };
                if i + 1 < form.answers.len() {
                    return operation::focus(answer_field(i + 1));
                }
                if form.answers.len() < MAX_ANSWERS && !form.answers[i].trim().is_empty() {
                    form.answers.push(String::new());
                    return operation::focus(answer_field(i + 1));
                }
                return self.poll(Input::Post);
            }
            Input::AddAnswer => {
                if let Some(form) = &mut self.polling
                    && form.answers.len() < MAX_ANSWERS
                {
                    form.answers.push(String::new());
                    return operation::focus(answer_field(form.answers.len() - 1));
                }
            }
            Input::RemoveAnswer(i) => {
                if let Some(form) = &mut self.polling
                    && form.answers.len() > 2
                    && i < form.answers.len()
                {
                    form.answers.remove(i);
                }
            }
            Input::Multiple(on) => {
                if let Some(form) = &mut self.polling {
                    form.multiple = on;
                }
            }
            Input::Closed(on) => {
                if let Some(form) = &mut self.polling {
                    form.closed = on;
                }
            }
            Input::Post => self.post_poll(),
            Input::Pick {
                room,
                event,
                answer,
            } => {
                let Some(post) = self.poll_post(&room, &event) else {
                    return Task::none();
                };
                let Some(poll) = post.poll.as_ref().filter(|_| self.may_vote(post)) else {
                    return Task::none();
                };
                if let Some(answers) = picking(poll, &poll.picked(&self.matrix_id), &answer) {
                    self.vote(room, event, answers);
                }
            }
            Input::Unvote { room, event } => {
                let Some(post) = self.poll_post(&room, &event) else {
                    return Task::none();
                };
                if self.may_vote(post)
                    && post
                        .poll
                        .as_ref()
                        .is_some_and(|p| !p.picked(&self.matrix_id).is_empty())
                {
                    self.vote(room, event, Vec::new());
                }
            }
        }
        Task::none()
    }

    fn vote(&mut self, room: String, event: String, answers: Vec<String>) {
        self.command(
            move |id| Command::Vote {
                id,
                room,
                event,
                answers,
            },
            Request::Poll(Op::Vote),
        );
    }

    fn post_poll(&mut self) {
        if self.poll_sending() {
            return;
        }
        let Some(form) = &mut self.polling else {
            return;
        };
        let draft = match form.draft().tidy() {
            Ok(draft) => draft,
            Err(problem) => {
                form.problem = Some(problem);
                return;
            }
        };
        form.problem = None;
        let room = form.room.clone();
        self.command(
            move |id| Command::CreatePoll { id, room, draft },
            Request::Poll(Op::Create),
        );
    }

    /// End the poll at `event`, once its creator confirmed.
    pub fn end_poll(&mut self, room: String, event: String) {
        if !self
            .poll_post(&room, &event)
            .is_some_and(|p| self.may_end_poll(p))
        {
            return;
        }
        self.command(
            move |id| Command::EndPoll { id, room, event },
            Request::Poll(Op::End),
        );
    }

    pub fn poll_done(&mut self, op: Op, ok: bool) {
        match (op, ok) {
            (Op::Create, true) => self.polling = None,
            (Op::Create, false) => {
                if let Some(form) = &mut self.polling {
                    form.problem = Some("Couldn't post the poll. Try again.");
                }
            }
            (Op::Vote, false) => {
                self.status = "Couldn't send your vote. Try again.".into();
            }
            (Op::End, false) => {
                self.status = "Couldn't end the poll. Try again.".into();
            }
            (_, true) => {}
        }
    }

    pub fn poll_sending(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Poll(Op::Create)))
    }

    pub fn poll_ending(&self) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Request::Poll(Op::End)))
    }

    /// Create Poll needs the matrix-sdk engine and a room you may post in.
    pub fn can_create_poll(&self) -> bool {
        self.logged_in
            && self.verified
            && self.sdk.is_some()
            && self.live.list.get(&self.selected).is_some_and(|r| {
                !r.space
                    && r.successor.is_none()
                    && r.posting.can_post(&self.matrix_id, r.encrypted) != Some(false)
            })
    }

    fn poll_post(&self, room: &str, event: &str) -> Option<&Post> {
        self.rooms().list.get(room)?.by_id(event)
    }

    /// Voting needs a sent poll that's still open, in this room rather than
    /// one it was upgraded from.
    pub fn may_vote(&self, post: &Post) -> bool {
        self.verified
            && self.connected
            && post.event_id.is_some()
            && post.from.is_none()
            && post.poll.as_ref().is_some_and(|p| !p.ended)
    }

    /// Only a poll's creator ends it, as in Element and on Discord.
    pub fn may_end_poll(&self, post: &Post) -> bool {
        post.sender == self.matrix_id && self.may_vote(post)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::polls::Answer;
    use crate::engine::{Event, RoomList, RoomSummary};
    use async_channel::Receiver;

    fn poll(max: u64) -> Poll {
        Poll {
            question: "Lunch?".into(),
            answers: ["a", "b", "c"]
                .iter()
                .map(|id| Answer {
                    id: (*id).into(),
                    text: id.to_uppercase(),
                    voters: Vec::new(),
                })
                .collect(),
            max_selections: max,
            disclosed: true,
            ended: false,
            edited: false,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn a_single_choice_moves_the_vote() {
        let p = poll(1);
        assert_eq!(picking(&p, &[], "b"), Some(ids(&["b"])));
        assert_eq!(picking(&p, &ids(&["b"]), "c"), Some(ids(&["c"])));
        assert_eq!(picking(&p, &ids(&["b"]), "b"), None);
        assert_eq!(picking(&p, &[], "z"), None);
    }

    #[test]
    fn a_multiple_choice_toggles_up_to_its_limit() {
        let p = poll(2);
        assert_eq!(picking(&p, &ids(&["c"]), "a"), Some(ids(&["a", "c"])));
        assert_eq!(picking(&p, &ids(&["a", "c"]), "a"), Some(ids(&["c"])));
        assert_eq!(picking(&p, &ids(&["a", "c"]), "b"), None);
        assert_eq!(picking(&p, &ids(&["a"]), "a"), Some(Vec::new()));
    }

    fn app() -> (App, Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(64);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.matrix_id = "@me:h".into();
        app.engine_event(Event::Rooms(RoomList {
            invites: Vec::new(),
            rooms: vec![RoomSummary {
                id: "!general:h".into(),
                name: "general".into(),
                ..Default::default()
            }],
            children: Default::default(),
        }));
        app.selected = "!general:h".into();
        (app, rx)
    }

    /// The polls sent since last asked; the room's other requests aside.
    fn created(rx: &Receiver<Command>) -> Vec<(i64, String, Draft)> {
        let mut out = Vec::new();
        while let Ok(command) = rx.try_recv() {
            if let Command::CreatePoll { id, room, draft } = command {
                out.push((id, room, draft));
            }
        }
        out
    }

    #[test]
    fn create_poll_sends_the_tidied_draft_and_closes_once_sent() {
        let (mut app, rx) = app();
        let _ = app.update(Msg::Poll(Input::Open));
        let _ = app.update(Msg::Poll(Input::Question(" Lunch? ".into())));
        let _ = app.update(Msg::Poll(Input::Answer(0, "Pizza".into())));
        let _ = app.update(Msg::Poll(Input::Post));
        assert_eq!(
            app.polling.as_ref().unwrap().problem,
            Some("Add at least two answers.")
        );
        assert!(created(&rx).is_empty());
        // Enter in the last answer adds another.
        let _ = app.update(Msg::Poll(Input::Answer(1, "Soup".into())));
        let _ = app.update(Msg::Poll(Input::Next(1)));
        assert_eq!(app.polling.as_ref().unwrap().answers.len(), 3);
        let _ = app.update(Msg::Poll(Input::Multiple(true)));
        let _ = app.update(Msg::Poll(Input::Closed(true)));
        // Enter in an empty last answer posts.
        let _ = app.update(Msg::Poll(Input::Next(2)));
        let [(id, room, draft)] = created(&rx).try_into().expect("one poll sent");
        assert_eq!(room, "!general:h");
        assert_eq!(draft.question, "Lunch?");
        assert_eq!(draft.answers, ["Pizza", "Soup"]);
        assert!(draft.multiple && !draft.disclosed);
        assert!(app.poll_sending());
        // Escape waits for the answer.
        let _ = app.update(Msg::Poll(Input::Close));
        assert!(app.polling.is_some());
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert!(app.polling.is_none());
    }
}
