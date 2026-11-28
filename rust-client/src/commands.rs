//! Discord's built-in slash commands, typed at the start of a message:
//! `/me` sends an action (`m.emote`, which the timeline shows as
//! "* Name text"), `/shrug` appends ¯\_(ツ)_/¯ and `/spoiler` hides the
//! whole message. Any other text starting with a slash goes out as written,
//! as Discord sends text that names no command.
//!
//! The composer lists the commands while a slash is the first thing typed
//! (autocomplete.rs). The engine reads the command when it sends
//! (`engine::timeline::Timelines::send`), so the App's own copy of the
//! draft stays the text the user typed.
use crate::{
    engine::compose,
    pills::{self, Part, Pill},
};
use matrix_sdk::ruma::events::room::message::{
    EmoteMessageEventContent, MessageType, RoomMessageEventContentWithoutRelation as Content,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Me,
    Shrug,
    Spoiler,
}

/// A command as the composer's hints list it.
#[derive(Debug, PartialEq, Eq)]
pub struct Spec {
    pub command: Command,
    /// Without its slash.
    pub name: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [Spec; 3] = [
    Spec {
        command: Command::Me,
        name: "me",
        about: "Displays your message as an action",
    },
    Spec {
        command: Command::Shrug,
        name: "shrug",
        about: "Appends ¯\\_(ツ)_/¯ to your message",
    },
    Spec {
        command: Command::Spoiler,
        name: "spoiler",
        about: "Marks your message as a spoiler",
    },
];

pub const SHRUG: &str = "¯\\_(ツ)_/¯";

/// The command `body` starts with, and the text after it, trimmed. The
/// name is a whole word: `/meh` is no command, and `/me` is one.
pub fn parse(body: &str) -> Option<(Command, &str)> {
    let rest = body.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, text) = rest.split_at(end);
    let spec = COMMANDS
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))?;
    Some((spec.command, text.trim()))
}

/// Why `body` can't be sent yet: `/me` and `/spoiler` need a message, as on
/// Discord. The composer keeps the draft and says so.
pub fn incomplete(body: &str) -> Option<String> {
    match parse(body)? {
        (command @ (Command::Me | Command::Spoiler), "") => {
            let name = COMMANDS.iter().find(|s| s.command == command)?.name;
            Some(format!("Type a message after /{name}."))
        }
        _ => None,
    }
}

/// The commands whose names start with `prefix` (no slash), for the
/// composer's hints.
pub fn matching(prefix: &str) -> Vec<&'static Spec> {
    let prefix = prefix.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|s| s.name.starts_with(&prefix))
        .collect()
}

