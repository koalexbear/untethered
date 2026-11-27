//! Polls (MSC3381, ROADMAP M9) on the matrix-sdk engine: what the poll
//! card shows, read from the timeline's poll state, and the events that
//! start, vote in and end a poll. They go out under the unstable names
//! (`org.matrix.msc3381.poll.*`), as Element sends them: Element and the
//! SDK's timeline read those, and nothing reads the stable `m.poll.*` yet.
use super::Failure;
use matrix_sdk::ruma::{
    OwnedEventId, TransactionId,
    events::{
        AnyMessageLikeEventContent,
        poll::{
            start::PollKind,
            unstable_end::UnstablePollEndEventContent,
            unstable_response::UnstablePollResponseEventContent,
            unstable_start::{
                NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
                UnstablePollStartContentBlock, UnstablePollStartEventContent,
            },
        },
    },
};
use matrix_sdk_ui::timeline::PollState;

/// The most answers a poll may have (MSC3381).
pub const MAX_ANSWERS: usize = 20;

/// A poll as its card shows it.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Poll {
    pub question: String,
    pub answers: Vec<Answer>,
    /// How many answers one person may pick: 1 for a single choice.
    pub max_selections: u64,
    /// Results show while it's open (`m.disclosed`); otherwise only once it
    /// has ended.
    pub disclosed: bool,
    pub ended: bool,
    pub edited: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub id: String,
    pub text: String,
    /// Who picked it, in no particular order.
    pub voters: Vec<String>,
}

/// A poll to start, from the Create Poll dialog.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    pub question: String,
    pub answers: Vec<String>,
    /// Anyone may pick several answers, up to all of them.
    pub multiple: bool,
    /// Voters see the results as soon as they've voted, rather than when the
    /// poll ends.
    pub disclosed: bool,
}

impl Poll {
    pub(super) fn read(state: &PollState) -> Self {
        let results = state.results();
        Poll {
            question: results.question,
            answers: results
                .answers
                .into_iter()
                .map(|a| Answer {
                    voters: results
                        .votes
                        .get(&a.id)
                        .map(|users| users.iter().map(ToString::to_string).collect())
                        .unwrap_or_default(),
                    id: a.id,
                    text: a.text,
                })
                .collect(),
            max_selections: results.max_selections.max(1),
            disclosed: results.kind == PollKind::Disclosed,
            ended: results.end_time.is_some(),
            edited: results.has_been_edited,
        }
    }

    /// The answers `user` picked, by ID.
    pub fn picked(&self, user: &str) -> Vec<String> {
        self.answers
            .iter()
            .filter(|a| a.voters.iter().any(|v| v == user))
            .map(|a| a.id.clone())
            .collect()
    }

    /// How many people voted.
    pub fn voters(&self) -> usize {
        let mut all: Vec<&str> = self
            .answers
            .iter()
            .flat_map(|a| a.voters.iter().map(String::as_str))
            .collect();
        all.sort_unstable();
        all.dedup();
        all.len()
    }

    /// The answers with the most votes, none if nobody voted.
    pub fn leaders(&self) -> Vec<&Answer> {
        let most = self.answers.iter().map(|a| a.voters.len()).max();
        match most {
            None | Some(0) => Vec::new(),
            Some(most) => self
                .answers
                .iter()
                .filter(|a| a.voters.len() == most)
                .collect(),
        }
    }

