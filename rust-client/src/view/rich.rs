//! Message bodies with formatting ([`crate::rich`]): spans for inline styles,
//! and their own blocks for code, quotes and lists.
use super::{BODY, CHANNEL, El, LINE, LINK, MUTED, Msg, TEXT, line_rule, plain, surface};
use crate::{
    App,
    rich::{Block, Doc, Link, Run},
};
use iced::{
    Border, Color, Fill, Font, Length, Padding, color, font,
    widget::{Column, button, container, hover, row, span, text, text::Span},
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

struct Context<'a> {
    app: &'a App,
    /// The message's event; spoilers in a message not yet sent can't open.
    event: Option<&'a str>,
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

/// `edited` adds "(edited)" after the last paragraph, as for plain text.
pub(super) fn view<'a>(app: &'a App, event: Option<&'a str>, doc: &'a Doc, edited: bool) -> El<'a> {
    blocks(&Context { app, event }, &doc.blocks, 0, edited).into()
}

fn blocks<'a>(c: &Context<'a>, blocks: &'a [Block], depth: usize, edited: bool) -> Column<'a, Msg> {
    let ends_in_text = matches!(
        blocks.last(),
        Some(Block::Paragraph(_) | Block::Heading(..))
    );
    let mut column = Column::new().spacing(4).width(Fill);
    for (i, b) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        column = column.push(block(c, b, depth, edited && last && ends_in_text));
    }
    if edited && !ends_in_text {
        column = column.push(text("(edited)").size(11).color(CHANNEL));
    }
    column
}

fn block<'a>(c: &Context<'a>, b: &'a Block, depth: usize, edited: bool) -> El<'a> {
    match b {
        Block::Paragraph(runs) => paragraph(c, runs, 16., false, edited),
        Block::Heading(level, runs) => {
            let size = match level {
                1 => 24.,
                2 => 20.,
                3 => 18.,
                _ => 16.,
            };
            paragraph(c, runs, size, true, edited)
        }
        Block::Code {
            language,
            text,
            spoiler,
        } => code(c, language.as_deref(), text, *spoiler),
        Block::Quote(inner) => row![
            container(iced::widget::Space::new())
                .width(4)
                .height(Fill)
                .style(|_| surface(QUOTE_BAR, 4.)),
            blocks(c, inner, depth, false)
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
) -> El<'a> {
    let mut spans: Vec<Span<'a, Msg, Font>> = runs.iter().map(|r| styled(c, r, heading)).collect();
    if edited {
        spans.push(span(" (edited)").size(11).color(CHANNEL));
    }
    super::selectable::rich(spans)
        .size(size)
        .line_height(1.375)
        .on_link_click(|m: Msg| m)
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
            style: if s.italic {
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
        super::selectable::rich([span(code)])
            .font(MONO)
            .size(14)
            .line_height(1.375)
            .color(BODY)
            .wrapping(text::Wrapping::WordOrGlyph)
            .on_link_click(iced::never),
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
                blocks(c, item, depth + 1, false)
            ]
            .spacing(6),
        );
    }
    column.into()
}
