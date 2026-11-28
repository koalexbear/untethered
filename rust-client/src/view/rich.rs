//! Message bodies with formatting ([`crate::rich`]): spans for inline styles,
//! and their own blocks for code, quotes and lists.
use super::{BODY, CHANNEL, El, LINE, LINK, MUTED, Msg, TEXT, line_rule, plain, surface};
use crate::{
    App,
    highlight::Painted,
    rich::{Block, Link, Run},
    rooms::{Post, Room},
};
use iced::{
    Border, Color, Fill, Font, Length, Padding, color, font,
    widget::{Column, button, column, container, hover, row, span, text, text::Span},
};

const SANS: Font = Font::with_name("Liberation Sans");
const MONO: Font = Font::with_name("Liberation Mono");
/// Inline code and code blocks, as on Discord.
const CODE: Color = color!(0x2b2d31);
const CODE_EDGE: Color = color!(0x1e1f22);
const SPOILER: Color = color!(0x1e1f22);
const QUOTE_BAR: Color = color!(0x4e5058);
const MENTION: Color = color!(0xc9cdfb);
const MENTION_BACK: Color = color!(0x5865f2, 0.3);
/// The text size of a message of only emoji, as Discord's jumbo emoji
/// (jumbo.rs); custom emoji draw at 1.375 times it, about 48 px.
const JUMBO: f32 = 36.;

struct Context<'a> {
    app: &'a App,
    /// The message's event; spoilers in a message not yet sent can't open.
    event: Option<&'a str>,
    /// Paragraphs' text size.
    size: f32,
    /// An action (`/me`): its text in italics, as Discord shows it.
    italic: bool,
}
impl Context<'_> {
    fn revealed(&self, spoiler: u16) -> bool {
        self.event
            .and_then(|e| self.app.spoilers.get(e))
            .is_some_and(|open| open.contains(&spoiler))
    }
    fn reveal(&self, spoiler: u16) -> Option<Msg> {
        self.event
            .map(|e| Msg::RevealSpoiler(e.to_owned(), spoiler))
    }
}

impl App {
    /// A message's text, formatted or plain. An action (`/me`, `m.emote`)
    /// follows "* Name", and a message of only emoji shows them larger
    /// (ROADMAP M13). "(edited)" goes after the last paragraph.
    pub(super) fn message_body<'a>(&'a self, room: &'a Room, p: &'a Post) -> El<'a> {
        let jumbo = !p.emote && crate::jumbo::emoji_only(p.text(), p.rich.as_deref());
        let c = Context {
            app: self,
            event: p.event_id.as_deref(),
            size: if jumbo { JUMBO } else { 16. },
            italic: p.emote,
        };
        let lead = p.emote.then(|| lead(room.display(&p.sender)));
        let edited = p.revision.is_some();
        if let Some(doc) = p.rich.as_deref() {
            return blocks(&c, &doc.blocks, 0, edited, lead).into();
        }
        let mut spans = lead.unwrap_or_default();
        spans.push(span(p.text()).font(italic(c.italic)).color(BODY));
        if edited {
            spans.push(span(" (edited)").size(11).color(CHANNEL));
        }
        super::selectable::rich(spans)
            .size(c.size)
            .line_height(1.375)
            .on_link_click(|m: Msg| m)
            .into()
    }
}

/// "* Name " before an action's text, as Element and IRC show `/me`.
fn lead<'a>(name: &str) -> Vec<Span<'a, Msg, Font>> {
    vec![
        span("* ").color(BODY),
        span(name.to_owned()).font(super::BOLD).color(TEXT),
        span(" ").color(BODY),
    ]
}

fn italic(on: bool) -> Font {
    Font {
        style: if on {
            font::Style::Italic
        } else {
            font::Style::Normal
        },
        ..SANS
    }
}

/// `lead` goes before the first paragraph's text, or on a line of its own
/// when the blocks start otherwise.
fn blocks<'a>(
    c: &Context<'a>,
    blocks: &'a [Block],
    depth: usize,
    edited: bool,
    lead: Option<Vec<Span<'a, Msg, Font>>>,
) -> Column<'a, Msg> {
    let ends_in_text = matches!(
        blocks.last(),
        Some(Block::Paragraph(_) | Block::Heading(..))
    );
    let mut column = Column::new().spacing(4).width(Fill);
    let mut lead = lead;
    for (i, b) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        let edited = edited && last && ends_in_text;
        column = column.push(match (lead.take(), b) {
            (Some(lead), Block::Paragraph(runs)) => paragraph(c, runs, c.size, false, edited, lead),
            (Some(lead), b) => column![
                paragraph(c, &[], c.size, false, false, lead),
                block(c, b, depth, edited)
            ]
            .spacing(4)
            .into(),
            (None, b) => block(c, b, depth, edited),
        });
    }
    if edited && !ends_in_text {
        column = column.push(text("(edited)").size(11).color(CHANNEL));
    }
    column
}