    /// The text an end event carries for clients without polls, as
    /// Element words it.
    fn end_text(&self) -> String {
        match self.leaders().as_slice() {
            [] => "The poll has ended. No votes were cast.".to_owned(),
            leaders => format!(
                "The poll has ended. Top answer: {}",
                leaders
                    .iter()
                    .map(|a| a.text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl Draft {
    /// The question and answers with surrounding space trimmed and empty
    /// answers dropped; why it can't be sent otherwise.
    pub fn tidy(&self) -> Result<Draft, &'static str> {
        let question = self.question.trim().to_owned();
        let answers: Vec<String> = self
            .answers
            .iter()
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect();
        if question.is_empty() {
            return Err("Ask a question.");
        }
        if answers.len() < 2 {
            return Err("Add at least two answers.");
        }
        if answers.len() > MAX_ANSWERS {
            return Err("A poll can have at most 20 answers.");
        }
        Ok(Draft {
            question,
            answers,
            ..*self
        })
    }

    /// The start event. Its fallback text lists the answers, as Element's
    /// does, for clients without polls.
    pub(super) fn content(&self) -> Result<AnyMessageLikeEventContent, Failure> {
        let draft = self.tidy().map_err(|_| Failure::Failed)?;
        let answers: Vec<UnstablePollAnswer> = draft
            .answers
            .iter()
            .map(|text| UnstablePollAnswer::new(TransactionId::new().to_string(), text))
            .collect();
        let count = answers.len();
        let answers = UnstablePollAnswers::try_from(answers).map_err(|_| Failure::Failed)?;
        let mut block = UnstablePollStartContentBlock::new(draft.question.clone(), answers);
        block.kind = if draft.disclosed {
            PollKind::Disclosed
        } else {
            PollKind::Undisclosed
        };
        if draft.multiple {
            block.max_selections = (count as u32).into();
        }
        let fallback = std::iter::once(draft.question.clone())
            .chain(
                draft
                    .answers
                    .iter()
                    .enumerate()
                    .map(|(i, a)| format!("{}. {a}", i + 1)),
            )
            .collect::<Vec<_>>()
            .join("\n");
        Ok(
            UnstablePollStartEventContent::New(NewUnstablePollStartEventContent::plain_text(
                fallback, block,
            ))
            .into(),
        )
    }
}

/// A vote: `answers` replaces whatever `user` picked before; none takes
/// the vote back.
pub(super) fn response(poll: OwnedEventId, answers: Vec<String>) -> AnyMessageLikeEventContent {
    UnstablePollResponseEventContent::new(answers, poll).into()
}

/// Ending `state`'s poll, with its result in the fallback text.
pub(super) fn end(poll: OwnedEventId, state: &PollState) -> AnyMessageLikeEventContent {
    UnstablePollEndEventContent::new(Poll::read(state).end_text(), poll).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(votes: &[(&str, &[&str])]) -> Poll {
        Poll {
            question: "Lunch?".into(),
            answers: votes
                .iter()
                .map(|(text, voters)| Answer {
                    id: text.to_lowercase(),
                    text: (*text).into(),
                    voters: voters.iter().map(|v| (*v).into()).collect(),
                })
                .collect(),
            max_selections: 2,
            disclosed: true,
            ended: false,
            edited: false,
        }
    }

    #[test]
    fn counts_people_once_and_finds_the_leaders() {
        let p = poll(&[
            ("Pizza", &["@a:x", "@b:x"]),
            ("Soup", &["@a:x"]),
            ("Tacos", &["@c:x", "@b:x"]),
        ]);
        assert_eq!(p.voters(), 3);
        assert_eq!(p.picked("@b:x"), ["pizza", "tacos"]);
        let leaders: Vec<_> = p.leaders().iter().map(|a| a.text.as_str()).collect();
        assert_eq!(leaders, ["Pizza", "Tacos"]);
        assert_eq!(p.end_text(), "The poll has ended. Top answer: Pizza, Tacos");
        let none = poll(&[("Pizza", &[]), ("Soup", &[])]);
        assert!(none.leaders().is_empty());
        assert_eq!(none.end_text(), "The poll has ended. No votes were cast.");
    }

    #[test]
    fn a_draft_needs_a_question_and_two_answers() {
        let draft = |q: &str, a: &[&str]| Draft {
            question: q.into(),
            answers: a.iter().map(|a| (*a).into()).collect(),
            multiple: false,
            disclosed: true,
        };
        assert_eq!(draft("  ", &["a", "b"]).tidy(), Err("Ask a question."));
        assert_eq!(
            draft("Lunch?", &["Pizza", "  ", ""]).tidy(),
            Err("Add at least two answers.")
        );
        let many: Vec<String> = (0..21).map(|i| i.to_string()).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(draft("Lunch?", &many).tidy().is_err());
        let tidy = draft(" Lunch? ", &[" Pizza", "", "Soup "]).tidy().unwrap();
        assert_eq!(tidy.question, "Lunch?");
        assert_eq!(tidy.answers, ["Pizza", "Soup"]);
    }

    #[test]
    fn the_start_event_reads_as_element_sends_it() {
        let draft = Draft {
            question: "Lunch?".into(),
            answers: vec!["Pizza".into(), "Soup".into(), "Tacos".into()],
            multiple: true,
            disclosed: false,
        };
        let content = draft.content().unwrap();
        let json = serde_json::to_value(&content).unwrap();
        let start = &json["org.matrix.msc3381.poll.start"];
        assert_eq!(start["question"]["org.matrix.msc1767.text"], "Lunch?");
        assert_eq!(start["kind"], "org.matrix.msc3381.poll.undisclosed");
        assert_eq!(start["max_selections"], 3);
        let answers = start["answers"].as_array().unwrap();
        assert_eq!(answers.len(), 3);
        assert_eq!(answers[2]["org.matrix.msc1767.text"], "Tacos");
        assert_ne!(answers[0]["id"], answers[1]["id"]);
        assert_eq!(
            json["org.matrix.msc1767.text"],
            "Lunch?\n1. Pizza\n2. Soup\n3. Tacos"
        );
        let single = Draft {
            multiple: false,
            disclosed: true,
            ..draft
        };
        let json = serde_json::to_value(single.content().unwrap()).unwrap();
        let start = &json["org.matrix.msc3381.poll.start"];
        assert_eq!(start["kind"], "org.matrix.msc3381.poll.disclosed");
        assert_eq!(start["max_selections"], 1);
    }
}
