//! Mentions and room links picked from the composer's autocomplete, and the
//! message text they become.
//!
//! The composer is plain text, so a pick is remembered as its label ("@Mira",
//! "#general") and where it points. When the message goes out, every place
//! the label still stands becomes a matrix.to link in `formatted_body`, as
//! Element sends pills; a label the user has since deleted or changed is just
//! text. People also go into `m.mentions`, which is what notifies them and
//! puts the message in their mentions inbox (MSC3952).
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pill {
    /// The text as it stands in the composer.
    pub label: String,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// A person, by Matrix ID. `name` is the link text, as Element writes it.
    User { id: String, name: String },
    /// A room, by ID, and servers that can route a visitor to it.
    Room { id: String, via: Vec<String> },
}

impl Target {
    /// The matrix.to permalink, which every client recognises as a pill.
    pub fn href(&self) -> String {
        match self {
            Self::User { id, .. } => format!("https://matrix.to/#/{id}"),
            Self::Room { id, via } => {
                let mut link = format!("https://matrix.to/#/{id}");
                for (i, server) in via.iter().enumerate() {
                    link.push(if i == 0 { '?' } else { '&' });
                    link.push_str("via=");
                    link.push_str(server);
                }
                link
            }
        }
    }
}

/// A message split at its pills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Text(&'a str),
    Pill(&'a Pill),
}

/// The pills still standing in `text`, in order. A label counts only as a
/// whole word, so "@Mi" isn't found inside "@Mira"; longer labels win
/// where two would overlap.
pub fn split<'a>(text: &'a str, pills: &'a [Pill]) -> Vec<Part<'a>> {
    let mut longest: Vec<&Pill> = pills.iter().filter(|p| !p.label.is_empty()).collect();
    longest.sort_by_key(|p| std::cmp::Reverse(p.label.len()));
    let mut found: Vec<(usize, &Pill)> = Vec::new();
    for pill in longest {
        for (at, _) in text.match_indices(pill.label.as_str()) {
            let end = at + pill.label.len();
            let overlaps = found
                .iter()
                .any(|(s, p)| at < s + p.label.len() && *s < end);
            if !overlaps && word_edge(text, at, end) {
                found.push((at, pill));
            }
        }
    }
    found.sort_by_key(|(at, _)| *at);
    let mut parts = Vec::new();
    let mut rest = 0;
    for (at, pill) in found {
        if at > rest {
            parts.push(Part::Text(&text[rest..at]));
        }
        parts.push(Part::Pill(pill));
        rest = at + pill.label.len();
    }
    if rest < text.len() {
        parts.push(Part::Text(&text[rest..]));
    }
    parts
}

/// Nothing word-like touches `text[start..end]` on either side.
fn word_edge(text: &str, start: usize, end: usize) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !text[..start].chars().next_back().is_some_and(word)
        && !text[end..].chars().next().is_some_and(word)
}

/// What a pill reads as in a link: a person's name, a room's label.
fn link_text(pill: &Pill) -> &str {
    match &pill.target {
        Target::User { name, .. } => name,
        Target::Room { .. } => &pill.label,
    }
}

/// The plain `body`: people by name as Element writes them, rooms as their
/// link so a client without HTML can still follow it.
pub fn plain(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => (*t).to_owned(),
            Part::Pill(pill) => match &pill.target {
                Target::User { name, .. } => name.clone(),
                Target::Room { .. } => pill.target.href(),
            },
        })
        .collect()
}

