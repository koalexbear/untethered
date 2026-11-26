//! Rich text in received messages, as blocks of styled runs for the timeline.
//!
//! A message's `formatted_body` (`org.matrix.custom.html`) goes through
//! ruma's sanitiser in the mode matrix-sdk-ui uses (the Matrix spec's
//! allow-list plus `matrix:` links), with the reply fallback and the contents
//! of script-like elements removed as Element does. The walk below then only
//! gives meaning to the elements Element styles; anything else keeps just its
//! text.
//!
//! A message with only a `body` is read as the Markdown our own composer
//! sends (CommonMark with strikethrough, newlines kept), restricted to the
//! same constructs. Raw HTML in a plain body stays literal text: a client
//! that sends no `formatted_body` never meant any markup. Element would show
//! such a body unformatted; the difference is that `**this**` and code
//! fences look as their sender intended, as on Discord.
//!
//! `||spoiler||` (Discord's syntax) hides text in both kinds of message,
//! next to Element's `data-mx-spoiler`.
//!
//! Search snippets, reply quotes, notifications and the mentions inbox keep
//! using the plain `body`.
use matrix_sdk::ruma::html::{Html, NodeData, NodeRef, SanitizerConfig};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// A message's content, ready for the timeline.
#[derive(Debug, PartialEq)]
pub struct Doc {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Vec<Run>),
    /// Level 1 to 6.
    Heading(u8, Vec<Run>),
    Code {
        language: Option<String>,
        text: String,
        /// A code block inside a spoiler stays hidden with it.
        spoiler: Option<u16>,
    },
    Quote(Vec<Block>),
    /// `start` is None for a bulleted list.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Rule,
}

/// Text with one style. A paragraph's runs may contain `\n` line breaks.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: bool,
    pub code: bool,
    /// `data-mx-color`, `data-mx-bg-color`.
    pub color: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub link: Option<Link>,
    /// Hidden until revealed; numbered from 0 within the message.
    pub spoiler: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// A page or address the system can open (http, https, mailto).
    Web(String),
    /// A matrix.to link to a user, shown as a mention.
    User(String),
}

/// The grey of a spoiler's reason, as for secondary labels.
const REASON: [u8; 3] = [0x94, 0x9b, 0xa4];
/// Nested quotes and lists beyond this stay at this depth, so a hostile
/// message can't make the view recurse without bound.
const MAX_NESTING: usize = 12;

impl Doc {
    /// `formatted` is an `org.matrix.custom.html` body. None when the message
    /// reads the same as its plain body, which the view then shows as text.
    pub fn message(formatted: Option<&str>, body: &str) -> Option<Self> {
        let doc = match formatted {
            Some(html) => Self::html(html),
            None => Self::markdown(body),
        };
        (!doc.is_plain(body)).then_some(doc)
    }

    pub fn html(html: &str) -> Self {
        let tree = Html::parse(html);
        // Element drops these elements' contents (sanitize-html's
        // nonTextTags); ruma alone would keep their text.
        let config = SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(["script", "style", "textarea", "option", "noscript"]);
        tree.sanitize_with(&config);
        let mut b = Builder::default();
        for node in tree.children() {
            walk(&mut b, &node, &Style::default());
        }
        b.finish()
    }

