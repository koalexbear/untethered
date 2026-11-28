//! Matrix profile changes and member-card actions shared by desktop and web.
use super::{Command, Failure};
use crate::{App, Msg, Section, media::LocalFile};
use iced::Task;
use matrix_sdk::{Client, ruma::UserId};

#[cfg(target_arch = "wasm32")]
pub(crate) const AVATAR_PICKER: &str = "profile-avatar:";
const AVATAR_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub enum Action {
    Open { room: String, user: String },
    Close,
    Message,
    Name(String),
    SaveName,
    PickAvatar,
    Picked(i64, Result<Option<Vec<u8>>, &'static str>),
    RemoveAvatar,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Edit {
    Name(String),
    Avatar(Option<Vec<u8>>),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Saved {
    Name(Option<String>),
    Avatar(Option<String>),
}

#[derive(Default, Clone)]
pub(super) struct Changes {
    pub name: Option<Option<String>>,
    pub avatar: Option<Option<String>>,
}

impl Changes {
    pub fn record(&mut self, saved: &Saved) {
        match saved {
            Saved::Name(name) => self.name = Some(name.clone()),
            Saved::Avatar(avatar) => self.avatar = Some(avatar.clone()),
        }
    }
}

#[derive(Default)]
pub struct Ui {
    pub card: Option<(String, String)>,
    pub name: Option<String>,
    pub note: Option<&'static str>,
    pub card_error: Option<&'static str>,
    pub saving: Option<i64>,
    pub picking: bool,
    picker_id: i64,
    dm_request: Option<(i64, String)>,
    dm_room: Option<(String, String)>,
}

impl Ui {
    pub fn dismiss_card(&mut self) {
        self.card = None;
        self.card_error = None;
        self.dm_room = None;
    }

    pub fn opening_dm(&self) -> bool {
        self.dm_request.is_some() || self.dm_room.is_some()
    }

    pub fn busy(&self) -> bool {
        self.saving.is_some() || self.picking
    }
}

pub(super) async fn save(client: &Client, edit: Edit) -> Result<Saved, Failure> {
    let account = client.account();
    match edit {
        Edit::Name(name) => {
            let name = (!name.is_empty()).then_some(name);
            account
                .set_display_name(name.as_deref())
                .await
                .map_err(|e| super::messaging::http(&e))?;
            Ok(Saved::Name(name))
        }
        Edit::Avatar(Some(bytes)) => {
            if bytes.len() as u64 > AVATAR_LIMIT {
                return Err(Failure::Failed);
            }
            let mime = image_mime(&bytes).ok_or(Failure::Failed)?;
            let uri = account
                .upload_avatar(&mime, bytes)
                .await
                .map_err(|e| super::messaging::http(&e))?;
            Ok(Saved::Avatar(Some(uri.to_string())))
        }
        Edit::Avatar(None) => {
            account
                .set_avatar_url(None)
                .await
                .map_err(|e| super::messaging::http(&e))?;
            Ok(Saved::Avatar(None))
        }
    }
}

/// Upload a picture as it is, for a server picture or a banner: its mxc URI.
pub(super) async fn upload(client: &Client, bytes: Vec<u8>) -> Result<String, Failure> {
    if bytes.len() as u64 > AVATAR_LIMIT {
        return Err(Failure::Failed);
    }
    let mime = image_mime(&bytes).ok_or(Failure::Failed)?;
    let response = client
        .media()
        .upload(&mime, bytes, None)
        .await
        .map_err(|e| {
            if e.client_api_error_kind().is_some() {
                Failure::Failed
            } else {
                Failure::Unreachable
            }
        })?;
    Ok(response.content_uri.to_string())
}

fn image_mime(bytes: &[u8]) -> Option<mime::Mime> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some(mime::IMAGE_PNG),
        image::ImageFormat::Jpeg => Some(mime::IMAGE_JPEG),
        image::ImageFormat::Gif => Some(mime::IMAGE_GIF),
        image::ImageFormat::WebP => "image/webp".parse().ok(),
        _ => None,
    }
}

pub(super) async fn direct(client: &Client, user: &str) -> Result<String, Failure> {
    let user = UserId::parse(user).map_err(|_| Failure::Failed)?;
    if client.user_id() == Some(user.as_ref()) {
        return Err(Failure::Unavailable);
    }
    let room = match client.get_dm_room(&user) {
        Some(room) => room,
        None => client
            .create_dm(&user)
            .await
            .map_err(|e| super::messaging::http(&e))?,
    };
    Ok(room.room_id().to_string())
}

impl App {
    pub(crate) fn profile_action(&mut self, action: Action) -> Task<Msg> {
        match action {
            Action::Open { room, user } if self.verified && self.live.list.contains_key(&room) => {
                // Their banner, About Me and accent colour (profile_fields.rs).
                self.load_profile_fields(user.clone());
                let ui = &mut self.sdk_ui.profile;
                ui.card = Some((room, user));
                ui.card_error = None;
            }
            Action::Close => self.sdk_ui.profile.dismiss_card(),
            Action::Message
                if self.verified && self.connected && !self.sdk_ui.profile.opening_dm() =>
            {
                if let Some((_, user)) = &self.sdk_ui.profile.card
                    && user != &self.matrix_id
                {
                    let user = user.clone();
                    if let Some(room) = self.direct_with(&user).cloned() {
                        self.sdk_ui.profile.dm_room = Some((room, user));
                        self.open_profile_dm();
                    } else if let Some(id) = self.queue_profile(|id| Command::DirectProfile {
                        id,
                        user: user.clone(),
                    }) {
                        self.sdk_ui.profile.dm_request = Some((id, user));
                        self.sdk_ui.profile.card_error = None;
                    } else {
                        self.sdk_ui.profile.card_error =
                            Some("Could not open a direct message. Try again.");
                    }
                }
            }
            Action::Name(name) if self.account && !self.sdk_ui.profile.busy() => {
                self.sdk_ui.profile.name = Some(name);
                self.sdk_ui.profile.note = None;
            }
            Action::SaveName if self.account => {
                if let Some(name) = &self.sdk_ui.profile.name {
                    self.save_profile(Edit::Name(name.trim().to_owned()));
                }
            }
            Action::RemoveAvatar if self.account => self.save_profile(Edit::Avatar(None)),
            Action::PickAvatar
                if self.account
                    && self.verified
                    && self.connected
                    && !self.sdk_ui.profile.busy() =>
            {
                self.seq += 1;
                let id = self.seq;
                let ui = &mut self.sdk_ui.profile;
                ui.note = None;
                ui.picker_id = id;
                ui.picking = true;
                #[cfg(target_arch = "wasm32")]
                crate::web_files::pick_avatar(&format!("{AVATAR_PICKER}{id}"));
                #[cfg(not(target_arch = "wasm32"))]
                {
                    return Task::perform(
                        async {
                            #[cfg(target_os = "linux")]
                            crate::app_id::portal_ready().await;
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
                                .pick_file()
                                .await
                            else {
                                return Ok(None);
                            };
                            let file = LocalFile::from_path(file.path().to_path_buf())
                                .ok_or("Could not read the selected image.")?;
                            avatar_bytes(file).await.map(Some)
                        },
                        move |result| msg(Action::Picked(id, result)),
                    );
                }
            }
            Action::Picked(id, result)
                if self.sdk_ui.profile.picking && id == self.sdk_ui.profile.picker_id =>
            {
                self.sdk_ui.profile.picking = false;
                if self.account {
                    match result {
                        Ok(Some(bytes)) => self.save_profile(Edit::Avatar(Some(bytes))),
                        Ok(None) => {}
                        Err(note) => self.sdk_ui.profile.note = Some(note),
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn profile_picked(&mut self, id: i64, file: Option<LocalFile>) -> Task<Msg> {
        let ui = &self.sdk_ui.profile;
        if !ui.picking || ui.picker_id != id {
            return Task::none();
        }
        if let Some(file) = file {
            Task::perform(avatar_bytes(file), move |result| {
                msg(Action::Picked(id, result.map(Some)))
            })
        } else {
            self.profile_action(Action::Picked(id, Ok(None)))
        }
    }

    fn queue_profile(&mut self, command: impl FnOnce(i64) -> Command) -> Option<i64> {
        if !self.connected || !self.verified {
            return None;
        }
        self.seq += 1;
        self.sdk.as_ref()?.try_send(command(self.seq)).ok()?;
        Some(self.seq)
    }

    fn save_profile(&mut self, edit: Edit) {
        if self.sdk_ui.profile.busy() {
            return;
        }
        if let Some(id) = self.queue_profile(|id| Command::SaveProfile { id, edit }) {
            self.sdk_ui.profile.saving = Some(id);
            self.sdk_ui.profile.note = None;
        } else {
            self.sdk_ui.profile.note = Some("Could not save your profile. Try again.");
        }
    }

    pub(crate) fn profile_saved(&mut self, id: i64, result: Result<Saved, Failure>) {
        let ui = &mut self.sdk_ui.profile;
        if ui.saving != Some(id) {
            return;
        }
        ui.saving = None;
        ui.note = match result {
            Ok(Saved::Name(name)) => {
                self.profile = name.unwrap_or_else(|| self.matrix_id.clone());
                ui.name = None;
                Some("Display name saved.")
            }
            Ok(Saved::Avatar(avatar)) => {
                self.own_avatar = avatar;
                Some("Profile picture saved.")
            }
            Err(_) => Some("Could not save your profile. Try again."),
        };
    }

    pub(crate) fn profile_direct(&mut self, id: i64, result: Result<String, Failure>) {
        let ui = &mut self.sdk_ui.profile;
        if !ui
            .dm_request
            .as_ref()
            .is_some_and(|(wanted, _)| *wanted == id)
        {
            return;
        }
        let (_, user) = ui.dm_request.take().unwrap();
        if !ui.card.as_ref().is_some_and(|(_, target)| target == &user) {
            return;
        }
        match result {
            Ok(room) => {
                ui.dm_room = Some((room, user));
                self.open_profile_dm();
            }
            Err(_) => ui.card_error = Some("Could not open a direct message. Try again."),
        }
    }

    pub(crate) fn open_profile_dm(&mut self) {
        let ui = &mut self.sdk_ui.profile;
        let Some((room, user)) = &ui.dm_room else {
            return;
        };
        if !ui.card.as_ref().is_some_and(|(_, target)| target == user) {
            ui.dm_room = None;
            return;
        }
        let Some(room_state) = self.live.list.get(room).filter(|room| room.has_metadata) else {
            return;
        };
        let dm = room_state.dm;
        let room = room.clone();
        ui.dm_room = None;
        ui.card = None;
        self.close_mentions();
        self.setup = None;
        self.section = if dm { Section::Direct } else { Section::All };
        self.open(room);
    }
}

pub(crate) fn msg(action: Action) -> Msg {
    Msg::Sdk(super::account_ui::Action::Profile(action))
}

/// A picked picture's bytes, when it's a PNG, JPEG, GIF or WebP within the
/// limits: for the profile picture, a server picture or a banner.
pub(crate) async fn avatar_bytes(file: LocalFile) -> Result<Vec<u8>, &'static str> {
    const READ: &str = "Could not read the selected image.";
    if file.size > AVATAR_LIMIT {
        return Err("Choose an image smaller than 8 MiB.");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = {
        use tokio::io::AsyncReadExt;
        let path = file.origin.read_path().ok_or(READ)?;
        let f = tokio::fs::File::open(path).await.map_err(|_| READ)?;
        let mut bytes = Vec::new();
        f.take(AVATAR_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| READ)?;
        bytes
    };
    #[cfg(target_arch = "wasm32")]
    let bytes = {
        let buffer = wasm_bindgen_futures::JsFuture::from(file.file.array_buffer())
            .await
            .map_err(|_| READ)?;
        js_sys::Uint8Array::new(&buffer).to_vec()
    };
    if bytes.len() as u64 > AVATAR_LIMIT {
        return Err("Choose an image smaller than 8 MiB.");
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| READ)?;
    if image_mime(&bytes).is_none()
        || !reader
            .into_dimensions()
            .is_ok_and(|(width, height)| width > 0 && height > 0 && width <= 4096 && height <= 4096)
    {
        return Err("Choose a PNG, JPEG, GIF, or WebP image up to 4096 × 4096.");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{RoomList, RoomSummary};

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (backend, _) = async_channel::bounded(1);
        let (sdk, rx) = async_channel::bounded(8);
        let mut app = App::new(backend);
        app.sdk = Some(sdk);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.matrix_id = "@me:example.org".into();
        app.profile = "Old name".into();
        app.own_avatar = Some("mxc://example.org/old".into());
        (app, rx)
    }

    #[test]
    fn profile_updates_only_after_the_matching_acknowledgement() {
        let (mut app, rx) = app();
        app.account = true;
        let _ = app.profile_action(Action::Name(" New name ".into()));
        let _ = app.profile_action(Action::SaveName);
        let Command::SaveProfile {
            id,
            edit: Edit::Name(name),
        } = rx.try_recv().unwrap()
        else {
            panic!("expected a name update");
        };
        assert_eq!(name, "New name");
        assert_eq!(app.profile, "Old name");
        app.profile_saved(id + 1, Ok(Saved::Name(Some("Wrong request".into()))));
        assert_eq!(app.profile, "Old name");
        app.profile_saved(id, Err(Failure::Failed));
        assert_eq!(app.profile, "Old name");
        assert_eq!(app.sdk_ui.profile.name.as_deref(), Some(" New name "));

        let _ = app.profile_action(Action::SaveName);
        let Command::SaveProfile {
            id,
            edit: Edit::Name(name),
        } = rx.try_recv().unwrap()
        else {
            panic!("expected a name retry");
        };
        app.profile_saved(id, Ok(Saved::Name(Some(name))));
        assert_eq!(app.profile, "New name");
        assert!(app.sdk_ui.profile.name.is_none());

        let _ = app.profile_action(Action::RemoveAvatar);
        let Command::SaveProfile {
            id,
            edit: Edit::Avatar(None),
        } = rx.try_recv().unwrap()
        else {
            panic!("expected an avatar removal");
        };
        assert!(app.own_avatar.is_some());
        app.profile_saved(id, Ok(Saved::Avatar(None)));
        assert!(app.own_avatar.is_none());
    }

    #[test]
    fn repeated_message_clicks_do_not_make_another_dm() {
        let (mut app, rx) = app();
        app.account = false;
        app.section = Section::All;
        let source = RoomSummary {
            id: "!source:example.org".into(),
            name: "Chat".into(),
            ..RoomSummary::default()
        };
        app.selected = source.id.clone();
        app.live.apply_engine(RoomList {
            invites: Vec::new(),
            rooms: vec![source.clone()],
            ..RoomList::default()
        });
        let open_card = Action::Open {
            room: source.id.clone(),
            user: "@other:example.org".into(),
        };
        let _ = app.profile_action(open_card.clone());
        // Opening the card loads their banner and About Me (M15).
        assert!(matches!(
            rx.try_recv(),
            Ok(Command::ProfileFields { request: crate::engine::profile_fields::Request::Load(user), .. })
                if user == "@other:example.org"
        ));
        let _ = app.profile_action(Action::Message);
        let Command::DirectProfile { id, user } = rx.try_recv().unwrap() else {
            panic!("expected a direct message request");
        };
        let _ = app.profile_action(Action::Message);
        assert!(rx.try_recv().is_err());

        let dm = RoomSummary {
            id: "!dm:example.org".into(),
            name: "Other".into(),
            dm: true,
            dm_peer: Some(user.clone()),
            encrypted: true,
            ..RoomSummary::default()
        };
        app.profile_direct(id, Ok(dm.id.clone()));
        assert_eq!(app.selected, source.id);
        let _ = app.profile_action(Action::Message);
        assert!(rx.try_recv().is_err());

        app.live.apply_engine(RoomList {
            invites: Vec::new(),
            rooms: vec![source.clone(), dm.clone()],
            ..RoomList::default()
        });
        app.open_profile_dm();
        assert_eq!(app.selected, dm.id);
        assert!(matches!(app.section, Section::Direct));
        while rx.try_recv().is_ok() {}

        let _ = app.profile_action(open_card);
        while rx.try_recv().is_ok() {}
        let _ = app.profile_action(Action::Message);
        assert_eq!(app.selected, dm.id);
        assert!(rx.try_recv().is_err());
    }
}
