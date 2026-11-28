//! Profile customization (ROADMAP M15): a banner, an About Me and an accent
//! colour on the profile card, which only Untethered shows.
//!
//! They are extended profile fields under our own names
//! (engine/profile_fields.rs). This account's are loaded when My Account
//! first opens, with whether the homeserver lets it set them
//! (`m.profile_fields`): where it doesn't, My Account hides the editors
//! (ROADMAP rule 5). Anyone's are loaded again each time their card opens. Per-server pictures, the
//! other half of M15, live with the nicknames (names.rs).
use crate::{
    App, Msg,
    backend::Command as Backend,
    engine::{
        Command, Failure,
        profile_fields::{BIO_LENGTH, Edit, Editable, Fields, Reply, Request},
    },
    media::{Fit, LocalFile, PreviewRef},
    names::Picked,
    preview::{Frames, Picture},
};
use iced::{Task, widget::text_editor};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// The file picker's answer for a banner (`Msg::Files`).
pub const PICKER: &str = "profile-banner:";
/// Banners fetched at once.
const INFLIGHT: usize = 2;
/// Accent colours offered, as Discord offers a palette.
pub const ACCENTS: [u32; 8] = [
    0x5865f2, 0x3ccfae, 0x23a55a, 0xf1c40f, 0xe67e22, 0xf47067, 0xeb459e, 0x9b59b6,
];

