//! Polls (polls.rs) on screen: the card a poll shows as in the timeline,
//! shaped like Discord's, the Create a Poll dialog, and the composer's +
//! menu that opens it.
use super::{
    App, BLURPLE, BODY, BOLD, CHANNEL, CHAT, El, FLOAT, GREEN, HOVER, Icon, LINE, LINK, MUTED, Msg,
    SIDE, TEXT, icon, icon_button, input_style, plain, primary, scroll_style, surface,
    thin_scrollbar,
};
use crate::{
    context_menu::Event as Menu,
    polls::{ANSWER_LENGTH, Input, MAX_ANSWERS, Poll, QUESTION, answer_field},
    rooms::Post,
};
use iced::{
    Alignment::Center,
    Border, Color, Fill, Theme, color,
    widget::{
        Column, Space, button, checkbox, column, container, mouse_area, opaque, progress_bar, row,
        scrollable, stack, text, text_input, tooltip,
    },
};

const WARN: Color = color!(0xffc785);

/// "1 vote", "3 votes".
fn votes(n: usize) -> String {
    format!("{n} vote{}", if n == 1 { "" } else { "s" })
}

/// A radio button for a single-choice poll, a checkbox for a multiple one.
fn mark<'a>(multiple: bool, on: bool) -> El<'a> {
    let radius = if multiple { 4. } else { 9. };
    let inner: El = match (on, multiple) {
        (false, _) => Space::new().into(),
        (true, true) => icon(Icon::Check, 14., Color::WHITE),
        (true, false) => container(Space::new())
            .width(8)
            .height(8)
            .style(|_| surface(Color::WHITE, 4.))
            .into(),
    };
    container(inner)
        .center(18)
        .style(move |_| container::Style {
            background: on.then(|| BLURPLE.into()),
            border: Border {
                color: if on { BLURPLE } else { MUTED },
                width: 2.,
                radius: radius.into(),
            },
            ..Default::default()
        })
        .into()
}

/// A short link in the card's footer.
fn footer_link<'a>(label: &'a str, msg: Option<Msg>) -> El<'a> {
    button(text(label).size(13))
        .padding(0)
        .style(plain(LINK, color!(0x5fc5ff)))
        .on_press_maybe(msg)
        .into()
}