    pub fn markdown(body: &str) -> Self {
        let mut b = Builder::default();
        let mut styles = vec![Style::default()];
        let mut code: Option<(Option<String>, String, Option<u16>)> = None;
        for event in Parser::new_ext(body, Options::ENABLE_STRIKETHROUGH) {
            let style = styles.last().cloned().unwrap_or_default();
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => b.flush(),
                    Tag::Heading { level, .. } => {
                        b.flush();
                        b.heading = Some(level as u8);
                    }
                    Tag::BlockQuote(_) => b.open(Frame::Quote(Vec::new())),
                    Tag::List(start) => b.open(Frame::List(start, Vec::new())),
                    Tag::Item => b.open(Frame::Item(Vec::new())),
                    Tag::CodeBlock(kind) => {
                        let language = match kind {
                            CodeBlockKind::Fenced(info) => info
                                .split_whitespace()
                                .next()
                                .map(str::to_owned)
                                .filter(|l| !l.is_empty()),
                            CodeBlockKind::Indented => None,
                        };
                        code = Some((language, String::new(), b.spoiler_id(&style)));
                    }
                    Tag::Emphasis => styles.push(Style {
                        italic: true,
                        ..style
                    }),
                    Tag::Strong => styles.push(Style {
                        bold: true,
                        ..style
                    }),
                    Tag::Strikethrough => styles.push(Style {
                        strike: true,
                        ..style
                    }),
                    // An image is shown as its description, linking to it.
                    Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                        let link = link(&dest_url).or(style.link.clone());
                        styles.push(Style { link, ..style });
                    }
                    // Other blocks (HTML blocks, footnotes, the extensions
                    // left off) only contribute their text.
                    _ => {}
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph | TagEnd::Heading(_) => b.flush(),
                    TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => b.close(),
                    TagEnd::CodeBlock => {
                        if let Some((language, text, spoiler)) = code.take() {
                            b.code_block(language, text, spoiler);
                        }
                    }
                    TagEnd::Emphasis
                    | TagEnd::Strong
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Image
                        if styles.len() > 1 =>
                    {
                        styles.pop();
                    }
                    _ => {}
                },
                Event::Text(text) => match &mut code {
                    Some((_, block, _)) => block.push_str(&text),
                    None => b.text(&text, &style, false),
                },
                Event::Code(text) => b.text(
                    &text,
                    &Style {
                        code: true,
                        ..style
                    },
                    false,
                ),
                // Literal: see the module comment.
                Event::Html(text) | Event::InlineHtml(text) => b.text(&text, &style, false),
                Event::SoftBreak | Event::HardBreak => b.line_break(&style),
                Event::Rule => b.rule(),
                Event::InlineMath(text)
                | Event::DisplayMath(text)
                | Event::FootnoteReference(text) => b.text(&text, &style, false),
                Event::TaskListMarker(_) => {}
            }
        }
        b.finish()
    }

    /// One unstyled paragraph that says exactly what the plain body says.
    fn is_plain(&self, body: &str) -> bool {
        let [Block::Paragraph(runs)] = self.blocks.as_slice() else {
            return self.blocks.is_empty() && body.trim().is_empty();
        };
        runs.iter().all(|r| r.style == Style::default())
            && runs.iter().map(|r| r.text.as_str()).collect::<String>() == body.trim()
    }

    /// Text after the content, such as "Sending…", unstyled.
    pub fn note(&mut self, text: &str) {
        let run = Run {
            text: text.to_owned(),
            style: Style::default(),
        };
        match self.blocks.last_mut() {
            Some(Block::Paragraph(runs) | Block::Heading(_, runs)) => runs.push(run),
            _ => self.blocks.push(Block::Paragraph(vec![run])),
        }
    }
}

