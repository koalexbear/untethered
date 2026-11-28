use super::{
    App, BODY, BOLD, CHANNEL, CHAT, El, FIELD, ICON, Icon, LINE, MUTED, Msg, SIDE, TEXT,
    composer_editor, composer_send_key, emoji_picker, face, icon, icon_button, input_style,
    label_owned, line_rule, link_preview, nav, plain, scroll_style, surface, thin_scrollbar,
};
use crate::engine::{
    Command, Failure, Message,
    pins::Pin,
    threads::{Page, Request, Response},
};
use crate::media::{Kind as MediaKind, LocalFile, Media, PreviewRef};
use iced::{
    Alignment::Center,
    ContentFit, Fill,
    widget::{
        button, column, container, image, row, scrollable, text, text_editor, text_input, tooltip,
    },
};

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Panel {
    Pins,
    Threads,
    Replies(String),
}

#[derive(Clone)]
pub(crate) enum Action {
    Open(Panel),
    Close,
    Pin(String, bool),
    Draft(text_editor::Action),
    ToggleEmoji,
    EmojiQuery(String),
    Emoji(String),
    RemoveImage,
    Send,
    More,
    Refresh,
}

#[derive(Default)]
pub(crate) struct State {
    room: String,
    pub(crate) panel: Option<Panel>,
    waiting: Option<(i64, bool)>,
    pins: Vec<Pin>,
    pub(crate) root: Option<Box<Message>>,
    pub(crate) messages: Vec<Message>,
    before: Option<String>,
    images: BTreeMap<String, Media>,
    preview_links: BTreeMap<String, Vec<String>>,
    draft: text_editor::Content,
    emoji_search: Option<String>,
    photo: Option<LocalFile>,
    upload: Option<async_channel::Sender<()>>,
    picker: Option<i64>,
    pub(crate) error: Option<&'static str>,
}

impl State {
    pub(crate) fn loading(&self) -> bool {
        self.waiting.is_some()
    }

    pub(crate) fn pinned(&self, room: &str, event: &str) -> bool {
        self.room == room && self.pins.iter().any(|pin| pin.event == event)
    }
}

impl App {
    fn shown_thread_messages(&self) -> impl Iterator<Item = &Message> {
        let state = &self.discussions;
        let shown = state.room == self.selected
            && !self.people
            && matches!(state.panel, Some(Panel::Replies(_)));
        shown.then_some(state).into_iter().flat_map(|state| {
            state
                .root
                .iter()
                .map(Box::as_ref)
                .chain(state.messages.iter().rev())
        })
    }

    pub(crate) fn thread_preview_links(&self) -> impl Iterator<Item = &str> {
        self.shown_thread_messages()
            .flat_map(|message| self.discussions.links(message).iter().map(String::as_str))
    }

    pub(crate) fn thread_image_sources(&self) -> impl Iterator<Item = &PreviewRef> {
        self.shown_thread_messages()
            .filter_map(|message| self.discussions.image(message)?.preview.as_ref())
    }

    pub(crate) fn pick_thread_image(&mut self) -> iced::Task<Msg> {
        if self.discussions.loading() || !matches!(self.discussions.panel, Some(Panel::Replies(_)))
        {
            return iced::Task::none();
        }
        self.seq += 1;
        self.discussions.picker = Some(self.seq);
        let token = format!("thread-image:{}", self.seq);
        #[cfg(target_arch = "wasm32")]
        {
            crate::web_files::pick_avatar(&token);
            iced::Task::none()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            iced::Task::perform(
                async {
                    #[cfg(target_os = "linux")]
                    crate::app_id::portal_ready().await;
                    let picked = rfd::AsyncFileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                        .pick_file()
                        .await?;
                    LocalFile::from_path(picked.path().to_owned())
                },
                move |file| Msg::Files(token, file.into_iter().collect()),
            )
        }
    }

    pub(crate) fn thread_image_picked(&mut self, picker: i64, file: Option<LocalFile>) {
        let state = &mut self.discussions;
        if state.picker.take_if(|id| *id == picker).is_none() || state.loading() {
            return;
        }
        if let Some(file) = file {
            if file.size > self.media_limit {
                state.error = Some("This image exceeds the homeserver upload limit.");
            } else {
                state.photo = Some(file);
                state.error = None;
            }
        }
    }

