//! Syntax highlighting for code blocks that name their language, as on
//! Discord (```` ```rust ````). Iced's highlighter (syntect with the
//! syntaxes `bat` ships) colours each block once, when its message is read
//! (rich.rs); the view only draws the coloured ranges. A block without a
//! language, or in one it doesn't know, stays plain.
use iced_highlighter::{Settings, Stream, Theme};
use std::ops::Range;

/// Blocks longer than this stay plain: highlighting runs on the UI thread.
const MAX_BYTES: usize = 16 * 1024;

/// A coloured byte range of a code block's text.
pub type Painted = (Range<usize>, [u8; 3]);

/// The coloured ranges of `code` in `language`, in order and apart; text
/// between them keeps the block's own colour. Empty when there's nothing to
/// colour.
pub fn paint(language: &str, code: &str) -> Vec<Painted> {
    let language = language.trim().to_ascii_lowercase();
    if language.is_empty() || code.len() > MAX_BYTES {
        return Vec::new();
    }
    let token = alias(&language);
    let mut stream = Stream::new(&Settings {
        theme: Theme::Base16Ocean,
        token: token.to_owned(),
    });
    let mut out: Vec<Painted> = Vec::new();
    let mut start = 0;
    for line in code.split('\n') {
        for (range, highlight) in stream.highlight_line(line) {
            let Some(color) = highlight.color() else {
                continue;
            };
            let [r, g, b, _] = color.into_rgba8();
            let rgb = [r, g, b];
            if rgb == FOREGROUND {
                continue;
            }
            let range = start + range.start..start + range.end;
            match out.last_mut() {
                Some((last, c)) if *c == rgb && last.end == range.start => last.end = range.end,
                _ => out.push((range, rgb)),
            }
        }
        stream.commit();
        start += line.len() + 1;
    }
    out
}

/// The theme's plain text colour, left to the block's own.
const FOREGROUND: [u8; 3] = [0xc0, 0xc5, 0xce];

/// Names people write that the syntaxes know by another. Names and file
/// extensions (`rust`, `rs`, `JavaScript`) find their syntax as they are.
fn alias(language: &str) -> &str {
    match language {
        "csharp" => "cs",
        "shell" | "zsh" => "sh",
        "golang" => "go",
        "node" => "js",
        "python3" => "py",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted<'a>(code: &'a str, spans: &[Painted]) -> Vec<&'a str> {
        spans.iter().map(|(r, _)| &code[r.clone()]).collect()
    }

    #[test]
    fn a_named_language_gets_colours() {
        let code = "fn main() {\n    let s = \"hi\"; // greet\n}";
        let spans = paint("rust", code);
        let words = painted(code, &spans);
        assert!(words.contains(&"fn"), "{words:?}");
        assert!(
            words.iter().any(|w| w.contains("\"hi\"") || *w == "hi"),
            "{words:?}"
        );
        assert!(words.iter().any(|w| w.contains("// greet")), "{words:?}");
        // In order and apart, inside the text.
        for pair in spans.windows(2) {
            assert!(pair[0].0.end <= pair[1].0.start);
        }
        assert!(spans.iter().all(|(r, _)| r.end <= code.len()));
        // The keyword and the string differ.
        let colour = |w: &str| {
            spans
                .iter()
                .find(|(r, _)| code[r.clone()].contains(w))
                .map(|s| s.1)
        };
        assert!(colour("fn").is_some());
        assert_ne!(colour("fn"), colour("hi"));
    }

    #[test]
    fn aliases_and_case_reach_the_same_syntax() {
        let code = "const x = 1;";
        assert_eq!(paint("JavaScript", code), paint("js", code));
        assert!(!paint("ts", "let x: number = 1;").is_empty());
        assert!(!paint("python", "def f():\n    return 1").is_empty());
    }

    #[test]
    fn no_language_unknown_ones_and_huge_blocks_stay_plain() {
        assert!(paint("", "fn main() {}").is_empty());
        assert!(paint("no-such-language", "fn main() {}").is_empty());
        let huge = "let x = 1;\n".repeat(MAX_BYTES / 10);
        assert!(paint("rust", &huge).is_empty());
    }
}