/// Open a message's link in the browser or mail client.
pub fn open(href: &str) {
    // Checked again: the address comes back from the view as a message.
    let Some(Link::Web(href)) = link(href) else {
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(url) = url::Url::parse(&href) {
        crate::engine::browser::system()(url);
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        let _ =
            window.open_with_url_and_target_and_features(&href, "_blank", "noopener,noreferrer");
    }
}

/// Where a link goes, if it is one this client follows.
fn link(href: &str) -> Option<Link> {
    let url = url::Url::parse(href.trim()).ok()?;
    if url.host_str() == Some("matrix.to")
        && let Some(user) = url
            .fragment()
            .and_then(|f| f.strip_prefix("/@"))
            .map(|f| f.split(['?', '/']).next().unwrap_or(f))
    {
        let user = percent_decode(user);
        if user.contains(':') {
            return Some(Link::User(format!("@{user}")));
        }
    }
    // Never hand a sender-chosen file:, app: or custom-scheme URL to the OS.
    matches!(url.scheme(), "https" | "http" | "mailto").then(|| Link::Web(url.into()))
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map_or_else(|| s.to_owned(), |(_, v)| v.into_owned())
}

/// `#rrggbb`, as `data-mx-color` carries it.
fn color(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn walk(b: &mut Builder, node: &NodeRef, style: &Style) {
    match node.data() {
        NodeData::Text(text) => {
            let text = text.borrow();
            b.text(&text, style, true);
        }
        NodeData::Element(e) => {
            let name = e.name.local.as_ref();
            let attrs = e.attrs.borrow();
            let attr = |key: &str| {
                attrs
                    .iter()
                    .find(|a| a.name.local.as_ref() == key)
                    .map(|a| a.value.to_string())
            };
            let mut s = style.clone();
            let children = |b: &mut Builder, s: &Style| {
                for child in node.children() {
                    walk(b, &child, s);
                }
            };
            match name {
                "b" | "strong" => {
                    s.bold = true;
                    children(b, &s);
                }
                "i" | "em" => {
                    s.italic = true;
                    children(b, &s);
                }
                "s" | "del" | "strike" => {
                    s.strike = true;
                    children(b, &s);
                }
                "u" | "ins" => {
                    s.underline = true;
                    children(b, &s);
                }
                "code" => {
                    s.code = true;
                    children(b, &s);
                }
                "a" => {
                    if let Some(link) = attr("href").as_deref().and_then(link) {
                        s.link = Some(link);
                    }
                    children(b, &s);
                }
                "span" | "font" => {
                    if let Some(c) = attr("data-mx-color").as_deref().and_then(color) {
                        s.color = Some(c);
                    }
                    if let Some(c) = attr("data-mx-bg-color").as_deref().and_then(color) {
                        s.background = Some(c);
                    }
                    if let Some(reason) = attr("data-mx-spoiler") {
                        if !reason.trim().is_empty() {
                            let label = Style {
                                color: Some(REASON),
                                ..s.clone()
                            };
                            b.text(&format!("({}) ", reason.trim()), &label, false);
                        }
                        if s.spoiler.is_none() {
                            s.spoiler = Some(b.new_spoiler());
                        }
                    }
                    children(b, &s);
                }
                "br" => b.line_break(style),
                "hr" => b.rule(),
                "img" => {
                    // Element shows only emoticons from the homeserver
                    // inline; their description reads the same.
                    if let Some(alt) = attr("alt").or_else(|| attr("title")) {
                        b.text(&alt, style, true);
                    }
                }
                "p" | "div" | "details" | "summary" | "caption" | "table" | "thead" | "tbody"
                | "tfoot" => {
                    b.flush();
                    children(b, &s);
                    b.flush();
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    b.flush();
                    b.heading = name[1..].parse().ok();
                    children(b, &s);
                    b.flush();
                }
                "tr" => {
                    b.flush();
                    let mut first = true;
                    for cell in node.children() {
                        if cell.as_element().is_some() {
                            if !first {
                                b.text(" | ", style, false);
                            }
                            first = false;
                        }
                        walk(b, &cell, style);
                    }
                    b.flush();
                }
                "th" => {
                    s.bold = true;
                    children(b, &s);
                }
                "blockquote" => {
                    b.open(Frame::Quote(Vec::new()));
                    children(b, &s);
                    b.close();
                }
                "ul" | "ol" => {
                    let start = (name == "ol").then(|| {
                        attr("start")
                            .and_then(|n| n.trim().parse().ok())
                            .unwrap_or(1)
                    });
                    b.open(Frame::List(start, Vec::new()));
                    children(b, &s);
                    b.close();
                }
                "li" => {
                    b.open(Frame::Item(Vec::new()));
                    children(b, &s);
                    b.close();
                }
                "pre" => {
                    // Element marks the language as `language-rust` on the
                    // inner code element.
                    let language = node.children().find_map(|c| {
                        let e = c.as_element()?;
                        let attrs = e.attrs.borrow();
                        let class = attrs.iter().find(|a| a.name.local.as_ref() == "class")?;
                        class
                            .value
                            .split_whitespace()
                            .find_map(|c| c.strip_prefix("language-"))
                            .filter(|l| !l.is_empty())
                            .map(str::to_owned)
                    });
                    let mut text = String::new();
                    raw_text(node, &mut text);
                    let spoiler = b.spoiler_id(style);
                    b.code_block(language, text, spoiler);
                }
                // The sanitiser has already unwrapped elements outside the
                // allow-list; sup, sub, mx-maths and the like keep their text.
                _ => children(b, &s),
            }
        }
        NodeData::Document | NodeData::Other => {}
    }
}

/// The text of a `<pre>`, whitespace and line breaks as written.
fn raw_text(node: &NodeRef, out: &mut String) {
    for child in node.children() {
        match child.data() {
            NodeData::Text(t) => out.push_str(&t.borrow()),
            NodeData::Element(e) if e.name.local.as_ref() == "br" => out.push('\n'),
            NodeData::Element(_) => raw_text(&child, out),
            _ => {}
        }
    }
}

enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List(Option<u64>, Vec<Vec<Block>>),
    Item(Vec<Block>),
}

/// Collects blocks from either parser. Text goes into the current paragraph
/// until a block boundary flushes it.
struct Builder {
    frames: Vec<Frame>,
    /// Opened frames that [`MAX_NESTING`] turned away; their closes too.
    flattened: usize,
    line: Vec<Run>,
    heading: Option<u8>,
    /// An unmatched `||` in the current paragraph: the index of its run,
    /// kept as literal text until the closing one turns the runs after it
    /// into a spoiler.
    open_spoiler: Option<(usize, u16)>,
    spoilers: u16,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            frames: vec![Frame::Root(Vec::new())],
            flattened: 0,
            line: Vec::new(),
            heading: None,
            open_spoiler: None,
            spoilers: 0,
        }
    }
}