/// The message `body` sends: a command's, or the text as typed.
/// `markdown`: messages render Markdown; replies keep their text literal.
pub fn content(body: String, pills: &[Pill], markdown: bool) -> Content {
    let Some((command, text)) = parse(&body) else {
        return if markdown {
            compose::markdown(body, pills)
        } else {
            compose::literal(body, pills)
        };
    };
    let (plain, html, mentions) = compose::parts(text, pills, markdown);
    // Commands that wrap the text need it as HTML even when it has no
    // formatting of its own.
    let literal = |plain: &str| pills::html(&[Part::Text(plain)]);
    let msgtype = match command {
        Command::Me => MessageType::Emote(match html {
            Some(html) => EmoteMessageEventContent::html(plain, html),
            None => EmoteMessageEventContent::plain(plain),
        }),
        // Always with HTML: a plain body is read as Markdown (rich.rs), and
        // Markdown would take the shrug's backslash for an escape.
        Command::Shrug if text.is_empty() => MessageType::text_html(SHRUG, SHRUG),
        Command::Shrug => {
            let html = html.unwrap_or_else(|| literal(&plain));
            MessageType::text_html(format!("{plain} {SHRUG}"), format!("{html} {SHRUG}"))
        }
        // Element's spoiler in the HTML; Discord's syntax in the plain body.
        Command::Spoiler => {
            let html = html.unwrap_or_else(|| literal(&plain));
            MessageType::text_html(
                format!("||{plain}||"),
                format!("<span data-mx-spoiler>{html}</span>"),
            )
        }
    };
    Content::new(msgtype).add_mentions(mentions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pills::Target;
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
    use serde_json::{Value, json};

    fn sent(body: &str, pills: &[Pill], markdown: bool) -> Value {
        serde_json::to_value(RoomMessageEventContent::from(content(
            body.into(),
            pills,
            markdown,
        )))
        .unwrap()
    }
    fn mira() -> Pill {
        Pill {
            label: "@Mira".into(),
            target: Target::User {
                id: "@mira:example.org".into(),
                name: "Mira".into(),
            },
        }
    }

    #[test]
    fn a_command_is_a_slash_and_a_whole_word_at_the_start() {
        assert_eq!(parse("/me waves"), Some((Command::Me, "waves")));
        assert_eq!(parse("/ME  waves \n"), Some((Command::Me, "waves")));
        assert_eq!(parse("/shrug"), Some((Command::Shrug, "")));
        assert_eq!(
            parse("/spoiler line one\nline two"),
            Some((Command::Spoiler, "line one\nline two"))
        );
        assert_eq!(parse("/meh"), None, "not a word of its own");
        assert_eq!(parse(" /me waves"), None, "not at the start");
        assert_eq!(parse("say /me"), None);
        assert_eq!(parse("/unknown thing"), None);
        assert_eq!(parse("/"), None);
        assert_eq!(parse("//me"), None);
    }

    #[test]
    fn me_and_spoiler_need_a_message_and_shrug_does_not() {
        assert_eq!(incomplete("/me "), Some("Type a message after /me.".into()));
        assert_eq!(
            incomplete("/spoiler"),
            Some("Type a message after /spoiler.".into())
        );
        assert_eq!(incomplete("/shrug"), None);
        assert_eq!(incomplete("/me waves"), None);
        assert_eq!(incomplete("hello"), None);
    }

    #[test]
    fn hints_match_the_typed_prefix() {
        let names = |p: &str| matching(p).iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(""), ["me", "shrug", "spoiler"]);
        assert_eq!(names("s"), ["shrug", "spoiler"]);
        assert_eq!(names("SP"), ["spoiler"]);
        assert!(names("x").is_empty());
    }

    #[test]
    fn me_sends_an_emote_with_its_formatting_and_mentions() {
        let plain = sent("/me waves", &[], true);
        assert_eq!(plain["msgtype"], "m.emote");
        assert_eq!(plain["body"], "waves");
        assert!(plain.get("formatted_body").is_none());
        assert_eq!(plain["m.mentions"], json!({}));
        let rich = sent("/me waves at @Mira **hard**", &[mira()], true);
        assert_eq!(rich["msgtype"], "m.emote");
        assert_eq!(rich["body"], "waves at Mira **hard**");
        assert_eq!(
            rich["formatted_body"],
            "waves at <a href=\"https://matrix.to/#/@mira:example.org\">Mira</a> <strong>hard</strong>"
        );
        assert_eq!(rich["m.mentions"]["user_ids"], json!(["@mira:example.org"]));
    }

    #[test]
    fn shrug_appends_and_keeps_its_backslash() {
        let alone = sent("/shrug", &[], true);
        assert_eq!(alone["msgtype"], "m.text");
        assert_eq!(alone["body"], "¯\\_(ツ)_/¯");
        assert_eq!(alone["formatted_body"], "¯\\_(ツ)_/¯");
        let after = sent("/shrug no idea", &[], true);
        assert_eq!(after["body"], "no idea ¯\\_(ツ)_/¯");
        assert_eq!(after["formatted_body"], "no idea ¯\\_(ツ)_/¯");
        // Literal text (a reply) is escaped in the HTML.
        let reply = sent("/shrug <ok>", &[], false);
        assert_eq!(reply["body"], "<ok> ¯\\_(ツ)_/¯");
        assert_eq!(reply["formatted_body"], "&lt;ok&gt; ¯\\_(ツ)_/¯");
        let bold = sent("/shrug **why**", &[], true);
        assert_eq!(bold["formatted_body"], "<strong>why</strong> ¯\\_(ツ)_/¯");
    }

    #[test]
    fn spoiler_hides_the_message_for_html_and_plain_clients() {
        let s = sent("/spoiler the butler did it", &[], true);
        assert_eq!(s["msgtype"], "m.text");
        assert_eq!(s["body"], "||the butler did it||");
        assert_eq!(
            s["formatted_body"],
            "<span data-mx-spoiler>the butler did it</span>"
        );
        assert_eq!(s["format"], "org.matrix.custom.html");
    }

    #[test]
    fn replies_read_commands_but_keep_text_literal() {
        let r = sent("/me *not italic*", &[], false);
        assert_eq!(r["msgtype"], "m.emote");
        assert_eq!(r["body"], "*not italic*");
        assert!(r.get("formatted_body").is_none());
        let s = sent("/spoiler a\nb", &[], false);
        assert_eq!(s["formatted_body"], "<span data-mx-spoiler>a<br>b</span>");
    }

    #[test]
    fn text_that_names_no_command_goes_out_as_typed() {
        let t = sent("/meh **ok**", &[], true);
        assert_eq!(t["msgtype"], "m.text");
        assert_eq!(t["body"], "/meh **ok**");
        assert_eq!(t["formatted_body"], "/meh <strong>ok</strong>");
        let r = sent("/unknown", &[], false);
        assert_eq!(
            (r["body"].as_str(), r.get("format")),
            (Some("/unknown"), None)
        );
    }
}