    pub(crate) fn thread_uploaded(&mut self, id: i64, result: Result<serde_json::Value, Failure>) {
        let state = &mut self.discussions;
        if !state.waiting.is_some_and(|(waiting, _)| waiting == id) || state.upload.take().is_none()
        {
            return;
        }
        state.waiting = None;
        let Some(Panel::Replies(root)) = &state.panel else {
            return;
        };
        let Ok(mut content) = result else {
            state.error = Some("Image upload failed. Send again to retry.");
            return;
        };
        if content["msgtype"] != "m.image" {
            state.error = Some("Choose a supported image file.");
            return;
        }
        let caption = state.draft.text();
        if !caption.trim().is_empty() {
            content["filename"] = content["body"].clone();
            content["body"] = serde_json::json!(caption);
        }
        let request = Request::ReplyImage {
            root: root.clone(),
            latest: state.latest(root),
            content,
        };
        self.queue_discussion(request, false);
    }

    pub(crate) fn discussion(&mut self, action: Action) {
        let state = &mut self.discussions;
        if state.room != self.selected {
            *state = State {
                room: self.selected.clone(),
                ..State::default()
            };
        }
        match action {
            Action::Close => {
                state.panel = None;
                state.picker = None;
                state.emoji_search = None;
                if state.upload.take().is_some() {
                    state.waiting = None;
                }
                return;
            }
            Action::ToggleEmoji => {
                state.emoji_search = state.emoji_search.is_none().then(String::new);
                return;
            }
            Action::EmojiQuery(query) => {
                state.emoji_search = Some(query);
                return;
            }
            Action::Emoji(emoji) => {
                if !state.loading() && state.emoji_search.take().is_some() {
                    state
                        .draft
                        .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                            std::sync::Arc::new(emoji),
                        )));
                }
                return;
            }
            Action::RemoveImage => {
                if !state.loading() {
                    state.photo = None;
                }
                return;
            }
            Action::Draft(draft) => {
                if !state.loading() {
                    state.draft.perform(draft);
                }
                return;
            }
            _ => {}
        }
        if !self.logged_in || !self.verified || !self.connected || state.loading() {
            return;
        }
        let more = matches!(action, Action::More);
        let pin = match action {
            Action::Open(panel) => {
                if state.panel.as_ref() != Some(&panel) {
                    state.messages.clear();
                    state.root = None;
                    state.before = None;
                    state.preview_links.clear();
                    state.images.clear();
                    state.draft = text_editor::Content::new();
                    state.photo = None;
                    state.picker = None;
                    state.emoji_search = None;
                }
                state.panel = Some(panel);
                self.people = false;
                None
            }
            Action::Pin(event, pinned) => {
                state.panel = Some(Panel::Pins);
                state.picker = None;
                self.people = false;
                Some(Request::SetPin { event, pinned })
            }
            Action::Send => {
                let Some(Panel::Replies(root)) = &state.panel else {
                    return;
                };
                let Some(room) = self.live.list.get(&state.room) else {
                    return;
                };
                if state.root.is_none()
                    || room.successor.is_some()
                    || room.posting.can_post(&self.matrix_id, room.encrypted) == Some(false)
                {
                    return;
                }
                if let Some(file) = &state.photo {
                    if !room.has_metadata {
                        state.error = Some("Waiting for room encryption information. Try again.");
                        return;
                    }
                    let (lease, cancel) = async_channel::bounded(1);
                    self.seq += 1;
                    if self
                        .commands
                        .try_send(crate::backend::Command::Upload {
                            cancel,
                            id: self.seq,
                            file: file.clone(),
                            encrypt: room.encrypted,
                            limit: self.media_limit,
                        })
                        .is_ok()
                    {
                        state.waiting = Some((self.seq, false));
                        state.upload = Some(lease);
                        state.error = None;
                        state.emoji_search = None;
                    } else {
                        state.error = Some("Could not queue the image upload. Try again.");
                    }
                    return;
                }
                let body = state.draft.text();
                if body.trim().is_empty() {
                    return;
                }
                Some(Request::Reply {
                    root: root.clone(),
                    latest: state.latest(root),
                    body,
                })
            }
            Action::More if state.before.is_none() => return,
            Action::More | Action::Refresh => None,
            Action::Close
            | Action::Draft(_)
            | Action::ToggleEmoji
            | Action::EmojiQuery(_)
            | Action::Emoji(_)
            | Action::RemoveImage => return,
        };
        let before = more.then(|| state.before.clone()).flatten();
        let request = match (pin, &state.panel) {
            (Some(request), _) => request,
            (None, Some(Panel::Pins)) => Request::Pins,
            (None, Some(Panel::Threads)) => Request::Threads { before },
            (None, Some(Panel::Replies(root))) => Request::Replies {
                root: root.clone(),
                before,
            },
            (None, None) => return,
        };
        self.queue_discussion(request, more);
    }

    fn queue_discussion(&mut self, request: Request, more: bool) {
        let state = &mut self.discussions;
        self.seq += 1;
        state.error = None;
        if self.sdk.as_ref().is_some_and(|sdk| {
            sdk.try_send(Command::Discussion {
                id: self.seq,
                room: state.room.clone(),
                action: request,
            })
            .is_ok()
        }) {
            state.waiting = Some((self.seq, more));
        } else {
            state.error = Some("Could not queue the request. Try again.");
        }
    }

    pub(crate) fn discussion_response(&mut self, id: i64, result: Result<Response, Failure>) {
        let state = &mut self.discussions;
        let Some((_, more)) = state.waiting.filter(|(waiting, _)| *waiting == id) else {
            return;
        };
        state.waiting = None;
        match result {
            Ok(Response::Pins(pins)) => state.pins = pins,
            Ok(Response::Threads(page)) => state.page(page, more, false),
            Ok(Response::Replies { root, page }) => {
                if !more {
                    state.root = root;
                    state.preview_links.clear();
                    state.images.clear();
                }
                for message in state.root.iter().map(Box::as_ref).chain(&page.messages) {
                    if let Some(event) = &message.event_id {
                        if let Some(media) = message.media.as_ref().and_then(Media::parse)
                            && media.kind == MediaKind::Image
                        {
                            state.images.insert(event.clone(), media);
                        }
                        state.preview_links.entry(event.clone()).or_insert_with(|| {
                            if matches!(message.kind, crate::engine::Kind::Text) {
                                crate::link_previews::message_links(
                                    &message.body,
                                    message.formatted.as_deref(),
                                )
                            } else {
                                Vec::new()
                            }
                        });
                    }
                }
                state.page(page, more, true);
            }
            Ok(Response::PinChanged) => self.discussion(Action::Refresh),
            Ok(Response::Sent(_)) => {
                state.draft = text_editor::Content::new();
                state.photo = None;
                self.discussion(Action::Refresh);
            }
            Err(Failure::Forbidden) => state.error = Some("The homeserver denied this action."),
            Err(_) => state.error = Some("The request failed. Try again."),
        }
    }
}

