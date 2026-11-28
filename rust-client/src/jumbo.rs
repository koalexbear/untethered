//! Messages of only emoji show them larger, as Discord's "jumbo" emoji:
//! Unicode emoji and custom ones (ROADMAP M11), with any spacing between,
//! up to 27 of them.
use crate::rich::{Block, Doc, EMOTE, Style};
use unicode_segmentation::UnicodeSegmentation;

/// Discord's limit: more emoji than this stay at text size.
pub const MAX: usize = 27;

/// Whether the message reads as nothing but emoji: its formatted `doc` if
/// it has one (custom emoji are pictures there), otherwise its plain body.
pub fn emoji_only(body: &str, doc: Option<&Doc>) -> bool {
    match doc {
        Some(doc) => doc_emoji(doc),
        None => count(body).is_some_and(|n| n > 0),
    }
}

/// One plain paragraph of emoji, custom or not.
fn doc_emoji(doc: &Doc) -> bool {
    let [Block::Paragraph(runs)] = doc.blocks.as_slice() else {
        return false;
    };
    let mut total = 0;
    for run in runs {
        let n = if run.style.emote.is_some() && run.style.spoiler.is_none() {
            // Two of the same picture side by side share one run.
            run.text.matches(EMOTE).count()
        } else if run.style == Style::default() {
            match count(&run.text) {
                Some(n) => n,
                None => return false,
            }
        } else {
            return false;
        };
        total += n;
        if total > MAX {
            return false;
        }
    }
    total > 0
}

/// How many emoji `text` is, if it's only emoji and spaces and no more
/// than [`MAX`] of them.
fn count(text: &str) -> Option<usize> {
    // Even the longest sequences (families, flags of subdivisions) are
    // under 30 bytes each.
    if text.len() > MAX * 32 {
        return None;
    }
    let mut n = 0;
    for g in text.graphemes(true) {
        if g.chars().all(char::is_whitespace) {
            continue;
        }
        if !is_emoji(g) {
            return None;
        }
        n += 1;
        if n > MAX {
            return None;
        }
    }
    Some(n)
}

/// One grapheme that shows as an emoji picture. A character that is text
/// unless asked (©, ™, ☺) counts only with the emoji variation selector
/// or as part of a longer sequence; digits and # only as keycaps.
fn is_emoji(g: &str) -> bool {
    if emojis::get(g).is_none() {
        return false;
    }
    let mut chars = g.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => presented(c),
        _ => true,
    }
}

/// Characters drawn as emoji by default (Unicode's `Emoji_Presentation`):
/// all of them from U+1F000 on, and these in the Basic Multilingual Plane.
fn presented(c: char) -> bool {
    const BMP: &[(char, char)] = &[
        ('\u{231A}', '\u{231B}'),
        ('\u{23E9}', '\u{23EC}'),
        ('\u{23F0}', '\u{23F0}'),
        ('\u{23F3}', '\u{23F3}'),
        ('\u{25FD}', '\u{25FE}'),
        ('\u{2614}', '\u{2615}'),
        ('\u{2648}', '\u{2653}'),
        ('\u{267F}', '\u{267F}'),
        ('\u{2693}', '\u{2693}'),
        ('\u{26A1}', '\u{26A1}'),
        ('\u{26AA}', '\u{26AB}'),
        ('\u{26BD}', '\u{26BE}'),
        ('\u{26C4}', '\u{26C5}'),
        ('\u{26CE}', '\u{26CE}'),
        ('\u{26D4}', '\u{26D4}'),
        ('\u{26EA}', '\u{26EA}'),
        ('\u{26F2}', '\u{26F3}'),
        ('\u{26F5}', '\u{26F5}'),
        ('\u{26FA}', '\u{26FA}'),
        ('\u{26FD}', '\u{26FD}'),
        ('\u{2705}', '\u{2705}'),
        ('\u{270A}', '\u{270B}'),
        ('\u{2728}', '\u{2728}'),
        ('\u{274C}', '\u{274C}'),
        ('\u{274E}', '\u{274E}'),
        ('\u{2753}', '\u{2755}'),
        ('\u{2757}', '\u{2757}'),
        ('\u{2795}', '\u{2797}'),
        ('\u{27B0}', '\u{27B0}'),
        ('\u{27BF}', '\u{27BF}'),
        ('\u{2B1B}', '\u{2B1C}'),
        ('\u{2B50}', '\u{2B50}'),
        ('\u{2B55}', '\u{2B55}'),
    ];
    c >= '\u{1F000}' || BMP.iter().any(|(a, b)| (*a..=*b).contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_emoji_and_spaces_are_jumbo() {
        assert!(emoji_only("😀", None));
        assert!(emoji_only("😀🎉 👍", None));
        assert!(emoji_only("  🔥\n🔥 ", None));
        assert!(!emoji_only("hi 😀", None));
        assert!(!emoji_only("😀!", None));
        assert!(!emoji_only("", None));
        assert!(!emoji_only("   ", None));
    }

    #[test]
    fn sequences_count_once_each() {
        // Skin tone, ZWJ family, flag, keycap, rainbow flag.
        let five = "👍🏽👨‍👩‍👧🇫🇷1️⃣🏳️‍🌈";
        assert_eq!(count(five), Some(5));
        assert!(emoji_only(five, None));
        // The heart is text unless it carries the emoji selector.
        assert!(emoji_only("❤\u{fe0f}", None));
        assert!(!emoji_only("❤", None));
    }

    #[test]
    fn text_symbols_digits_and_hash_are_not_emoji() {
        assert!(!emoji_only("1", None));
        assert!(!emoji_only("#", None));
        assert!(!emoji_only("©", None), "text unless asked");
        assert!(emoji_only("©\u{fe0f}", None));
        assert!(!emoji_only("™ ™", None));
        assert!(emoji_only("⭐⚡", None), "emoji by default");
    }

    #[test]
    fn up_to_twenty_seven() {
        assert!(emoji_only(&"😀".repeat(MAX), None));
        assert!(!emoji_only(&"😀".repeat(MAX + 1), None));
        assert!(!emoji_only(&"😀 ".repeat(MAX + 1), None));
    }

    #[test]
    fn custom_emoji_count_and_any_other_formatting_does_not() {
        let html = |h: &str| Doc::html(h);
        let blob = r#"<img data-mx-emoticon src="mxc://h/blob" alt=":blob:">"#;
        let cat = r#"<img data-mx-emoticon src="mxc://h/cat" alt=":cat:">"#;
        assert!(emoji_only(":blob:", Some(&html(blob))));
        assert!(emoji_only("", Some(&html(&format!("{blob} 🎉 {cat}")))));
        assert!(emoji_only("", Some(&html(&format!("{blob}{blob}")))));
        assert!(!emoji_only("", Some(&html(&format!("hi {blob}")))));
        assert!(!emoji_only("", Some(&html("<b>😀</b>"))));
        assert!(!emoji_only("", Some(&html("<p>😀</p><p>😀</p>"))));
        assert!(!emoji_only("", Some(&html(&blob.repeat(MAX + 1)))));
        assert!(emoji_only("", Some(&html(&blob.repeat(MAX)))));
        // A hidden spoiler isn't shown as a picture.
        let hidden = format!("<span data-mx-spoiler>{blob}</span>");
        assert!(!emoji_only("", Some(&html(&hidden))));
    }
}
