use super::{App, BOLD, El, MUTED, Msg, nav};
use crate::community::{Form, Input, Kind};
use iced::{
    Fill,
    widget::{button, column, container, row, text, text_input},
};

pub(super) fn view<'a>(app: &'a App, f: &'a Form) -> El<'a> {
    let busy = app.setup_busy();
    let unlocked = !busy && !f.done && f.created.is_none();
    let mut fields = column![
        text(if f.kind == Kind::Invite {
            "Invite to space"
        } else if f.parent.is_some() {
            "Add channel"
        } else {
            "New space"
        })
        .size(24)
        .font(BOLD)
    ]
    .spacing(12);
    if let Some(parent) = &f.parent {
        fields = fields.push(
            text(format!(
                "Space: {}",
                app.live
                    .list
                    .get(parent)
                    .map_or(parent.as_str(), |r| &r.name)
            ))
            .size(15),
        );
        if f.kind == Kind::Invite {
            fields = fields.push(
                text("Once they join this space, they can join its members-only channels.")
                    .size(14)
                    .color(MUTED),
            );
        } else {
            fields=fields.push(row![
                button("Text channel").style(nav(f.kind==Kind::Text)).on_press_maybe(unlocked.then_some(Msg::Setup(Input::Kind(Kind::Text)))),
                button("Voice channel").style(nav(f.kind==Kind::Voice)).on_press_maybe(unlocked.then_some(Msg::Setup(Input::Kind(Kind::Voice))))
            ].spacing(8))
            .push(text("Encrypted. Space members can join this channel; others need a separate invitation.").size(14).color(MUTED));
        }
    } else {
        fields = fields.push(
            text(
                "An invite-only Matrix space for your community. Add channels after you create it.",
            )
            .size(14)
            .color(MUTED),
        );
    }
    if f.kind != Kind::Invite {
        fields = fields.push(
            text_input("Name", &f.name)
                .id("setup-name")
                .on_input_maybe(unlocked.then_some(|s| Msg::Setup(Input::Name(s))))
                .padding(8),
        );
    }
    if matches!(f.kind, Kind::Space | Kind::Invite) {
        fields = fields
            .push(
                text_input(
                    if f.kind == Kind::Space {
                        "Invite a Matrix user (optional)"
                    } else {
                        "Matrix user ID"
                    },
                    &f.invite,
                )
                .id("setup-invite")
                .on_input_maybe(unlocked.then_some(|s| Msg::Setup(Input::Invite(s))))
                .padding(8),
            )
            .push(
                text("Use the full ID, e.g. @alex:example.org.")
                    .size(13)
                    .color(MUTED),
            );
    }
    if !f.note.is_empty() {
        fields = fields.push(text(&f.note).size(14));
    }
    if let Some(id) = &f.created {
        fields = fields.push(text(id).size(12).color(MUTED));
    }
    fields = fields.push(
        row![
            button(if f.kind == Kind::Invite {
                "Send invite"
            } else if f.created.is_some() {
                "Retry link"
            } else {
                "Create"
            })
            .style(nav(true))
            .on_press_maybe(
                (app.connected && !busy && !f.done && f.valid(&app.matrix_id))
                    .then_some(Msg::Setup(Input::Submit))
            ),
            button(if f.created.is_some() || f.done {
                "Close"
            } else {
                "Cancel"
            })
            .style(nav(false))
            .on_press_maybe((!busy).then_some(Msg::Setup(Input::Close)))
        ]
        .spacing(8),
    );
    container(fields.max_width(490))
        .width(Fill)
        .height(Fill)
        .padding(24)
        .into()
}