impl Builder {
    fn new_spoiler(&mut self) -> u16 {
        self.spoilers = self.spoilers.saturating_add(1);
        self.spoilers - 1
    }

    /// The spoiler new content belongs to: its style's, or an open `||`.
    fn spoiler_id(&self, style: &Style) -> Option<u16> {
        style.spoiler.or(self.open_spoiler.map(|(_, id)| id))
    }

    fn finish(mut self) -> Doc {
        self.flush();
        while self.frames.len() > 1 {
            self.close();
        }
        match self.frames.pop() {
            Some(Frame::Root(blocks)) => Doc { blocks },
            _ => Doc { blocks: Vec::new() },
        }
    }

    fn open(&mut self, frame: Frame) {
        self.flush();
        if self.frames.len() > MAX_NESTING {
            self.flattened += 1;
            return;
        }
        self.frames.push(frame);
    }

    fn close(&mut self) {
        self.flush();
        if self.flattened > 0 {
            self.flattened -= 1;
            return;
        }
        if self.frames.len() < 2 {
            return;
        }
        match self.frames.pop() {
            Some(Frame::Quote(blocks)) if !blocks.is_empty() => self.push(Block::Quote(blocks)),
            Some(Frame::List(start, items)) if !items.is_empty() => {
                self.push(Block::List { start, items })
            }
            Some(Frame::Item(blocks)) => match self.frames.last_mut() {
                Some(Frame::List(_, items)) => items.push(blocks),
                // A list item outside a list: just its content.
                _ => {
                    for block in blocks {
                        self.push(block);
                    }
                }
            },
            _ => {}
        }
    }

    fn push(&mut self, block: Block) {
        match self.frames.last_mut() {
            Some(Frame::Root(blocks) | Frame::Quote(blocks) | Frame::Item(blocks)) => {
                blocks.push(block)
            }
            // Content directly inside a list (not in an item) is an item.
            Some(Frame::List(_, items)) => items.push(vec![block]),
            None => {}
        }
    }