impl State {
    fn latest(&self, root: &str) -> String {
        self.messages
            .iter()
            .rev()
            .find_map(|m| m.event_id.clone())
            .unwrap_or_else(|| root.to_owned())
    }

    fn image(&self, message: &Message) -> Option<&Media> {
        self.images.get(message.event_id.as_ref()?)
    }

    fn links(&self, message: &Message) -> &[String] {
        message
            .event_id
            .as_ref()
            .and_then(|id| self.preview_links.get(id))
            .map_or(&[], Vec::as_slice)
    }

    fn page(&mut self, mut page: Page, more: bool, older_first: bool) {
        if more {
            page.messages.retain(|message| {
                !self
                    .messages
                    .iter()
                    .any(|old| message.event_id.is_some() && message.event_id == old.event_id)
            });
            if older_first {
                page.messages.append(&mut self.messages);
                self.messages = page.messages;
            } else {
                self.messages.append(&mut page.messages);
            }
        } else {
            self.messages = page.messages;
        }
        self.before = page.before;
    }
}

fn control<'a>(label: &'a str, action: Action, enabled: bool) -> El<'a> {
    button(text(label).size(12))
        .padding([2, 4])
        .style(plain(CHANNEL, BODY))
        .on_press_maybe(enabled.then_some(Msg::Discussion(action)))
        .into()
}

fn message<'a>(app: &App, message: &'a Message) -> El<'a> {
    let name = message
        .sender_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| crate::rooms::short(&message.sender));
    let picture = app.avatars.get(message.sender_avatar.as_deref());
    row![
        face(name, picture, 32.),
        column![
            text(name).size(14).font(BOLD).color(TEXT),
            text(&message.body).size(14).color(BODY),
        ]
        .spacing(4)
        .width(Fill),
    ]
    .spacing(10)
    .into()
}