fn block<'a>(c: &Context<'a>, b: &'a Block, depth: usize, edited: bool) -> El<'a> {
    match b {
        Block::Paragraph(runs) => paragraph(c, runs, c.size, false, edited, Vec::new()),
        Block::Heading(level, runs) => {
            let size = match level {
                1 => 24.,
                2 => 20.,
                3 => 18.,
                _ => 16.,
            };
            paragraph(c, runs, size, true, edited, Vec::new())
        }
        Block::Code {
            language,
            text,
            spoiler,
            painted,
        } => code(c, language.as_deref(), text, *spoiler, painted),
        Block::Quote(inner) => row![
            container(iced::widget::Space::new())
                .width(4)
                .height(Fill)
                .style(|_| surface(QUOTE_BAR, 4.)),
            blocks(c, inner, depth, false, None)
        ]
        .spacing(12)
        // The bar fills the quote's height; without this the row takes the
        // bar's Fill and collapses inside the scrolling timeline.
        .height(Length::Shrink)
        .into(),
        Block::List { start, items } => list(c, *start, items, depth),
        Block::Rule => container(line_rule(LINE)).padding([6, 0]).into(),
    }
}

fn paragraph<'a>(
    c: &Context<'a>,
    runs: &'a [Run],
    size: f32,
    heading: bool,
    edited: bool,
    lead: Vec<Span<'a, Msg, Font>>,
) -> El<'a> {
    let skip = lead.len();
    let mut spans = lead;
    spans.extend(runs.iter().map(|r| styled(c, r, heading)));
    // Custom emoji (ROADMAP M11): their span is an invisible em dash a bit
    // larger than the text, and the picture is drawn over it once it's here.
    let mut images = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        let i = i + skip;
        if let Some(url) = &r.style.emote {
            spans[i] = std::mem::replace(&mut spans[i], span(""))
                .size(size * 1.375)
                .color(Color::TRANSPARENT);
            let hidden = r.style.spoiler.is_some_and(|id| !c.revealed(id));
            if !hidden && let Some(picture) = c.app.emotes.get(url) {
                images.push((i, picture.clone()));
            }
        }
    }
    if edited {
        spans.push(span(" (edited)").size(11).color(CHANNEL));
    }
    super::selectable::rich(spans)
        .size(size)
        .line_height(1.375)
        .on_link_click(|m: Msg| m)
        .images(images)
        .into()
}

fn styled<'a>(c: &Context<'a>, r: &'a Run, heading: bool) -> Span<'a, Msg, Font> {
    let s = &r.style;
    let rgb = |[r, g, b]: [u8; 3]| Color::from_rgb8(r, g, b);
    let font = if s.code {
        MONO
    } else {
        Font {
            weight: if s.bold || heading {
                font::Weight::Bold
            } else {
                font::Weight::Normal
            },
            style: if s.italic || c.italic {
                font::Style::Italic
            } else {
                font::Style::Normal
            },
            ..SANS
        }
    };
    let hidden = s.spoiler.is_some_and(|id| !c.revealed(id));
    let mut out = if hidden {
        // Colour emoji ignore the text colour: hide the characters too.
        span(
            r.text
                .chars()
                .map(|ch| if ch.is_whitespace() { ch } else { 'x' })
                .collect::<String>(),
        )
    } else {
        span(r.text.as_str())
    }
    .font(font)
    .color(s.color.map_or(if heading { TEXT } else { BODY }, rgb))
    .underline(s.underline)
    .strikethrough(s.strike);
    if let Some(back) = s.background {
        out = out.background(rgb(back));
    }
    if s.code {
        out = out
            .size(14)
            .background(CODE)
            .border(Border::default().rounded(3))
            .padding([0, 2]);
    }
    match &s.link {
        Some(Link::Web(url)) => out = out.color(LINK).link(Msg::OpenLink(url.clone())),
        // Profiles come with M5; until then a mention only looks like one.
        Some(Link::User(_)) => {
            out = out
                .color(MENTION)
                .background(MENTION_BACK)
                .border(Border::default().rounded(3))
                .padding([0, 2]);
        }
        None => {}
    }
    match s.spoiler {
        Some(id) if hidden => {
            out = out
                .color(SPOILER)
                .background(SPOILER)
                .border(Border::default().rounded(3))
                .link_maybe(c.reveal(id));
        }
        Some(_) => out = out.background(color!(0xffffff, 0.1)),
        None => {}
    }
    out
}

