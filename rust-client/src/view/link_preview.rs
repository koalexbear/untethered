//! Link previews under a message, as Discord's embeds: the site's name, the
//! page's title as a link, its description, and its image, large below or
//! small beside the text. A link straight to an image shows the picture.
use super::{BODY, BOLD, CHANNEL, El, LINK, MUTED, Msg, plain, surface};
use crate::{
    App,
    engine::LinkPreview,
    media::{Fit, PreviewRef},
    rooms::Post,
};
use iced::{
    Border, Color, ContentFit, Length, Padding, color,
    widget::{Column, Space, button, column, container, image, mouse_area, row, text},
};

/// Discord's embed width, and its image bounds.
const WIDTH: f32 = 432.;
const IMAGE: (f32, f32) = (400., 260.);
/// An image narrower than this sits beside the text.
const BESIDE: u32 = 300;
const THUMB: f32 = 80.;
/// Description characters shown.
const DESCRIPTION: usize = 350;

/// The previews of `post`'s links that the homeserver had, if any.
pub(super) fn view<'a>(app: &'a App, post: &'a Post) -> Option<El<'a>> {
    let cards: Vec<El<'a>> = crate::link_previews::links(post)
        .into_iter()
        .filter_map(|url| app.link_previews.get(&url))
        .map(|preview| card(app, preview))
        .collect();
    (!cards.is_empty()).then(|| Column::with_children(cards).spacing(6).into())
}

/// The preview's image as the inline image cache loads it.
pub fn image_source(preview: &LinkPreview) -> Option<PreviewRef> {
    Some(PreviewRef {
        mxc: preview.image.clone()?,
        encrypted: false,
        fit: Fit::Inline,
    })
}

/// `(w, h)` within `bounds`, never enlarged.
fn fit((w, h): (u32, u32), bounds: (f32, f32)) -> (f32, f32) {
    let scale = (bounds.0 / w as f32).min(bounds.1 / h as f32).min(1.);
    (
        (w as f32 * scale).round().max(1.),
        (h as f32 * scale).round().max(1.),
    )
}

fn picture<'a>(app: &'a App, preview: &LinkPreview, size: (f32, f32)) -> El<'a> {
    match image_source(preview)
        .as_ref()
        .and_then(|s| app.inline.ready(s))
    {
        Some(handle) => image(handle)
            .width(size.0)
            .height(size.1)
            .content_fit(ContentFit::Contain)
            .border_radius(4)
            .into(),
        // Its space is kept while it loads.
        None => container(Space::new())
            .width(size.0)
            .height(size.1)
            .style(|_| surface(color!(0x1e1f22), 4.))
            .into(),
    }
}

fn card<'a>(app: &'a App, p: &LinkPreview) -> El<'a> {
    let open = Msg::OpenLink(p.url.clone());
    let size = p.image_size.unwrap_or((400, 225));
    // A link to an image: the picture, opening the link.
    if p.title.is_none() && p.description.is_none() && p.image.is_some() {
        return mouse_area(picture(app, p, fit(size, IMAGE)))
            .on_press(open)
            .interaction(iced::mouse::Interaction::Pointer)
            .into();
    }
    let mut words = Column::new().spacing(6).width(Length::Fill);
    if let Some(site) = &p.site {
        words = words.push(text(site.clone()).size(12).color(MUTED));
    }
    if let Some(title) = &p.title {
        words = words.push(
            button(text(title.clone()).size(16).font(BOLD))
                .padding(0)
                .on_press(open.clone())
                .style(plain(LINK, color!(0x5fc5ff))),
        );
    }
    if let Some(description) = &p.description {
        let mut shown: String = description.chars().take(DESCRIPTION).collect();
        if description.chars().count() > DESCRIPTION {
            shown.push('…');
        }
        words = words.push(text(shown).size(14).color(BODY));
    }
    let beside = p.image.is_some() && size.0 < BESIDE;
    let body: El<'a> = if beside {
        row![words, picture(app, p, fit(size, (THUMB, THUMB)))]
            .spacing(16)
            .into()
    } else if p.image.is_some() {
        column![words, picture(app, p, fit(size, IMAGE))]
            .spacing(12)
            .into()
    } else {
        words.into()
    };
    container(row![
        container(Space::new())
            .width(4)
            .height(Length::Fill)
            .style(|_| surface(color!(0x1e1f22), 2.)),
        container(body).padding(Padding {
            top: 8.,
            right: 16.,
            bottom: 16.,
            left: 12.,
        }),
    ])
    .height(Length::Shrink)
    .max_width(WIDTH)
    .style(|_| container::Style {
        background: Some(color!(0x2b2d31).into()),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.,
            radius: 4.0.into(),
        },
        text_color: Some(CHANNEL),
        ..Default::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_fit_their_bounds_without_growing() {
        assert_eq!(fit((1200, 630), IMAGE), (400., 210.));
        assert_eq!(fit((64, 64), IMAGE), (64., 64.));
        assert_eq!(fit((200, 100), (THUMB, THUMB)), (80., 40.));
    }
}