#[derive(Default)]
pub struct State {
    /// This account's fields, once loaded.
    pub own: Option<Fields>,
    /// What the homeserver lets this account set; None until it said.
    pub editable: Option<Editable>,
    /// Other people's fields, as their cards last loaded them.
    pub cards: BTreeMap<String, Fields>,
    /// This account's fields were asked for, this session.
    asked: bool,
    /// Loads in flight, by request.
    loading: BTreeMap<i64, String>,
    /// The save in flight.
    pub saving: Option<i64>,
    /// The banner's file picker is open, as this request.
    pub picking: Option<i64>,
    /// The About Me editor: the saved one until edited.
    pub bio: text_editor::Content,
    /// The editor holds an edit not saved yet.
    pub bio_edited: bool,
    pub note: Option<&'static str>,
    banners: BTreeMap<String, Option<Picture>>,
    inflight: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub enum Input {
    Bio(text_editor::Action),
    SaveBio,
    Accent(Option<u32>),
    PickBanner,
    Picked(i64, Result<Option<Arc<Picked>>, &'static str>),
    RemoveBanner,
    BannerReady(String, Option<Picture>),
}

impl State {
    pub fn busy(&self) -> bool {
        self.saving.is_some() || self.picking.is_some()
    }

    /// A loaded banner, playing if it's animated.
    pub fn banner(&self, mxc: &str) -> Option<&Picture> {
        self.banners.get(mxc)?.as_ref()
    }

    /// Someone's fields as last loaded: this account's own, or a card's.
    pub fn of(&self, user: &str, me: &str) -> Option<&Fields> {
        if user == me {
            self.own.as_ref()
        } else {
            self.cards.get(user)
        }
    }

    /// The About Me as the editor holds it.
    pub fn bio_text(&self) -> String {
        let text = self.bio.text();
        text.strip_suffix('\n').unwrap_or(&text).to_owned()
    }

    /// The About Me as saved.
    pub fn saved_bio(&self) -> &str {
        self.own
            .as_ref()
            .and_then(|f| f.bio.as_deref())
            .unwrap_or_default()
    }

    /// Show the saved About Me in the editor, unless it holds an edit.
    fn reset_bio(&mut self) {
        if !self.bio_edited {
            self.bio = text_editor::Content::with_text(self.saved_bio());
        }
    }
}

impl App {
    /// The profile fields may be shown: the matrix-sdk engine, signed in.
    fn may_load_profile_fields(&self) -> bool {
        self.logged_in && self.verified && self.connected && self.sdk.is_some()
    }

    /// Ask for this account's fields, and whether it may set them, once My
    /// Account opens; forget them on signing out.
    pub fn sync_profile_fields(&mut self) {
        if !self.logged_in {
            if self.profile_fields.asked {
                self.profile_fields = State::default();
            }
            return;
        }
        if self.account && !self.profile_fields.asked && self.may_load_profile_fields() {
            self.profile_fields.asked = true;
            let me = self.matrix_id.clone();
            self.load_profile_fields(me);
        }
    }

    /// Load someone's fields: on opening their card, and this account's.
    pub fn load_profile_fields(&mut self, user: String) {
        if !self.may_load_profile_fields()
            || self.profile_fields.loading.values().any(|u| *u == user)
        {
            return;
        }
        if let Some(id) = self.send_profile_fields(Request::Load(user.clone())) {
            self.profile_fields.loading.insert(id, user);
        }
    }

    fn send_profile_fields(&mut self, request: Request) -> Option<i64> {
        self.seq += 1;
        let id = self.seq;
        self.sdk
            .as_ref()?
            .try_send(Command::ProfileFields { id, request })
            .ok()?;
        Some(id)
    }

    pub fn profile_fields_reply(&mut self, id: i64, result: Result<Reply, Failure>) {
        let state = &mut self.profile_fields;
        if let Some(user) = state.loading.remove(&id) {
            match result {
                Ok(Reply::Loaded {
                    user: loaded,
                    fields,
                    editable,
                }) if loaded == user => {
                    if user == self.matrix_id {
                        state.own = Some(fields);
                        state.editable = editable.or(state.editable);
                        state.reset_bio();
                    } else {
                        state.cards.insert(user, fields);
                    }
                }
                // A card keeps what it showed; this account asks again next
                // time its card opens.
                _ => {}
            }
            return;
        }
        if state.saving != Some(id) {
            return;
        }
        state.saving = None;
        match result {
            Ok(Reply::Saved(saved)) => {
                use crate::engine::profile_fields::Saved;
                state.note = Some(match &saved {
                    Saved::Banner(_) => "Banner saved.",
                    Saved::Bio(_) => "About Me saved.",
                    Saved::Accent(_) => "Accent colour saved.",
                });
                let bio = matches!(saved, Saved::Bio(_));
                state.own.get_or_insert_with(Fields::default).apply(saved);
                if bio {
                    state.bio_edited = false;
                    state.reset_bio();
                }
            }
            _ => state.note = Some("Couldn't save your profile. Try again."),
        }
    }

    fn save_profile_field(&mut self, edit: Edit) {
        if self.profile_fields.busy() || !self.may_load_profile_fields() {
            return;
        }
        self.profile_fields.note = None;
        match self.send_profile_fields(Request::Save(edit)) {
            Some(id) => self.profile_fields.saving = Some(id),
            None => self.profile_fields.note = Some("Couldn't save your profile. Try again."),
        }
    }

    pub fn profile_fields_input(&mut self, input: Input) -> Task<Msg> {
        let editable = self.profile_fields.editable.unwrap_or_default();
        match input {
            Input::Bio(action) if editable.bio && self.profile_fields.saving.is_none() => {
                let state = &mut self.profile_fields;
                let edits = action.is_edit();
                state.bio.perform(action);
                if edits {
                    state.note = None;
                    state.bio_edited = true;
                    // Discord's limit; the engine trims it again.
                    let text = state.bio_text();
                    if text.chars().count() > BIO_LENGTH {
                        let kept: String = text.chars().take(BIO_LENGTH).collect();
                        state.bio = text_editor::Content::with_text(&kept);
                        state
                            .bio
                            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                    }
                }
            }
            Input::SaveBio if editable.bio => {
                let bio = self.profile_fields.bio_text();
                let bio = Some(bio.trim().to_owned()).filter(|b| !b.is_empty());
                self.save_profile_field(Edit::Bio(bio));
            }
            Input::Accent(accent) if editable.accent => {
                self.save_profile_field(Edit::Accent(accent));
            }
            Input::RemoveBanner if editable.banner => self.save_profile_field(Edit::Banner(None)),
            Input::PickBanner if editable.banner && !self.profile_fields.busy() => {
                self.seq += 1;
                self.profile_fields.picking = Some(self.seq);
                self.profile_fields.note = None;
                return pick_picture(format!("{PICKER}{}", self.seq), "Choose a profile banner");
            }
            Input::Picked(id, result) if self.profile_fields.picking == Some(id) => {
                self.profile_fields.picking = None;
                match result {
                    Ok(Some(picked)) => {
                        self.save_profile_field(Edit::Banner(Some(picked.bytes.clone())));
                    }
                    Ok(None) => {}
                    Err(note) => self.profile_fields.note = Some(note),
                }
            }
            Input::BannerReady(mxc, picture) => {
                let state = &mut self.profile_fields;
                if state.inflight.remove(&mxc) {
                    state.banners.insert(mxc, picture);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// The file picker's answer for a banner.
    pub fn banner_picked(&mut self, id: i64, file: Option<LocalFile>) -> Task<Msg> {
        if self.profile_fields.picking != Some(id) {
            return Task::none();
        }
        let Some(file) = file else {
            return self.profile_fields_input(Input::Picked(id, Ok(None)));
        };
        Task::perform(read_picture(file, false), move |r| {
            Msg::ProfileFields(Input::Picked(id, r.map(Some)))
        })
    }

    /// The banners on screen: this account's, and the open card's.
    fn wanted_banners(&self) -> Vec<String> {
        let state = &self.profile_fields;
        let mut wanted: Vec<String> = Vec::new();
        if self.account
            && let Some(b) = state.own.as_ref().and_then(|f| f.banner.clone())
        {
            wanted.push(b);
        }
        if let Some((_, user)) = &self.sdk_ui.profile.card
            && let Some(b) = state
                .of(user, &self.matrix_id)
                .and_then(|f| f.banner.clone())
        {
            wanted.push(b);
        }
        wanted
    }

    /// Fetch the banners on screen, a couple at a time; each is kept for
    /// the session.
    pub fn schedule_banners(&mut self) -> Task<Msg> {
        if !self.logged_in || !self.connected {
            return Task::none();
        }
        let mut tasks = Vec::new();
        for mxc in self.wanted_banners() {
            let state = &mut self.profile_fields;
            if state.inflight.len() >= INFLIGHT {
                break;
            }
            if state.banners.contains_key(&mxc) || state.inflight.contains(&mxc) {
                continue;
            }
            let (tx, rx) = async_channel::bounded(1);
            let source = PreviewRef {
                mxc: mxc.clone(),
                encrypted: false,
                fit: Fit::Inline,
            };
            if self
                .commands
                .try_send(Backend::Inline { source, result: tx })
                .is_err()
            {
                break;
            }
            self.profile_fields.inflight.insert(mxc.clone());
            tasks.push(Task::perform(
                async move { rx.recv().await.ok().flatten() },
                move |picture| Msg::ProfileFields(Input::BannerReady(mxc.clone(), picture)),
            ));
        }
        Task::batch(tasks)
    }
}

/// Which picture a `Msg::Files` token answers for: (is a banner, request).
pub fn picker(token: &str) -> Option<(bool, i64)> {
    if let Some(id) = token.strip_prefix(PICKER) {
        return Some((true, id.parse().ok()?));
    }
    Some((
        false,
        token.strip_prefix(crate::names::PICKER)?.parse().ok()?,
    ))
}

impl App {
    /// A picture picked for a banner or a server picture.
    pub fn picture_picked(&mut self, banner: bool, id: i64, file: Option<LocalFile>) -> Task<Msg> {
        if banner {
            self.banner_picked(id, file)
        } else {
            self.server_picture_picked(id, file)
        }
    }
}

/// Open the file picker for a picture; it answers with `Msg::Files(token)`.
pub fn pick_picture(token: String, title: &'static str) -> Task<Msg> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = title;
        crate::web_files::pick_avatar(&token);
        Task::none()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Task::perform(
            async move {
                #[cfg(target_os = "linux")]
                crate::app_id::portal_ready().await;
                let picked = rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
                    .pick_file()
                    .await?;
                LocalFile::from_path(picked.path().to_owned())
            },
            move |file| Msg::Files(token, file.into_iter().collect()),
        )
    }
}

/// A picked picture's bytes, checked as the profile picture's are, and a
/// round preview for a server picture.
pub async fn read_picture(file: LocalFile, round: bool) -> Result<Arc<Picked>, &'static str> {
    let bytes = crate::engine::profile::avatar_bytes(file).await?;
    let preview = round.then(|| crate::avatars::preview(&bytes)).flatten();
    Ok(Arc::new(Picked { bytes, preview }))
}

/// The frames of an animated banner, for `animated::animated`.
pub fn moving(picture: &Picture) -> Option<&Arc<Frames>> {
    match picture {
        Picture::Moving(frames) => Some(frames),
        Picture::Still(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::profile_fields::Saved;

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (backend, _) = async_channel::bounded(1);
        let (sdk, rx) = async_channel::bounded(16);
        let mut app = App::new(backend);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.matrix_id = "@me:example.org".into();
        app.profile = "Me".into();
        (app, rx)
    }

    fn loaded(app: &mut App, rx: &async_channel::Receiver<Command>, editable: Editable) {
        app.sync_profile_fields();
        assert!(rx.try_recv().is_err(), "not until My Account opens");
        app.account = true;
        app.sync_profile_fields();
        let Ok(Command::ProfileFields {
            id,
            request: Request::Load(user),
        }) = rx.try_recv()
        else {
            panic!("expected this account's fields to load");
        };
        assert_eq!(user, "@me:example.org");
        app.profile_fields_reply(
            id,
            Ok(Reply::Loaded {
                user,
                fields: Fields {
                    bio: Some("Hello".into()),
                    ..Fields::default()
                },
                editable: Some(editable),
            }),
        );
    }

    #[test]
    fn own_fields_load_once_and_signing_out_forgets_them() {
        let (mut app, rx) = app();
        loaded(&mut app, &rx, Editable::default());
        assert_eq!(
            app.profile_fields.own.as_ref().unwrap().bio.as_deref(),
            Some("Hello")
        );
        app.sync_profile_fields();
        assert!(rx.try_recv().is_err(), "once a session");
        app.logged_in = false;
        app.sync_profile_fields();
        assert!(app.profile_fields.own.is_none());
        assert!(app.profile_fields.editable.is_none());
    }

    #[test]
    fn a_homeserver_without_profile_fields_gets_no_edits() {
        let (mut app, rx) = app();
        loaded(&mut app, &rx, Editable::default());
        assert_eq!(app.profile_fields.editable, Some(Editable::default()));
        let _ = app.profile_fields_input(Input::Accent(Some(0x3ccfae)));
        let _ = app.profile_fields_input(Input::RemoveBanner);
        let _ = app.profile_fields_input(Input::PickBanner);
        let _ = app.profile_fields_input(Input::Bio(text_editor::Action::Edit(
            text_editor::Edit::Insert('x'),
        )));
        let _ = app.profile_fields_input(Input::SaveBio);
        assert!(rx.try_recv().is_err());
        assert_eq!(app.profile_fields.bio_text(), "Hello");
        assert!(!app.profile_fields.bio_edited);
        assert!(app.profile_fields.picking.is_none());
    }

    #[test]
    fn edits_save_one_field_and_apply_only_their_answer() {
        let all = Editable {
            banner: true,
            bio: true,
            accent: true,
        };
        let (mut app, rx) = app();
        loaded(&mut app, &rx, all);
        let _ = app.profile_fields_input(Input::Accent(Some(0x3ccfae)));
        let Ok(Command::ProfileFields {
            id,
            request: Request::Save(Edit::Accent(Some(0x3ccfae))),
        }) = rx.try_recv()
        else {
            panic!("expected the accent to save");
        };
        // One save at a time.
        let _ = app.profile_fields_input(Input::Accent(None));
        assert!(rx.try_recv().is_err());
        app.profile_fields_reply(id + 100, Ok(Reply::Saved(Saved::Accent(Some(1)))));
        assert!(app.profile_fields.saving.is_some());
        app.profile_fields_reply(id, Ok(Reply::Saved(Saved::Accent(Some(0x3ccfae)))));
        assert_eq!(
            app.profile_fields.own.as_ref().unwrap().accent,
            Some(0x3ccfae)
        );

        // The About Me starts from the saved one.
        let _ = app.profile_fields_input(Input::Bio(text_editor::Action::Move(
            text_editor::Motion::DocumentEnd,
        )));
        let _ = app.profile_fields_input(Input::Bio(text_editor::Action::Edit(
            text_editor::Edit::Insert('!'),
        )));
        assert_eq!(app.profile_fields.bio_text(), "Hello!");
        let _ = app.profile_fields_input(Input::SaveBio);
        let Ok(Command::ProfileFields {
            id,
            request: Request::Save(Edit::Bio(Some(bio))),
        }) = rx.try_recv()
        else {
            panic!("expected the bio to save");
        };
        assert_eq!(bio, "Hello!");
        app.profile_fields_reply(id, Err(Failure::Failed));
        assert!(app.profile_fields.note.is_some());
        assert_eq!(app.profile_fields.bio_text(), "Hello!");
        // A load meanwhile keeps the edit.
        let me = app.matrix_id.clone();
        app.load_profile_fields(me);
        let Ok(Command::ProfileFields {
            id,
            request: Request::Load(user),
        }) = rx.try_recv()
        else {
            panic!("expected a load");
        };
        app.profile_fields_reply(
            id,
            Ok(Reply::Loaded {
                user,
                fields: Fields::default(),
                editable: Some(all),
            }),
        );
        assert_eq!(app.profile_fields.bio_text(), "Hello!");
        let _ = app.profile_fields_input(Input::SaveBio);
        let Ok(Command::ProfileFields { id, .. }) = rx.try_recv() else {
            panic!("expected the bio to save again");
        };
        app.profile_fields_reply(id, Ok(Reply::Saved(Saved::Bio(Some("Hello!".into())))));
        assert!(!app.profile_fields.bio_edited);
        assert_eq!(app.profile_fields.saved_bio(), "Hello!");
    }

    #[test]
    fn a_card_loads_someone_elses_fields() {
        let (mut app, rx) = app();
        app.load_profile_fields("@mira:example.org".into());
        app.load_profile_fields("@mira:example.org".into());
        let Ok(Command::ProfileFields {
            id,
            request: Request::Load(user),
        }) = rx.try_recv()
        else {
            panic!("expected a load");
        };
        assert!(rx.try_recv().is_err(), "one load at a time per person");
        app.profile_fields_reply(
            id,
            Ok(Reply::Loaded {
                user,
                fields: Fields {
                    banner: Some("mxc://h/banner".into()),
                    ..Fields::default()
                },
                editable: None,
            }),
        );
        let fields = app
            .profile_fields
            .of("@mira:example.org", &app.matrix_id)
            .unwrap();
        assert_eq!(fields.banner.as_deref(), Some("mxc://h/banner"));
        assert!(app.profile_fields.editable.is_none());
    }

    #[test]
    fn picker_tokens_name_their_picture() {
        assert_eq!(picker("profile-banner:7"), Some((true, 7)));
        assert_eq!(picker("server-picture:9"), Some((false, 9)));
        assert_eq!(picker("server-settings:9"), None);
        assert_eq!(picker(""), None);
    }
}