fn code<'a>(
    c: &Context<'a>,
    language: Option<&'a str>,
    code: &'a str,
    spoiler: Option<u16>,
    painted: &'a [Painted],
) -> El<'a> {
    let frame = |_: &iced::Theme| container::Style {
        background: Some(CODE.into()),
        border: Border {
            color: CODE_EDGE,
            width: 1.,
            radius: 4.into(),
        },
        ..Default::default()
    };
    if let Some(id) = spoiler.filter(|id| !c.revealed(*id)) {
        return button(text("Spoiler: click to show").size(14).color(MUTED))
            .padding([10, 12])
            .width(Fill)
            .style(|_, _| button::Style {
                background: Some(SPOILER.into()),
                border: Border::default().rounded(4),
                ..Default::default()
            })
            .on_press_maybe(c.reveal(id))
            .into();
    }
    let block = container(
        super::selectable::rich(colored(code, painted))
            .font(MONO)
            .size(14)
            .line_height(1.375)
            .color(BODY)
            .wrapping(text::Wrapping::WordOrGlyph)
            .on_link_click(|m: Msg| m),
    )
    .padding(Padding {
        top: 8.,
        bottom: 8.,
        left: 10.,
        right: 10.,
    })
    .width(Fill)
    .style(frame);
    let copied = c.app.copied_code.as_deref() == Some(code);
    let mut tools = row![].spacing(8).align_y(iced::Alignment::Center);
    if let Some(language) = language {
        tools = tools.push(text(language).size(12).color(CHANNEL));
    }
    tools = tools.push(
        button(text(if copied { "Copied" } else { "Copy" }).size(12))
            .padding([2, 8])
            .style(move |theme, status| button::Style {
                background: Some(CODE_EDGE.into()),
                border: Border::default().rounded(3),
                ..plain(MUTED, TEXT)(theme, status)
            })
            .on_press(Msg::CopyCode(code.to_owned())),
    );
    // Like Element, the Copy button shows while the pointer is on the block.
    // At the bottom: the message toolbar covers the top right of a message.
    hover(
        block,
        container(tools)
            .align_right(Fill)
            .align_bottom(Fill)
            .padding(4),
    )
}

/// The block's text as spans, in its syntax colours where it has them
/// (highlight.rs); the rest in the block's own colour.
fn colored<'a>(code: &'a str, painted: &'a [Painted]) -> Vec<Span<'a, Msg, Font>> {
    let mut spans = Vec::with_capacity(painted.len() * 2 + 1);
    let mut at = 0;
    for (range, [r, g, b]) in painted {
        // Ranges come in order and apart; anything else is skipped.
        let Some(text) = code.get(range.clone()).filter(|_| range.start >= at) else {
            continue;
        };
        if range.start > at {
            spans.push(span(&code[at..range.start]));
        }
        spans.push(span(text).color(Color::from_rgb8(*r, *g, *b)));
        at = range.end;
    }
    if at < code.len() || spans.is_empty() {
        spans.push(span(&code[at..]));
    }
    spans
}

fn list<'a>(c: &Context<'a>, start: Option<u64>, items: &'a [Vec<Block>], depth: usize) -> El<'a> {
    let last = start.map(|n| n.saturating_add(items.len() as u64));
    // Room for the widest number.
    let width = last.map_or(14., |n| 10. + 9. * n.to_string().len() as f32);
    let mut column = Column::new().spacing(2);
    for (i, item) in items.iter().enumerate() {
        let marker = match start {
            Some(n) => format!("{}.", n.saturating_add(i as u64)),
            None => ["•", "◦", "▪"][depth % 3].to_owned(),
        };
        column = column.push(
            row![
                text(marker)
                    .size(16)
                    .line_height(1.375)
                    .color(BODY)
                    .width(width)
                    .align_x(text::Alignment::Right),
                blocks(c, item, depth + 1, false, None)
            ]
            .spacing(6),
        );
    }
    column.into()
}