    /// End the current paragraph.
    fn flush(&mut self) {
        if let Some((marker, id)) = self.open_spoiler.take() {
            // Never closed: the `||` was just text.
            for run in &mut self.line[marker..] {
                if run.style.spoiler == Some(id) {
                    run.style.spoiler = None;
                }
            }
            if id + 1 == self.spoilers {
                self.spoilers = id;
            }
        }
        let mut runs = std::mem::take(&mut self.line);
        let heading = self.heading.take();
        if let Some(first) = runs.first_mut() {
            first.text = first.text.trim_start().to_owned();
        }
        if let Some(last) = runs.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        runs.retain(|r| !r.text.is_empty());
        // Merge what the spoiler markers and trimming left side by side.
        let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
        for run in runs {
            match merged.last_mut() {
                Some(last) if last.style == run.style => last.text.push_str(&run.text),
                _ => merged.push(run),
            }
        }
        if merged.iter().all(|r| r.text.trim().is_empty()) {
            return;
        }
        self.push(match heading {
            Some(level) => Block::Heading(level, merged),
            None => Block::Paragraph(merged),
        });
    }

    fn rule(&mut self) {
        self.flush();
        self.push(Block::Rule);
    }

    fn code_block(&mut self, language: Option<String>, mut text: String, spoiler: Option<u16>) {
        self.flush();
        if text.ends_with('\n') {
            text.pop();
        }
        self.push(Block::Code {
            language,
            text,
            spoiler,
        });
    }

    fn line_break(&mut self, style: &Style) {
        self.run("\n", style);
    }

    /// `collapse`: HTML text, where any whitespace is one space.
    fn text(&mut self, text: &str, style: &Style, collapse: bool) {
        let collapsed;
        let text = if collapse {
            let mut out = String::with_capacity(text.len());
            let mut space = self
                .line
                .last()
                .and_then(|r| r.text.chars().last())
                .is_none_or(char::is_whitespace);
            for c in text.chars() {
                if c.is_ascii_whitespace() {
                    if !space {
                        out.push(' ');
                    }
                    space = true;
                } else {
                    out.push(c);
                    space = false;
                }
            }
            collapsed = out;
            collapsed.as_str()
        } else {
            text
        };
        if style.code || style.spoiler.is_some() {
            return self.linked(text, style);
        }
        let mut rest = text;
        while let Some(at) = rest.find("||") {
            self.linked(&rest[..at], style);
            self.spoiler_marker(style);
            rest = &rest[at + 2..];
        }
        self.linked(rest, style);
    }

    fn spoiler_marker(&mut self, style: &Style) {
        match self.open_spoiler {
            // `||||` hides nothing: both stay text.
            Some((marker, _)) if marker + 1 == self.line.len() => {
                self.open_spoiler = None;
                self.spoilers -= 1;
                self.line.push(Run {
                    text: "||".into(),
                    style: style.clone(),
                });
            }
            Some((marker, _)) => {
                self.line.remove(marker);
                self.open_spoiler = None;
            }
            None => {
                let id = self.new_spoiler();
                // Pushed apart from its neighbours, so its index holds.
                self.line.push(Run {
                    text: "||".into(),
                    style: style.clone(),
                });
                self.open_spoiler = Some((self.line.len() - 1, id));
            }
        }
    }

    /// Text with bare web addresses made into links, as Element does.
    fn linked(&mut self, text: &str, style: &Style) {
        if style.code || style.link.is_some() {
            return self.run(text, style);
        }
        let mut rest = text;
        while let Some((start, end)) = find_url(rest) {
            self.run(&rest[..start], style);
            let url = &rest[start..end];
            let href = if url.starts_with("www.") {
                format!("https://{url}")
            } else {
                url.to_owned()
            };
            let link = Style {
                link: link(&href),
                ..style.clone()
            };
            self.run(url, &link);
            rest = &rest[end..];
        }
        self.run(rest, style);
    }

