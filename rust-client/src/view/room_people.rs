use super::{App, BOLD, El, MUTED, Msg, SIDE, nav, surface};
use crate::moderation::{Action, Kind};
use iced::{
    Fill,
    widget::{Column, Row, Space, button, column, container, row, scrollable, text, text_input},
};

pub(super) fn view(app: &App) -> El<'_> {
    let d = &app.directory;
    let busy = app.membership_busy();
    let mut panel = column![
        row![
            text("Room members").font(BOLD).width(Fill),
            button(text("Refresh").size(12))
                .style(nav(false))
                .on_press_maybe((app.connected && !busy).then_some(Msg::RefreshMembers))
        ],
        text_input("Find member by name or ID", &d.filter)
            .on_input(Msg::MemberFilter)
            .size(13)
            .padding(5),
    ]
    .spacing(6)
    .padding(10);
    if let Some(id) = &d.selected
        && let Some(m) = d.users.get(id)
    {
        panel = panel
            .push(text(id).size(13).font(BOLD))
            .push(text(format!("Membership: {}", m.kind.label())).size(13));
        let mut actions = Row::new().spacing(4);
        let mut available = false;
        for action in Action::ALL {
            if let Some(change) = app.member_change(action) {
                available = true;
                actions = actions.push(
                    button(text(action.label()).size(14))
                        .style(nav(false))
                        .on_press_maybe((!busy).then_some(Msg::Moderate(change))),
                );
            }
        }
        if available {
            panel = panel
                .push(
                    text_input("Reason (optional)", &d.reason)
                        .on_input_maybe((!busy).then_some(Msg::MemberReason))
                        .size(13)
                        .padding(5),
                )
                .push(actions.wrap());
        }
    }
    if busy {
        panel = panel.push(text("Updating room membership…").size(12).color(MUTED));
    }
    if !d.note.is_empty() {
        panel = panel.push(text(&d.note).size(12));
    }
    let query = d.filter.trim().to_lowercase();
    let mut list = Column::new().spacing(2);
    let mut total = 0;
    for (id, m) in &d.users {
        if query.is_empty() && m.kind == Kind::Left && d.selected.as_ref() != Some(id) {
            continue;
        }
        if !query.is_empty()
            && !id.to_lowercase().contains(&query)
            && !m.name.to_lowercase().contains(&query)
        {
            continue;
        }
        total += 1;
        if total > 80 {
            continue;
        }
        let name = if m.name.is_empty() {
            crate::rooms::short(id)
        } else {
            &m.name
        };
        list = list.push(
            button(
                column![
                    row![
                        text(name).size(14),
                        Space::new().width(Fill),
                        text(m.kind.label()).size(12)
                    ],
                    text(id).size(11).color(MUTED)
                ]
                .spacing(1),
            )
            .width(Fill)
            .padding([4, 5])
            .style(nav(d.selected.as_ref() == Some(id)))
            .on_press(Msg::SelectMember(id.clone())),
        );
    }
    if total == 0 && !busy {
        list = list.push(text("No matching members.").size(13).color(MUTED));
    }
    if total > 80 {
        panel =
            panel.push(text(format!("Showing 80 of {total}. Type to narrow the list.")).size(12));
    }
    container(panel.push(scrollable(list).height(Fill)))
        .width(258)
        .height(Fill)
        .style(|_| surface(SIDE, 0.))
        .into()
}