impl App {
    /// A poll in the timeline: its question, its answers to click, and how
    /// the votes stand once you may see them.
    pub(super) fn poll_card<'a>(&'a self, p: &'a Post, poll: &'a Poll) -> El<'a> {
        let room = self.selected.clone();
        let event = p.event_id.clone().unwrap_or_default();
        let mine = poll.picked(&self.matrix_id);
        let can_vote = self.may_vote(p);
        let multiple = poll.max_selections > 1;
        let people = poll.voters();
        // Results show to those who voted, or to everyone once it ends;
        // a closed poll keeps them until then.
        let results = poll.ended || (poll.disclosed && !mine.is_empty());
        let leaders: Vec<&str> = if poll.ended {
            poll.leaders().iter().map(|a| a.id.as_str()).collect()
        } else {
            Vec::new()
        };
        let mut hint = if poll.ended {
            "Final results".to_owned()
        } else if !multiple {
            "Select one answer".to_owned()
        } else if poll.max_selections as usize >= poll.answers.len() {
            "Select one or more answers".to_owned()
        } else {
            format!("Select up to {} answers", poll.max_selections)
        };
        if poll.edited {
            hint.push_str(" (edited)");
        }
        let mut card = Column::new().spacing(8).push(
            column![
                text(&poll.question).size(16).font(BOLD).color(TEXT),
                text(hint).size(12).color(CHANNEL),
            ]
            .spacing(2),
        );
        for answer in &poll.answers {
            let on = mine.contains(&answer.id);
            let won = leaders.contains(&answer.id.as_str());
            let mut line = row![
                mark(multiple, on),
                text(&answer.text)
                    .size(15)
                    .font(if won { BOLD } else { iced::Font::default() })
                    .color(if on || won { TEXT } else { BODY })
                    .width(Fill),
            ]
            .spacing(10)
            .align_y(Center);
            let mut content = Column::new().spacing(6);
            if results {
                let count = answer.voters.len();
                let share = if people == 0 {
                    0.
                } else {
                    count as f32 / people as f32
                };
                line = line.push(text(votes(count)).size(12).color(CHANNEL)).push(
                    text(format!("{}%", (share * 100.).round()))
                        .size(14)
                        .font(BOLD)
                        .color(TEXT),
                );
                let fill = if won {
                    GREEN
                } else if on {
                    BLURPLE
                } else {
                    MUTED
                };
                content = content
                    .push(line)
                    .push(progress_bar(0.0..=1.0, share).girth(4).style(move |_| {
                        progress_bar::Style {
                            background: LINE.into(),
                            bar: fill.into(),
                            border: Border::default().rounded(2),
                        }
                    }));
            } else {
                content = content.push(line);
            }
            let pick = can_vote.then(|| {
                Msg::Poll(Input::Pick {
                    room: room.clone(),
                    event: event.clone(),
                    answer: answer.id.clone(),
                })
            });
            card = card.push(
                button(content)
                    .width(Fill)
                    .padding([10, 12])
                    .on_press_maybe(pick)
                    .style(move |_: &Theme, s| button::Style {
                        background: Some(
                            if on {
                                Color { a: 0.18, ..BLURPLE }
                            } else if s == button::Status::Hovered {
                                HOVER
                            } else {
                                CHAT
                            }
                            .into(),
                        ),
                        text_color: BODY,
                        border: Border {
                            color: if on { BLURPLE } else { Color::TRANSPARENT },
                            width: 1.,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    }),
            );
        }
        let mut footer = row![
            text(if people == 0 {
                "No votes yet".to_owned()
            } else {
                votes(people)
            })
            .size(13)
            .color(CHANNEL)
        ]
        .spacing(6)
        .align_y(Center);
        let state = if poll.ended {
            Some("Poll closed")
        } else if !poll.disclosed {
            Some("Results show when the poll ends")
        } else if mine.is_empty() && people > 0 {
            Some("Vote to see the results")
        } else {
            None
        };
        let dot = || text("·").size(13).color(CHANNEL);
        if let Some(state) = state {
            footer = footer.push(dot()).push(text(state).size(13).color(CHANNEL));
        }
        if can_vote && !mine.is_empty() {
            footer = footer.push(dot()).push(footer_link(
                "Remove vote",
                Some(Msg::Poll(Input::Unvote {
                    room: room.clone(),
                    event: event.clone(),
                })),
            ));
        }
        if self.may_end_poll(p) {
            footer = footer.push(dot()).push(footer_link(
                "End poll",
                (!self.poll_ending()).then(|| {
                    Msg::ContextMenu(Menu::EndPoll {
                        room: room.clone(),
                        event: event.clone(),
                    })
                }),
            ));
        }
        card = card.push(footer.wrap());
        container(card)
            .padding([12, 14])
            .width(460)
            .style(|_| surface(SIDE, 8.))
            .into()
    }

    /// The composer's + menu, over the composer: Upload a File, Create Poll.
    pub(super) fn composer_menu(&self) -> Option<El<'_>> {
        if !self.composer_menu {
            return None;
        }
        let entry = |glyph: Icon, label: &'static str, msg: Option<Msg>| -> El<'_> {
            let enabled = msg.is_some();
            button(
                row![
                    icon(glyph, 18., if enabled { BODY } else { CHANNEL }),
                    text(label)
                        .size(14)
                        .color(if enabled { BODY } else { CHANNEL })
                ]
                .spacing(10)
                .align_y(Center),
            )
            .width(Fill)
            .padding([8, 10])
            .on_press_maybe(msg)
            .style(|_, s| button::Style {
                background: (s == button::Status::Hovered || s == button::Status::Pressed)
                    .then(|| BLURPLE.into()),
                text_color: BODY,
                border: Border::default().rounded(4),
                ..Default::default()
            })
            .into()
        };
        Some(
            container(
                column![
                    entry(Icon::File, "Upload a File", Some(Msg::PickFiles)),
                    entry(
                        Icon::Poll,
                        "Create Poll",
                        self.can_create_poll().then_some(Msg::Poll(Input::Open))
                    ),
                ]
                .spacing(2),
            )
            .padding(6)
            .width(220)
            .style(|_| container::Style {
                border: Border {
                    color: LINE,
                    width: 1.,
                    radius: 8.0.into(),
                },
                ..surface(FLOAT, 8.)
            })
            .into(),
        )
    }

    /// Create a Poll, as Discord's: a question, two to twenty answers,
    /// whether people may pick several, and whether the results wait for
    /// the end (Element's closed poll).
    pub(super) fn poll_dialog(&self) -> Option<El<'_>> {
        let form = self.polling.as_ref()?;
        let busy = self.poll_sending();
        let close = (!busy).then_some(Msg::Poll(Input::Close));
        let room = self
            .live
            .list
            .get(&form.room)
            .map(|r| r.name.as_str())
            .unwrap_or_default();
        let header = row![
            column![
                text("Create a Poll").size(20).font(BOLD).color(TEXT),
                text(format!("In #{room}")).size(12).color(MUTED)
            ]
            .spacing(4)
            .width(Fill),
            icon_button(
                Icon::Close,
                "Close",
                false,
                close.clone(),
                tooltip::Position::Left
            )
        ]
        .align_y(iced::alignment::Vertical::Top);
        let label = |s: &'static str| text(s).size(12).font(BOLD).color(MUTED);
        let question = text_input("What question do you want to ask?", &form.question)
            .id(QUESTION)
            .size(15)
            .padding([10, 12])
            .style(input_style)
            .on_input_maybe((!busy).then_some(|q| Msg::Poll(Input::Question(q))))
            .on_submit(Msg::Poll(Input::QuestionEntered));
        let mut answers = Column::new().spacing(8);
        for (i, answer) in form.answers.iter().enumerate() {
            let mut line = row![
                text_input("Type your answer", answer)
                    .id(answer_field(i))
                    .size(15)
                    .padding([10, 12])
                    .style(input_style)
                    .on_input_maybe((!busy).then_some(move |a| Msg::Poll(Input::Answer(i, a))))
                    .on_submit(Msg::Poll(Input::Next(i)))
            ]
            .spacing(6)
            .align_y(Center);
            // Every row keeps its remove button, so removing one never
            // moves the fields (and their focus) around.
            line = line.push(icon_button(
                Icon::Close,
                "Remove answer",
                false,
                (!busy && form.answers.len() > 2).then_some(Msg::Poll(Input::RemoveAnswer(i))),
                tooltip::Position::Left,
            ));
            answers = answers.push(line);
        }
        if form.answers.len() < MAX_ANSWERS {
            answers = answers.push(
                button(
                    row![
                        icon(Icon::Plus, 16., LINK),
                        text("Add another answer").size(14)
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .padding([4, 2])
                .style(plain(LINK, color!(0x5fc5ff)))
                .on_press_maybe((!busy).then_some(Msg::Poll(Input::AddAnswer))),
            );
        }
        let answers = scrollable(answers)
            .direction(thin_scrollbar())
            .style(scroll_style(None))
            .height(iced::Length::Shrink)
            .width(Fill);
        let options = column![
            checkbox(form.multiple)
                .label("Allow multiple answers")
                .size(18)
                .text_size(14)
                .on_toggle_maybe((!busy).then_some(|on| Msg::Poll(Input::Multiple(on)))),
            checkbox(form.closed)
                .label("Hide the results until you end the poll")
                .size(18)
                .text_size(14)
                .on_toggle_maybe((!busy).then_some(|on| Msg::Poll(Input::Closed(on)))),
        ]
        .spacing(10);
        let mut body = column![
            header,
            column![label("QUESTION"), question].spacing(8),
            column![label("ANSWERS"), container(answers).max_height(320)].spacing(8),
            options,
        ]
        .spacing(16);
        if let Some(problem) = form.problem {
            body = body.push(text(problem).size(13).color(WARN));
        }
        let ready = self.connected
            && !busy
            && !form.question.trim().is_empty()
            && form.answers.iter().filter(|a| !a.trim().is_empty()).count() >= 2;
        let post = button(
            text(if busy { "Posting…" } else { "Post" })
                .size(14)
                .font(BOLD),
        )
        .padding([10, 18])
        .style(primary)
        .on_press_maybe(ready.then_some(Msg::Poll(Input::Post)));
        let cancel = button(text("Cancel").size(14))
            .padding([10, 16])
            .style(plain(BODY, Color::WHITE))
            .on_press_maybe(close.clone());
        let footer = container(
            row![
                text(format!("Answers up to {ANSWER_LENGTH} characters"))
                    .size(12)
                    .color(CHANNEL)
                    .width(Fill),
                cancel,
                post
            ]
            .spacing(8)
            .align_y(Center),
        )
        .padding(16)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(SIDE.into()),
            border: Border::default().rounded(iced::border::bottom(8)),
            ..Default::default()
        });
        let dialog = container(column![container(body).padding([20, 16]), footer])
            .width(500)
            .style(|_| surface(CHAT, 8.));
        let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
            container::Style {
                background: Some(Color::from_rgba8(0, 0, 0, 0.7).into()),
                ..Default::default()
            }
        }));
        let backdrop = match close {
            Some(close) => backdrop.on_press(close),
            None => backdrop,
        };
        Some(stack![opaque(backdrop), container(dialog).center(Fill)].into())
    }
}
