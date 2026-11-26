//! Text messages as the composer typed them, with the mentions and room
//! links picked from autocomplete as pills (pills.rs), and `m.mentions`
//! naming exactly the people mentioned.
//!
//! Every message carries `m.mentions`, empty when nobody is mentioned, as
//! Element sends it: homeservers then notify the people it names rather
//! than anyone whose name merely appears in the text (MSC3952).
use crate::pills::{self, Part, Pill};
use matrix_sdk::ruma::{
    UserId,
    events::{
        Mentions,
        room::message::{FormattedBody, RoomMessageEventContentWithoutRelation as Content},
    },
};

/// A message that renders Markdown. Without pills it is what it always
/// was: the source as `body`, and its HTML if the Markdown makes any.
/// Trailing spaces go, like the one autocomplete leaves after a pick.
pub fn markdown(body: String, pills: &[Pill]) -> Content {
    let body = body.trim_end();
    let parts = pills::split(body, pills);
    let content = if pills::any(&parts) {
        let html = FormattedBody::markdown(pills::markdown(&parts))
            .map_or_else(|| pills::html(&parts), |f| f.body);
        Content::text_html(pills::plain(&parts), html)
    } else {
        Content::text_markdown(body)
    };
    content.add_mentions(mentions(&parts))
}

/// A reply, whose text stays literal (no Markdown, no commands).
pub fn literal(body: String, pills: &[Pill]) -> Content {
    let body = body.trim_end();
    let parts = pills::split(body, pills);
    let content = if pills::any(&parts) {
        Content::text_html(pills::plain(&parts), pills::html(&parts))
    } else {
        Content::text_plain(body)
    };
    content.add_mentions(mentions(&parts))
}

fn mentions(parts: &[Part]) -> Mentions {
    Mentions::with_user_ids(
        pills::user_ids(parts)
            .iter()
            .filter_map(|id| UserId::parse(id).ok()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pills::Target;
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
    use serde_json::{Value, json};

    fn mira() -> Pill {
        Pill {
            label: "@Mira".into(),
            target: Target::User {
                id: "@mira:example.org".into(),
                name: "Mira".into(),
            },
        }
    }
    fn json(content: Content) -> Value {
        serde_json::to_value(RoomMessageEventContent::from(content)).unwrap()
    }

    #[test]
    fn a_mention_is_a_pill_and_notifies_through_m_mentions() {
        let sent = json(markdown("hey **@Mira**, look \n".into(), &[mira()]));
        assert_eq!(sent["msgtype"], "m.text");
        assert_eq!(sent["body"], "hey **Mira**, look");
        assert_eq!(sent["format"], "org.matrix.custom.html");
        assert_eq!(
            sent["formatted_body"],
            "hey <strong><a href=\"https://matrix.to/#/@mira:example.org\">Mira</a></strong>, look"
        );
        assert_eq!(
            sent["m.mentions"],
            json!({"user_ids": ["@mira:example.org"]})
        );
    }

    #[test]
    fn a_reply_keeps_its_text_literal_around_the_pill() {
        let sent = json(literal("*not bold* @Mira ".into(), &[mira()]));
        assert_eq!(sent["body"], "*not bold* Mira", "no trailing space");
        assert_eq!(
            sent["formatted_body"],
            "*not bold* <a href=\"https://matrix.to/#/@mira:example.org\">Mira</a>"
        );
        assert_eq!(sent["m.mentions"]["user_ids"], json!(["@mira:example.org"]));
    }

    #[test]
    fn without_pills_messages_are_unchanged_but_mention_nobody() {
        let sent = json(markdown("Mira, *hi*".into(), &[mira()]));
        assert_eq!(sent["body"], "Mira, *hi*");
        assert_eq!(sent["formatted_body"], "Mira, <em>hi</em>");
        assert_eq!(sent["m.mentions"], json!({}));
        let plain = json(markdown("just text".into(), &[]));
        assert!(plain.get("formatted_body").is_none());
        assert_eq!(plain["m.mentions"], json!({}));
        let reply = json(literal("**hi**".into(), &[]));
        assert_eq!(
            (reply["body"].as_str(), reply.get("format")),
            (Some("**hi**"), None)
        );
    }

    #[test]
    fn a_room_link_points_at_matrix_to() {
        let general = Pill {
            label: "#general".into(),
            target: Target::Room {
                id: "!abc:example.org".into(),
                via: vec!["example.org".into()],
            },
        };
        let sent = json(markdown("see #general".into(), &[general]));
        let href = "https://matrix.to/#/!abc:example.org?via=example.org";
        assert_eq!(sent["body"], format!("see {href}"));
        assert_eq!(
            sent["formatted_body"],
            format!("see <a href=\"{href}\">#general</a>")
        );
        assert_eq!(sent["m.mentions"], json!({}));
    }
}