fn thread_message<'a>(app: &'a App, item: &'a Message) -> El<'a> {
    let row = message(app, item);
    let mut content = column![row].spacing(8);
    if let Some(media) = app.discussions.image(item)
        && let Some(source) = &media.preview
    {
        let (w, h) = media.inline_size().unwrap_or((270., 165.));
        let scale = (270. / w).min(210. / h).min(1.);
        let (width, height) = (w * scale, h * scale);
        let preview: El = if let Some(handle) = app.inline.ready(source) {
            image(handle)
                .width(width)
                .height(height)
                .content_fit(ContentFit::Contain)
                .border_radius(6)
                .into()
        } else if app.inline.pending(source) {
            container(text("Loading image…").size(13).color(MUTED))
                .center_x(width)
                .center_y(height)
                .style(|_| surface(CHAT, 6.))
                .into()
        } else {
            button(text("Retry image preview").size(13))
                .style(nav(false))
                .on_press(Msg::ShowInline(source.clone()))
                .into()
        };
        content = content.push(preview);
    }
    if app
        .live
        .list
        .get(&app.selected)
        .is_some_and(|room| room.link_previews)
        && let Some(cards) = link_preview::view_links(
            app,
            app.discussions.links(item).iter().map(String::as_str),
            270.,
        )
    {
        content = content.push(cards);
    }
    content.into()
}

fn thread_origin<'a>(
    app: &'a App,
    event: &str,
    root: Option<&'a Message>,
    loading: bool,
) -> El<'a> {
    let mut content = column![label_owned("Original message".into())].spacing(10);
    if let Some(root) = root {
        content = content.push(thread_message(app, root));
    } else if !loading {
        content = content.push(text("Message preview unavailable").size(13).color(MUTED));
    }
    content = content.push(
        button(text("Jump to original").size(13).font(BOLD))
            .style(nav(false))
            .padding([5, 8])
            .on_press(Msg::Jump(event.to_owned())),
    );
    container(content)
        .padding(12)
        .width(Fill)
        .style(|_| surface(CHAT, 6.))
        .into()
}