/// Markdown source with each pill as a link, for messages that render
/// Markdown. The link text is escaped so a name can't add formatting.
pub fn markdown(parts: &[Part]) -> String {
    let mut out = String::new();
    for p in parts {
        match p {
            Part::Text(t) => out.push_str(t),
            Part::Pill(pill) => {
                out.push('[');
                for c in link_text(pill).chars() {
                    if c.is_ascii_punctuation() {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out.push_str("](<");
                out.push_str(&pill.target.href());
                out.push_str(">)");
            }
        }
    }
    out
}

/// HTML that keeps the text literal, for replies, which never render
/// Markdown: escaped text, line breaks and the pills' links.
pub fn html(parts: &[Part]) -> String {
    let mut out = String::new();
    for p in parts {
        match p {
            Part::Text(t) => out.push_str(&escape(t).replace('\n', "<br>")),
            Part::Pill(pill) => {
                out.push_str("<a href=\"");
                out.push_str(&escape(&pill.target.href()));
                out.push_str("\">");
                out.push_str(&escape(link_text(pill)));
                out.push_str("</a>");
            }
        }
    }
    out
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The people the message mentions, once each, for `m.mentions.user_ids`.
pub fn user_ids(parts: &[Part]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for p in parts {
        if let Part::Pill(Pill {
            target: Target::User { id, .. },
            ..
        }) = p
            && !ids.contains(id)
        {
            ids.push(id.clone());
        }
    }
    ids
}

/// Whether any pill survived editing.
pub fn any(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Pill(_)))
}

/// The server part of a Matrix ID, for `via`.
pub fn server(id: &str) -> Option<&str> {
    id.split_once(':').map(|(_, s)| s).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mira() -> Pill {
        Pill {
            label: "@Mira".into(),
            target: Target::User {
                id: "@mira:example.org".into(),
                name: "Mira".into(),
            },
        }
    }
    fn general() -> Pill {
        Pill {
            label: "#general".into(),
            target: Target::Room {
                id: "!abc:example.org".into(),
                via: vec!["example.org".into(), "other.net".into()],
            },
        }
    }

    #[test]
    fn a_mention_becomes_a_pill_link_and_an_m_mentions_entry() {
        let pills = [mira()];
        let parts = split("hey @Mira, look", &pills);
        assert_eq!(plain(&parts), "hey Mira, look");
        assert_eq!(
            markdown(&parts),
            "hey [Mira](<https://matrix.to/#/@mira:example.org>), look"
        );
        assert_eq!(
            html(&parts),
            "hey <a href=\"https://matrix.to/#/@mira:example.org\">Mira</a>, look"
        );
        assert_eq!(user_ids(&parts), ["@mira:example.org"]);
    }

    #[test]
    fn a_room_links_to_matrix_to_with_via_servers() {
        let pills = [general()];
        let parts = split("see #general", &pills);
        let href = "https://matrix.to/#/!abc:example.org?via=example.org&via=other.net";
        assert_eq!(plain(&parts), format!("see {href}"));
        assert_eq!(
            html(&parts),
            "see <a href=\"https://matrix.to/#/!abc:example.org?via=example.org&amp;via=other.net\">#general</a>"
        );
        assert!(user_ids(&parts).is_empty(), "rooms notify nobody");
    }

    #[test]
    fn deleted_or_changed_labels_are_just_text() {
        let pills = [mira(), general()];
        let parts = split("@Miranda and #generalist", &pills);
        assert!(!any(&parts));
        assert_eq!(plain(&parts), "@Miranda and #generalist");
        assert!(user_ids(&parts).is_empty());
        assert!(split("", &pills).is_empty());
    }

    #[test]
    fn every_occurrence_counts_and_a_person_is_listed_once() {
        let pills = [mira()];
        let parts = split("@Mira @Mira", &pills);
        assert_eq!(plain(&parts), "Mira Mira");
        assert_eq!(user_ids(&parts), ["@mira:example.org"]);
    }

    #[test]
    fn the_longer_of_two_overlapping_labels_wins() {
        let short = Pill {
            label: "@Mira".into(),
            target: Target::User {
                id: "@m1:a".into(),
                name: "Mira".into(),
            },
        };
        let long = Pill {
            label: "@Mira (@m2:b)".into(),
            target: Target::User {
                id: "@m2:b".into(),
                name: "Mira (@m2:b)".into(),
            },
        };
        let pills = [short, long];
        let parts = split("@Mira (@m2:b) and @Mira", &pills);
        assert_eq!(user_ids(&parts), ["@m2:b", "@m1:a"]);
    }

    #[test]
    fn names_cannot_inject_markup_or_formatting() {
        let pills = [Pill {
            label: "@*<b>".into(),
            target: Target::User {
                id: "@x:y".into(),
                name: "*<b>".into(),
            },
        }];
        let parts = split("hi @*<b> & co\nbye", &pills);
        assert_eq!(
            markdown(&parts),
            "hi [\\*\\<b\\>](<https://matrix.to/#/@x:y>) & co\nbye"
        );
        assert_eq!(
            html(&parts),
            "hi <a href=\"https://matrix.to/#/@x:y\">*&lt;b&gt;</a> &amp; co<br>bye"
        );
    }
}
