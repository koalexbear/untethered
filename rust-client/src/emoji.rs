//! Emoji by shortcode, with GitHub's gemoji names (`:tada:`) from the
//! `emojis` crate, which matrix-sdk-ui already brings in. Used by `:` in the
//! composer and by the emoji pickers' search.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub emoji: &'static str,
    /// The shortcode that matched, without colons.
    pub shortcode: &'static str,
}

/// Emoji matching `query`, best first: the exact shortcode, shortcodes
/// starting with it, then a word of a shortcode or of the emoji's name
/// starting with it, then shortcodes containing it. Ties keep Unicode's
/// order, which puts the common faces and hands first.
pub fn search(query: &str, limit: usize) -> Vec<Found> {
    let query = query.trim().to_lowercase().replace(' ', "_");
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, usize, Found)> = Vec::new();
    for (order, e) in emojis::iter().enumerate() {
        let best = e
            .shortcodes()
            .filter_map(|code| rank(code, e.name(), &query).map(|r| (r, code)))
            .min_by_key(|(r, _)| *r);
        if let Some((r, shortcode)) = best {
            hits.push((
                r,
                order,
                Found {
                    emoji: e.as_str(),
                    shortcode,
                },
            ));
        }
    }
    hits.sort_by_key(|(r, order, _)| (*r, *order));
    hits.into_iter().take(limit).map(|(.., f)| f).collect()
}

fn rank(code: &str, name: &str, query: &str) -> Option<u8> {
    if code == query {
        Some(0)
    } else if code.starts_with(query) {
        Some(1)
    } else if code.split('_').any(|w| w.starts_with(query))
        || name
            .split([' ', '-'])
            .any(|w| w.to_lowercase().starts_with(query))
    {
        Some(2)
    } else if code.contains(query) {
        Some(3)
    } else {
        None
    }
}

/// The first emoji in Unicode's order, the smileys, for a picker with no
/// search yet.
pub fn common(limit: usize) -> Vec<Found> {
    emojis::iter()
        .filter_map(|e| {
            Some(Found {
                emoji: e.as_str(),
                shortcode: e.shortcode()?,
            })
        })
        .take(limit)
        .collect()
}

/// The emoji a complete shortcode names, as typing `:tada:` gives 🎉.
pub fn by_shortcode(code: &str) -> Option<&'static str> {
    emojis::get_by_shortcode(&code.to_lowercase()).map(|e| e.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_shortcodes_come_first_then_prefixes() {
        let found = search("tada", 5);
        assert_eq!(found[0].emoji, "🎉");
        let smi = search("smi", 20);
        assert!(smi.iter().all(|f| f.shortcode.contains("smi")
            || emojis::get(f.emoji).is_some_and(|e| e.name().contains("smi"))));
        assert!(
            smi.iter().any(|f| f.shortcode == "smile"),
            "smile is among the first matches"
        );
        assert!(search("thumbsup", 1)[0].emoji == "👍");
    }

    #[test]
    fn words_inside_shortcodes_and_names_match() {
        assert!(search("heart", 10).iter().any(|f| f.emoji == "❤️"));
        assert!(search("eyes", 3).iter().any(|f| f.emoji == "👀"));
    }

    #[test]
    fn nothing_for_empty_or_unknown_queries() {
        assert!(search("", 5).is_empty());
        assert!(search("zzzqqq", 5).is_empty());
        assert_eq!(search("smile", 3).len(), 3);
    }

    #[test]
    fn complete_shortcodes_resolve() {
        assert_eq!(by_shortcode("tada"), Some("🎉"));
        assert_eq!(by_shortcode("Tada"), Some("🎉"));
        assert_eq!(by_shortcode("not_an_emoji"), None);
    }
}