pub(super) fn view(app: &App) -> Option<El<'_>> {
    let state = &app.discussions;
    let panel = state
        .panel
        .as_ref()
        .filter(|_| state.room == app.selected)?;
    let ready = app.connected && app.verified && !state.loading();
    let can_pin = ready
        && app
            .live
            .list
            .get(&app.selected)
            .is_some_and(|room| room.posting.can_pin_message(&app.matrix_id));
    let can_reply = ready
        && state.root.is_some()
        && app.live.list.get(&app.selected).is_some_and(|room| {
            room.posting.can_post(&app.matrix_id, room.encrypted) != Some(false)
        });
    let title = match panel {
        Panel::Pins => "Pinned Messages",
        Panel::Threads => "Threads",
        Panel::Replies(_) => "Thread",
    };
    let mut entries = column![].spacing(10);
    match panel {
        Panel::Pins => {
            for pin in &state.pins {
                let preview = pin.message.as_ref().map_or_else(
                    || text("This pinned message is unavailable.").size(14).into(),
                    |entry| message(app, entry),
                );
                entries = entries.push(
                    column![
                        preview,
                        row![
                            button(text("Jump to message").size(12))
                                .padding([2, 4])
                                .style(plain(CHANNEL, BODY))
                                .on_press(Msg::Jump(pin.event.clone())),
                            control("Unpin", Action::Pin(pin.event.clone(), false), can_pin),
                        ]
                        .spacing(6)
                    ]
                    .spacing(4)
                    .padding([5, 8]),
                );
            }
            if ready && state.error.is_none() && state.pins.is_empty() {
                entries = entries.push(text("No pinned messages.").size(13).color(MUTED));
            }
        }
        Panel::Threads => {
            for root in &state.messages {
                if let Some(event) = &root.event_id {
                    entries =
                        entries.push(
                            button(
                                row![message(app, root), icon(Icon::ChevronRight, 14., ICON)]
                                    .spacing(6)
                                    .align_y(Center),
                            )
                            .width(Fill)
                            .padding([5, 8])
                            .style(nav(false))
                            .on_press_maybe(ready.then(|| {
                                Msg::Discussion(Action::Open(Panel::Replies(event.clone())))
                            })),
                        );
                }
            }
            if ready && state.error.is_none() && state.messages.is_empty() {
                entries = entries.push(text("No threads.").size(13).color(MUTED));
            }
            if state.before.is_some() {
                entries = entries.push(control("More threads", Action::More, ready));
            }
        }
        Panel::Replies(event) => {
            entries = entries
                .push(thread_origin(
                    app,
                    event,
                    state.root.as_deref(),
                    state.loading(),
                ))
                .push(
                    row![label_owned("Replies".into()), line_rule(LINE),]
                        .spacing(8)
                        .align_y(Center),
                );
            if state.before.is_some() {
                entries = entries.push(control("Load older replies", Action::More, ready));
            }
            for reply in &state.messages {
                entries = entries.push(container(thread_message(app, reply)).padding([5, 4]));
            }
            if ready && state.error.is_none() && state.messages.is_empty() {
                entries = entries.push(text("No replies yet.").size(13).color(MUTED));
            }
        }
    }
    let mut header = row![container(label_owned(title.to_uppercase())).width(Fill)]
        .align_y(Center)
        .spacing(6);
    if matches!(panel, Panel::Replies(_)) {
        header = row![
            icon_button(
                Icon::ChevronLeft,
                "Back to Threads",
                false,
                ready.then_some(Msg::Discussion(Action::Open(Panel::Threads))),
                tooltip::Position::Bottom,
            ),
            header,
        ]
        .align_y(Center)
        .spacing(4);
    }
    let mut panel_view = column![
        header
            .push(icon_button(
                Icon::Refresh,
                "Refresh",
                false,
                ready.then_some(Msg::Discussion(Action::Refresh)),
                tooltip::Position::Bottom,
            ))
            .push(icon_button(
                Icon::Close,
                "Close",
                false,
                Some(Msg::Discussion(Action::Close)),
                tooltip::Position::Bottom,
            ))
    ]
    .spacing(10)
    .padding([12, 8]);
    if state.loading() {
        panel_view = panel_view.push(text("Loading…").size(13).color(CHANNEL));
    }
    if let Some(error) = state.error {
        panel_view = panel_view.push(text(error).size(13));
    }
    panel_view = panel_view.push(
        scrollable(entries)
            .direction(thin_scrollbar())
            .style(scroll_style(None))
            .height(Fill),
    );
    if matches!(panel, Panel::Replies(_)) {
        let on_edit: fn(text_editor::Action) -> Msg = |edit| Msg::Discussion(Action::Draft(edit));
        let input = composer_editor(
            &state.draft,
            "thread-composer",
            "Reply in thread".into(),
            can_reply.then_some(on_edit),
            |press| {
                if composer_send_key(&press) {
                    Some(text_editor::Binding::Custom(Msg::Discussion(Action::Send)))
                } else {
                    text_editor::Binding::from_key_press(press)
                }
            },
        );
        if let Some(query) = &state.emoji_search {
            panel_view = panel_view.push(emoji_picker::panel(
                "Emoji",
                query,
                text_input("Search emoji", query)
                    .style(input_style)
                    .on_input(|query| Msg::Discussion(Action::EmojiQuery(query)))
                    .into(),
                None,
                None,
                |emoji| Some(Msg::Discussion(Action::Emoji(emoji.into()))),
                |_| false,
            ));
        }
        if let Some(photo) = &state.photo {
            panel_view = panel_view.push(
                row![
                    text(&photo.name).size(13).width(Fill),
                    icon_button(
                        Icon::Close,
                        "Remove image",
                        false,
                        (!state.loading()).then_some(Msg::Discussion(Action::RemoveImage)),
                        tooltip::Position::Top
                    ),
                ]
                .align_y(Center),
            );
        }
        panel_view = panel_view.push(
            container(
                row![
                    icon_button(
                        Icon::Plus,
                        "Attach image",
                        false,
                        can_reply.then_some(Msg::PickThreadImage),
                        tooltip::Position::Top
                    ),
                    input,
                    icon_button(
                        Icon::Smile,
                        "Emoji",
                        state.emoji_search.is_some(),
                        can_reply.then_some(Msg::Discussion(Action::ToggleEmoji)),
                        tooltip::Position::Top
                    ),
                    icon_button(
                        Icon::Send,
                        "Send Reply",
                        false,
                        (can_reply
                            && (state.photo.is_some() || !state.draft.text().trim().is_empty()))
                        .then_some(Msg::Discussion(Action::Send)),
                        tooltip::Position::Top,
                    ),
                ]
                .align_y(Center)
                .spacing(6)
                .padding([0, 8]),
            )
            .width(Fill)
            .style(|_| surface(FIELD, 8.)),
        );
    }
    Some(
        container(panel_view)
            .width(if matches!(panel, Panel::Replies(_)) {
                320
            } else {
                240
            })
            .height(Fill)
            .style(|_| surface(SIDE, 0.0))
            .into(),
    )
}