    fn run(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let mut style = style.clone();
        if style.spoiler.is_none() {
            style.spoiler = self.open_spoiler.map(|(_, id)| id);
        }
        let after_marker = self.open_spoiler.map(|(i, _)| i + 1) == Some(self.line.len());
        match self.line.last_mut() {
            Some(last) if last.style == style && !after_marker => last.text.push_str(text),
            _ => self.line.push(Run {
                text: text.to_owned(),
                style,
            }),
        }
    }
}

/// The first bare `http://`, `https://` or `www.` address in `text`, as a
/// byte range, without trailing punctuation or an unbalanced bracket.
fn find_url(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let start = ["https://", "http://", "www."]
            .iter()
            .filter_map(|p| lower[from..].find(p).map(|i| from + i))
            .min()?;
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '/' && c != '.');
        let mut end = text[start..]
            .find(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"'))
            .map_or(text.len(), |i| start + i);
        loop {
            let url = &text[start..end];
            let Some(last) = url.chars().next_back() else {
                break;
            };
            let unbalanced = |open: char, close: char| {
                last == close && url.matches(close).count() > url.matches(open).count()
            };
            if matches!(
                last,
                '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*' | '_' | '~'
            ) || unbalanced('(', ')')
                || unbalanced('[', ']')
                || unbalanced('{', '}')
            {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        let url = &lower[start..end];
        let host = url
            .split_once("://")
            .map_or(url, |(_, rest)| rest)
            .trim_start_matches("www.");
        if boundary && host.chars().next().is_some_and(char::is_alphanumeric) {
            return Some((start, end));
        }
        from = start + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, style: Style) -> Run {
        Run {
            text: text.into(),
            style,
        }
    }
    fn plain(text: &str) -> Run {
        run(text, Style::default())
    }
    fn paragraph(runs: Vec<Run>) -> Block {
        Block::Paragraph(runs)
    }
    fn web(url: &str) -> Style {
        Style {
            link: Some(Link::Web(url.into())),
            ..Style::default()
        }
    }
    fn hidden(id: u16) -> Style {
        Style {
            spoiler: Some(id),
            ..Style::default()
        }
    }

    #[test]
    fn nested_formatting_keeps_every_style() {
        let doc = Doc::html("<b>bold <i>both <del>all</del></i></b> <code>x &lt; y</code>");
        let bold = Style {
            bold: true,
            ..Style::default()
        };
        let both = Style {
            italic: true,
            ..bold.clone()
        };
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                run("bold ", bold),
                run("both ", both.clone()),
                run(
                    "all",
                    Style {
                        strike: true,
                        ..both
                    }
                ),
                plain(" "),
                run(
                    "x < y",
                    Style {
                        code: true,
                        ..Style::default()
                    }
                ),
            ])]
        );
    }

    #[test]
    fn unsafe_markup_is_dropped() {
        let doc = Doc::html(concat!(
            "<script>alert(1)</script><style>p{}</style>",
            "<iframe src=\"https://evil\"></iframe>",
            "<a href=\"javascript:alert(1)\">click</a> ",
            "<a href=\"file:///etc/passwd\">file</a> ",
            "<img src=\"https://tracker/p.gif\" onerror=\"alert(1)\">",
            "<div onclick=\"x()\">safe</div>",
        ));
        assert_eq!(
            doc.blocks,
            [
                paragraph(vec![plain("click file")]),
                paragraph(vec![plain("safe")])
            ]
        );
    }

    #[test]
    fn reply_fallback_is_removed() {
        let doc = Doc::html(concat!(
            "<mx-reply><blockquote><a href=\"https://matrix.to/#/!r/$e\">In reply to</a> ",
            "<a href=\"https://matrix.to/#/@a:x\">@a:x</a><br>quoted</blockquote></mx-reply>",
            "the <b>answer</b>"
        ));
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                plain("the "),
                run(
                    "answer",
                    Style {
                        bold: true,
                        ..Style::default()
                    }
                )
            ])]
        );
    }

    #[test]
    fn spoilers_from_element_and_discord_syntax() {
        let doc = Doc::html(
            "a <span data-mx-spoiler>one</span> b <span data-mx-spoiler=\"plot\">two</span>",
        );
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                plain("a "),
                run("one", hidden(0)),
                plain(" b "),
                run(
                    "(plot) ",
                    Style {
                        color: Some(REASON),
                        ..Style::default()
                    }
                ),
                run("two", hidden(1)),
            ])]
        );
        // Across styles, not inside code, and an unmatched pair stays text.
        let doc = Doc::markdown("||**big** news|| `a || b` ||open\n\n||||");
        let bold_hidden = Style {
            bold: true,
            ..hidden(0)
        };
        assert_eq!(
            doc.blocks,
            [
                paragraph(vec![
                    run("big", bold_hidden),
                    run(" news", hidden(0)),
                    plain(" "),
                    run(
                        "a || b",
                        Style {
                            code: true,
                            ..Style::default()
                        }
                    ),
                    plain(" ||open"),
                ]),
                paragraph(vec![plain("||||")]),
            ]
        );
        // A plain body with a spoiler is rich; one without stays plain.
        assert!(Doc::message(None, "the end: ||they win||").is_some());
        assert!(Doc::message(None, "a | b || c").is_none());
    }

    #[test]
    fn code_blocks_keep_their_text_and_language() {
        let doc = Doc::html(concat!(
            "<p>before</p>\n<pre><code class=\"language-rust\">fn main() {\n    ",
            "println!(\"&lt;hi&gt;\");\n}\n</code></pre>\n<p>after</p>",
        ));
        assert_eq!(
            doc.blocks,
            [
                paragraph(vec![plain("before")]),
                Block::Code {
                    language: Some("rust".into()),
                    text: "fn main() {\n    println!(\"<hi>\");\n}".into(),
                    spoiler: None,
                },
                paragraph(vec![plain("after")]),
            ]
        );
        let doc = Doc::markdown("```py\nprint('**not bold**')\n```");
        assert_eq!(
            doc.blocks,
            [Block::Code {
                language: Some("py".into()),
                text: "print('**not bold**')".into(),
                spoiler: None,
            }]
        );
    }

    #[test]
    fn links_bare_addresses_and_mentions() {
        let doc = Doc::html(concat!(
            "<a href=\"https://example.org/a\">site</a>, see https://example.org/b(c). ",
            "or (www.example.net) <a href=\"https://matrix.to/#/@mira:example.org\">Mira</a>",
        ));
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                run("site", web("https://example.org/a")),
                plain(", see "),
                run("https://example.org/b(c)", web("https://example.org/b(c)")),
                plain(". or ("),
                run("www.example.net", web("https://www.example.net/")),
                plain(") "),
                run(
                    "Mira",
                    Style {
                        link: Some(Link::User("@mira:example.org".into())),
                        ..Style::default()
                    }
                ),
            ])]
        );
        // Not inside code, nor a word that merely contains the prefix.
        let doc = Doc::markdown("`https://x.org` nothttps://y.org [z](mailto:z@x.org)");
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                run(
                    "https://x.org",
                    Style {
                        code: true,
                        ..Style::default()
                    }
                ),
                plain(" nothttps://y.org "),
                run("z", web("mailto:z@x.org")),
            ])]
        );
    }

    #[test]
    fn quotes_lists_and_headings() {
        let doc = Doc::html(concat!(
            "<h2>Plan</h2><blockquote><p>quoted</p></blockquote>",
            "<ol start=\"3\"><li>three</li><li>four<ul><li>nested</li></ul></li></ol><hr>",
        ));
        assert_eq!(
            doc.blocks,
            [
                Block::Heading(2, vec![plain("Plan")]),
                Block::Quote(vec![paragraph(vec![plain("quoted")])]),
                Block::List {
                    start: Some(3),
                    items: vec![
                        vec![paragraph(vec![plain("three")])],
                        vec![
                            paragraph(vec![plain("four")]),
                            Block::List {
                                start: None,
                                items: vec![vec![paragraph(vec![plain("nested")])]],
                            },
                        ],
                    ],
                },
                Block::Rule,
            ]
        );
        // The same from Markdown; single newlines stay line breaks.
        let doc =
            Doc::markdown("## Plan\n> quoted\n\n3. three\n4. four\n   - nested\n\n---\none\ntwo");
        assert_eq!(
            doc.blocks[..3],
            [
                Block::Heading(2, vec![plain("Plan")]),
                Block::Quote(vec![paragraph(vec![plain("quoted")])]),
                Block::List {
                    start: Some(3),
                    items: vec![
                        vec![paragraph(vec![plain("three")])],
                        vec![
                            paragraph(vec![plain("four")]),
                            Block::List {
                                start: None,
                                items: vec![vec![paragraph(vec![plain("nested")])]],
                            },
                        ],
                    ],
                },
            ]
        );
        assert_eq!(
            doc.blocks[3..],
            [Block::Rule, paragraph(vec![plain("one\ntwo")])]
        );
    }

    /// Exactly what Element Web 1.12.35 sends for these Markdown messages.
    #[test]
    fn element_messages() {
        let doc = Doc::message(
            Some(concat!(
                "<h1>Release notes</h1>\n<blockquote>\n<p>quoted wisdom</p>\n</blockquote>\n",
                "<ul>\n<li>first item</li>\n<li>second item</li>\n</ul>\n",
                "<ol>\n<li>alpha</li>\n<li>beta</li>\n</ol>\n"
            )),
            "# Release notes\n> quoted wisdom\n\n- first item\n- second item\n\n1. alpha\n2. beta",
        )
        .unwrap();
        let item = |text: &str| vec![paragraph(vec![plain(text)])];
        assert_eq!(
            doc.blocks,
            [
                Block::Heading(1, vec![plain("Release notes")]),
                Block::Quote(vec![paragraph(vec![plain("quoted wisdom")])]),
                Block::List {
                    start: None,
                    items: vec![item("first item"), item("second item")],
                },
                Block::List {
                    start: Some(1),
                    items: vec![item("alpha"), item("beta")],
                },
            ]
        );
    }

    #[test]
    fn html_whitespace_collapses_and_breaks_stay() {
        let doc = Doc::html("<p>a\n  b</p>\n<p>  c<br>d<br></p>");
        assert_eq!(
            doc.blocks,
            [
                paragraph(vec![plain("a b")]),
                paragraph(vec![plain("c\nd")])
            ]
        );
    }

    #[test]
    fn plain_bodies_stay_plain_and_raw_html_stays_text() {
        assert!(Doc::message(None, "just words, 2 * 3 * 4").is_none());
        assert!(Doc::message(None, "line one\nline two").is_none());
        assert!(Doc::message(Some("same text"), "same text").is_none());
        let doc = Doc::message(None, "use <b>tags</b> **here**").unwrap();
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                plain("use <b>tags</b> "),
                run(
                    "here",
                    Style {
                        bold: true,
                        ..Style::default()
                    }
                ),
            ])]
        );
    }

    #[test]
    fn deep_nesting_is_flattened() {
        let doc = Doc::markdown(&format!("{} deep", ">".repeat(500)));
        let mut depth = 0;
        let mut blocks = &doc.blocks;
        while let [Block::Quote(inner)] = blocks.as_slice() {
            depth += 1;
            blocks = inner;
        }
        assert!(depth <= MAX_NESTING, "{depth}");
        assert_eq!(blocks, &[paragraph(vec![plain("deep")])]);
    }

    #[test]
    fn colors_and_notes() {
        let mut doc = Doc::html(
            "<font color=\"#ff0000\">red</font><span data-mx-bg-color=\"#00ff00\">bg</span>",
        );
        doc.note("  ·  Sending…");
        assert_eq!(
            doc.blocks,
            [paragraph(vec![
                run(
                    "red",
                    Style {
                        color: Some([255, 0, 0]),
                        ..Style::default()
                    }
                ),
                run(
                    "bg",
                    Style {
                        background: Some([0, 255, 0]),
                        ..Style::default()
                    }
                ),
                plain("  ·  Sending…"),
            ])]
        );
    }
}
