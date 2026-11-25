#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
// matrix-sdk futures nest deeply enough to exceed the default limit when
// rustc proves they are Send.
#![recursion_limit = "256"]
mod account;
mod backend;
#[cfg(target_arch = "wasm32")]
mod browser_verification;
#[cfg(target_os = "linux")]
mod call_devices;
mod community;
#[cfg(target_os = "linux")]
mod desktop_call;
mod edits;
#[cfg(not(target_arch = "wasm32"))]
mod engine;
#[cfg(not(target_arch = "wasm32"))]
mod fonts;
#[cfg(target_os = "linux")]
mod incoming;
mod inline;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod live_test;
mod media;
mod mentions;
mod moderation;
#[cfg(any(target_os = "linux", target_arch = "wasm32"))]
mod notifications;
#[cfg(not(target_arch = "wasm32"))]
mod paste;
mod permissions;
mod preview;
mod reactions;
mod reading;
mod replies;
mod reports;
mod room_state;
mod rooms;
mod search;
mod transfers;
mod view;
#[cfg(target_os = "linux")]
mod wayland_drop;
#[cfg(target_arch = "wasm32")]
mod web_files;

use async_channel::Sender;
use iced::{
    Task, Theme,
    widget::{operation, text_editor},
};
use rooms::{Rooms, items};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
enum Section {
    All,
    Direct,
    Space(String),
}
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
enum Field {
    Server,
    Username,
    Password,
    Composer(String),
}
#[derive(Clone)]
enum Msg {
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    NotificationPanel,
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    NotifyMode(String, notifications::Mode),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    SetDnd(bool),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    SpaceAlerts,
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    SpaceMode(String, notifications::Mode),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    RetrySpaceRules,
    #[cfg(not(target_arch = "wasm32"))]
    WindowFocus(bool),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    DesktopDelivery(bool),
    #[cfg(target_arch = "wasm32")]
    AllowBrowserAlerts,
    #[cfg(target_arch = "wasm32")]
    BrowserPermissionDone,
    #[cfg(target_arch = "wasm32")]
    OpenBrowserAlert(String, String),
    #[cfg(target_os = "linux")]
    WaylandDrop(async_channel::Receiver<smithay_clipboard::FileDrop>),
    #[cfg(target_os = "linux")]
    WaylandDropResolved(u64, String, wayland_drop::Result),
    #[cfg(not(target_arch = "wasm32"))]
    PasteComposer,
    #[cfg(not(target_arch = "wasm32"))]
    ClipboardReady(u64, String, paste::Result),
    HoverFiles(bool),
    #[cfg(not(target_arch = "wasm32"))]
    DropPaths(Vec<std::path::PathBuf>),
    Files(String, Vec<media::LocalFile>),
    Thumbnail(String, u64, Option<iced::widget::image::Handle>),
    PickFiles,
    RemoveFile(u64),
    DismissSkipped(String),
    CancelUpload(String, u64, i64),
    SaveFile(media::Media),
    InlineReady(u64, media::PreviewRef, Option<iced::widget::image::Handle>),
    ShowInline(media::PreviewRef),
    #[cfg(not(target_arch = "wasm32"))]
    SaveTo(media::Media, Option<std::path::PathBuf>),
    CheckCalls,
    #[cfg(target_os = "linux")]
    DialDm(bool),
    #[cfg(target_os = "linux")]
    JoinVoice,
    #[cfg(target_os = "linux")]
    EndCall,
    #[cfg(target_os = "linux")]
    ToggleMic,
    #[cfg(target_os = "linux")]
    ToggleDeafen,
    #[cfg(target_os = "linux")]
    SetShareSound(uuid::Uuid, bool),
    #[cfg(target_os = "linux")]
    ShareApp(uuid::Uuid, uuid::Uuid, Option<call_devices::AppStream>),
    #[cfg(target_os = "linux")]
    RoomPtt(bool),
    #[cfg(target_os = "linux")]
    TalkKey(bool),
    #[cfg(target_os = "linux")]
    ToggleVideo(desktop_call::VideoInput),
    #[cfg(target_os = "linux")]
    FocusVideo(uuid::Uuid, Option<String>),
    #[cfg(target_os = "linux")]
    ReturnToCall(uuid::Uuid),
    #[cfg(target_os = "linux")]
    CallLayout(uuid::Uuid, bool),
    #[cfg(target_os = "linux")]
    HideCallPreview(uuid::Uuid, bool),
    #[cfg(target_os = "linux")]
    AudioDevices,
    #[cfg(target_os = "linux")]
    ListCameras,
    #[cfg(target_os = "linux")]
    CamerasListed(
        uuid::Uuid,
        Result<Vec<desktop_call::CameraChoice>, &'static str>,
    ),
    #[cfg(target_os = "linux")]
    SelectCamera(uuid::Uuid, desktop_call::CameraChoice),
    #[cfg(target_os = "linux")]
    ToggleMixer,
    #[cfg(target_os = "linux")]
    PeerVolume(uuid::Uuid, String, u8),
    #[cfg(target_os = "linux")]
    RefreshDevices,
    #[cfg(target_os = "linux")]
    DevicesListed(uuid::Uuid, Result<call_devices::Catalog, &'static str>),
    #[cfg(target_os = "linux")]
    SelectDevice(call_devices::Port, call_devices::Choice),
    #[cfg(target_os = "linux")]
    DecideRing(String, bool),
    #[cfg(target_os = "linux")]
    DismissCallNotice,
    #[cfg(target_os = "linux")]
    RingExpired(String),
    Frame(Value),
    #[cfg(not(target_arch = "wasm32"))]
    Engine(engine::Event),
    #[cfg(not(target_arch = "wasm32"))]
    Sdk(engine::account_ui::Action),
    Stopped(String),
    Section(Section),
    Open(String),
    Setup(community::Input),
    CheckReading,
    ReadingScroll(String, bool),
    ShareReads(bool),
    ReadVisible(String, String, String, bool),
    MarkRoom(String, Option<String>),
    OpenMentions,
    MoreMentions,
    MentionHit(String, String),
    Draft(text_editor::Action),
    Send,
    Reply(String),
    Report(String, String),
    ReportReason(String),
    SubmitReport,
    CancelReport,
    ReactionPicker(String),
    ReactionInput(String),
    ToggleReaction(reactions::Toggle),
    Edit(String),
    ChangeEdit(text_editor::Action),
    SaveEdit,
    CancelEdit,
    CancelReply,
    Jump(String),
    Latest,
    Preceding,
    OpenSearch,
    SearchReady(String, search::Bookmark),
    SearchInput(String),
    RunSearch(bool),
    SearchCoverage,
    SearchHit(String),
    SearchResults,
    ReturnToReading,
    More,
    TimelineEdge(bool),
    TimelineRestored,
    MoveTimeline {
        room: String,
        from: Option<(i64, i64)>,
        to: Option<(i64, i64)>,
        bookmark: search::bookmark::Bookmark,
    },
    Copy,
    TogglePeople,
    /// Open or close the server dropdown in the channel sidebar.
    ServerMenu(bool),
    /// Close the topmost menu, dialog or settings page.
    Escape,
    /// Collapse or expand a channel category, keyed by section and kind.
    Collapse(String),
    MemberFilter(String),
    SelectMember(String),
    MemberReason(String),
    RefreshMembers,
    Moderate(moderation::Change),
    Permissions,
    SetPosting(permissions::Change),
    Account,
    Back,
    Server(String),
    Username(String),
    Password(String),
    Login,
    #[cfg(not(target_arch = "wasm32"))]
    Recovery(String),
    #[cfg(not(target_arch = "wasm32"))]
    Verify,
    #[cfg(target_arch = "wasm32")]
    SubmitWebRecovery(String),
    Tab(bool),
    Close,
    #[cfg(target_arch = "wasm32")]
    WebPaste(Field),
    #[cfg(target_arch = "wasm32")]
    Pasted(Field, String),
    #[cfg(target_arch = "wasm32")]
    ClipboardResult(bool),
}
// Iced requires Debug on messages. Never put passwords, keys or RPC data in debug output.
impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UI({:?})", std::mem::discriminant(self))
    }
}
enum Request {
    Setup(community::Step),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    NotifyRule(notifications::Change),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    SpacePref(String),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    SpaceRule(notifications::spaces::Write),
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    Dnd(bool, u64),
    #[cfg(target_os = "linux")]
    RingState(incoming::Offer),
    #[cfg(target_os = "linux")]
    DeclineSent,
    Upload(String, u64),
    MediaSend(String, u64),
    Download,
    MediaConfig,
    RtcDiscovery,
    Login,
    Verify(account::RecoveryFormat),
    VerifyState,
    Page(String),
    Members(String),
    Send(String, String, Option<rooms::Quote>),
    Context(String, String),
    Posting(permissions::Flight),
    Membership(moderation::Flight),
    Edit(String, String),
    Reaction(reactions::Toggle),
    Report,
    Search(String, search::Page),
    Mentions(mentions::Page),
    ReadState(String, Option<String>),
    ShareReads(bool, u64),
}
struct App {
    setup: Option<community::Form>,
    read_sharing: reading::Sharing,
    scrolled_away: bool,
    #[cfg(not(target_arch = "wasm32"))]
    focused: bool,
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    alerts: notifications::Alerts,
    #[cfg(target_os = "linux")]
    inbox: incoming::Inbox,
    #[cfg(target_os = "linux")]
    call: Option<desktop_call::ActiveCall>,
    #[cfg(target_os = "linux")]
    room_ptt: bool,
    #[cfg(target_os = "linux")]
    drop_epoch: u64,
    #[cfg(not(target_arch = "wasm32"))]
    paste_seq: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pasting: bool,
    commands: Sender<backend::Command>,
    /// Set when UNTETHERED_ENGINE=matrix-sdk replaces gomuks.
    #[cfg(not(target_arch = "wasm32"))]
    sdk: Option<Sender<engine::Command>>,
    #[cfg(not(target_arch = "wasm32"))]
    sdk_ui: engine::account_ui::AccountUi,
    outbox: media::Outbox,
    inline: inline::InlineImages,
    batch: Option<transfers::Batch>,
    hover_files: bool,
    media_limit: u64,
    rtc_status: String,
    pending: BTreeMap<i64, Request>,
    seq: i64,
    live: Rooms,
    demo: Rooms,
    section: Section,
    selected: String,
    drafts: BTreeMap<String, String>,
    reply_drafts: BTreeMap<String, rooms::Quote>,
    original: Option<replies::Original>,
    // Exclusive timestamp/row-ID boundary. None follows the latest message.
    timeline_end: Option<(i64, i64)>,
    timeline_restoring: bool,
    search: Option<search::Search>,
    mentions: Option<mentions::Inbox>,
    editing: Option<edits::Editing>,
    reporting: Option<reports::Draft>,
    reaction_picker: Option<reactions::Picker>,
    editor: text_editor::Content,
    loaded: BTreeSet<String>,
    connected: bool,
    logged_in: bool,
    verified: bool,
    account: bool,
    people: bool,
    server_menu: bool,
    collapsed: BTreeSet<String>,
    directory: moderation::Directory,
    permission_panel: bool,
    profile: String,
    matrix_id: String,
    status: String,
    server: String,
    username: String,
    password: String,
    recovery: String,
    verification_note: String,
    recovery_unavailable: bool,
    closing: bool,
    stopped: bool,
}
impl App {
    fn boot() -> (Self, Task<Msg>) {
        #[cfg(not(target_arch = "wasm32"))]
        if engine::selected() {
            return Self::boot_engine();
        }
        let (commands, rx) = async_channel::bounded(64);
        let (tx, events) = async_channel::bounded(64);
        let task = Task::batch([
            Task::perform(backend::run(rx, tx), |result| {
                Msg::Stopped(result.err().unwrap_or_else(|| "gomuks closed.".into()))
            }),
            Task::run(events, Msg::Frame),
        ]);
        #[cfg(target_arch = "wasm32")]
        let task = Task::batch([
            task,
            browser_verification::listen(),
            web_files::listen(),
            notifications::browser::listen(),
        ]);
        #[cfg(target_os = "linux")]
        let task = Task::batch([task, wayland_drop::listen()]);
        (Self::new(commands), task)
    }
    fn new(commands: Sender<backend::Command>) -> Self {
        Self {
            setup: None,
            read_sharing: Default::default(),
            scrolled_away: false,
            #[cfg(not(target_arch = "wasm32"))]
            focused: false,
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            alerts: Default::default(),
            #[cfg(target_os = "linux")]
            call: None,
            #[cfg(target_os = "linux")]
            room_ptt: false,
            #[cfg(target_os = "linux")]
            inbox: Default::default(),
            #[cfg(target_os = "linux")]
            drop_epoch: 0,
            #[cfg(not(target_arch = "wasm32"))]
            paste_seq: 0,
            #[cfg(not(target_arch = "wasm32"))]
            pasting: false,
            commands,
            #[cfg(not(target_arch = "wasm32"))]
            sdk: None,
            #[cfg(not(target_arch = "wasm32"))]
            sdk_ui: Default::default(),
            outbox: Default::default(),
            inline: Default::default(),
            batch: None,
            hover_files: false,
            media_limit: media::MAX_BYTES,
            rtc_status: String::new(),
            pending: Default::default(),
            seq: 0,
            live: Default::default(),
            demo: Rooms::demo(),
            section: Section::Space("!workshop".into()),
            selected: "!general".into(),
            drafts: Default::default(),
            reply_drafts: Default::default(),
            original: None,
            timeline_end: None,
            timeline_restoring: false,
            search: None,
            mentions: None,
            editing: None,
            reporting: None,
            reaction_picker: None,
            editor: text_editor::Content::new(),
            loaded: Default::default(),
            connected: false,
            logged_in: false,
            verified: false,
            account: false,
            people: false,
            server_menu: false,
            collapsed: Default::default(),
            directory: Default::default(),
            permission_panel: false,
            profile: String::new(),
            matrix_id: String::new(),
            status: "Starting gomuks…".into(),
            server: String::new(),
            username: String::new(),
            password: String::new(),
            recovery: String::new(),
            verification_note: String::new(),
            recovery_unavailable: false,
            closing: false,
            stopped: false,
        }
    }
    fn rooms(&self) -> &Rooms {
        if self.logged_in {
            &self.live
        } else {
            &self.demo
        }
    }
    fn timeline_range(&self, room: &rooms::Room) -> std::ops::Range<usize> {
        const WINDOW: usize = 120;
        let end = self
            .timeline_end
            .map_or_else(|| room.posts().len(), |key| room.index_at(key));
        end.saturating_sub(WINDOW)..end
    }

    fn visible(&self) -> impl Iterator<Item = (&String, &rooms::Room)> {
        let rooms = self.rooms();
        let ids: &[String] = match &self.section {
            Section::All => &rooms.ordered_rooms,
            Section::Direct => &rooms.ordered_direct,
            Section::Space(space) => rooms.ordered_spaces.get(space).map_or(&[], Vec::as_slice),
        };
        ids.iter()
            .filter_map(|id| rooms.list.get(id).map(|room| (id, room)))
    }
    fn rpc(&mut self, cmd: &str, data: Value, req: Request) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            return self.engine_rpc(cmd, data, req);
        }
        self.seq += 1;
        if self.connected
            && self
                .commands
                .try_send(backend::Command::Rpc(
                    json!({"command":cmd,"request_id":self.seq,"data":data}),
                ))
                .is_ok()
        {
            self.pending.insert(self.seq, req);
        } else {
            self.status = "Could not queue the request. Try again.".into();
        }
    }
    fn page(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.sdk.is_some() {
            return self.engine_page();
        }
        let id = self.selected.clone();
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self
                .pending
                .values()
                .any(|p| matches!(p,Request::Page(r) if r==&id))
        {
            return;
        }
        if let Some(r) = self.live.list.get(&id).filter(|r| !r.end) {
            self.rpc(
                "paginate",
                json!({"room_id":id,"limit":50,"max_timeline_id":r.cursor.unwrap_or(0)}),
                Request::Page(id),
            );
        }
    }
    #[cfg(target_os = "linux")]
    fn minimize_call(&mut self) {
        if let Some(call) = &mut self.call {
            call.stage_open = false;
            call.theater = false;
        }
    }

    fn open(&mut self, id: String) {
        if self.selected == id {
            return;
        }
        #[cfg(target_os = "linux")]
        self.minimize_call();
        self.close_original();
        self.scrolled_away = false;
        self.reset_directory();
        self.leave_permission_panel();
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        {
            self.alerts.panel = false;
        }
        self.clear_search();
        self.editing = None;
        self.drafts
            .insert(self.selected.clone(), self.editor.text());
        self.editor =
            text_editor::Content::with_text(self.drafts.get(&id).map(String::as_str).unwrap_or(""));
        self.selected = id.clone();
        self.timeline_end = None;
        if self.people {
            self.read_directory(None);
        }
        if self.logged_in && self.verified && !id.is_empty() && self.loaded.insert(id.clone()) {
            self.page();
            self.rpc(
                "get_room_state",
                json!({"room_id":id,"include_members":true}),
                Request::Members(id),
            );
        } else if !self.logged_in
            && let Some(r) = self.demo.list.get_mut(&id)
        {
            r.unread = false;
        }
    }
    fn reconcile(&mut self) {
        if matches!(&self.section,Section::Space(s) if !self.rooms().list.contains_key(s)) {
            self.section = Section::All;
        }
        if !self.visible().any(|(id, _)| id == &self.selected) {
            let first = self
                .visible()
                .next()
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            self.open(first);
        }
    }
    fn verifying(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Verify(_) | Request::VerifyState))
    }
    fn verification_feedback(&self) -> &str {
        if !self.connected {
            &self.status
        } else if self.verifying() {
            "Verifying with your homeserver…"
        } else if !self.verification_note.is_empty() {
            &self.verification_note
        } else if self.recovery_unavailable {
            "gomuks reports that recovery isn’t set up for this account. Check recovery in a trusted Matrix client."
        } else {
            "Use the key or passphrase from Matrix recovery setup. Your sign-in password is separate. Grouping spaces are fine."
        }
    }
    fn submit_verification(&mut self) {
        if !self.logged_in
            || self.verified
            || !self.connected
            || self.verifying()
            || self.recovery_unavailable
        {
            return;
        }
        if self.recovery.trim().is_empty() {
            self.verification_note = "Enter your recovery key or passphrase first.".into();
            return;
        }
        self.verification_note.clear();
        let (format, key) = account::recovery_input(&self.recovery);
        self.rpc(
            "verify",
            json!({"recovery_key":key}),
            Request::Verify(format),
        );
        if !self.verifying() {
            self.verification_note.clone_from(&self.status);
        }
    }
    fn frame(&mut self, f: Value) {
        let data = &f["data"];
        match f["command"].as_str().unwrap_or("") {
            #[cfg(target_os = "linux")]
            "share_audio_status" => {
                if let Some(c) = &mut self.call
                    && data["room"] == c.room
                    && c.audio
                        .borrow()
                        .send_app
                        .as_ref()
                        .is_some_and(|(id, _)| data["attempt"] == id.to_string())
                {
                    c.app_note = rooms::string(&data["label"]);
                    if data["live"] != true {
                        c.audio.send_modify(|m| m.send_app = None);
                    }
                }
            }
            #[cfg(target_os = "linux")]
            "room_video" => {
                let Some(input) = desktop_call::VideoInput::from_name(data["source"].as_str())
                else {
                    return;
                };
                let slot = input as usize;
                if let Some(call) = &mut self.call
                    && data["room"] == call.room
                    && call.audio.borrow().video[slot]
                        .is_some_and(|id| data["request"] == id.to_string())
                {
                    call.video_status[slot].label = rooms::string(&data["label"]);
                    call.video_status[slot].live = data["live"] == true;
                    if !call.video_status[slot].live {
                        call.audio.send_modify(|m| {
                            m.video[slot] = None;
                            if input == desktop_call::VideoInput::Window {
                                m.send_app = None;
                            }
                        });
                        if input == desktop_call::VideoInput::Window {
                            call.app_note.clear();
                        }
                    }
                }
            }
            #[cfg(target_os = "linux")]
            "call_device_status" => {
                if let Some(call) = &mut self.call
                    && data["room"] == call.room
                {
                    let mode = call.audio.borrow();
                    let failed = data["failed"] == true;
                    if data["mic"] == true && data["revision"] == mode.mic.revision {
                        call.mic_error = failed;
                    }
                    if data["mic"] == false && data["revision"] == mode.speaker.revision {
                        call.speaker_error = failed;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            "desktop_call" => {
                if let Some(call) = &mut self.call
                    && data["room"] == call.room
                {
                    call.label = rooms::string(&data["label"]);
                    if data["ended"] == true {
                        self.status = call.label.clone();
                        self.call = None;
                        if self.closing {
                            self.commands.close();
                        }
                    }
                }
            }
            "upload_progress" => {
                if let (Some(id), Some(fraction)) = (f["request_id"].as_i64(), data.as_f64()) {
                    self.upload_progress(id, fraction);
                }
            }
            #[cfg(target_arch = "wasm32")]
            "profile-waiting" => {
                self.status = "Another Untethered tab is using this Matrix device. Use that tab, or close it and this one will connect automatically.".into();
            }
            #[cfg(target_arch = "wasm32")]
            "profile-opening" => {
                self.status = "Opening your browser’s Matrix device…".into();
            }
            "client_state" => {
                let vs = &data["verification_state"];
                self.apply_account(account::State {
                    logged_in: data["is_logged_in"] == true,
                    verified: data["is_verified"] == true,
                    recovery_unavailable: vs["state_checked"] == true && vs["has_ssss"] == false,
                    profile: data["displayname"]
                        .as_str()
                        .or(data["user_id"].as_str())
                        .unwrap_or("")
                        .into(),
                    user_id: rooms::string(&data["user_id"]),
                });
            }
            "sync_complete" => {
                if data["clear_state"] == true {
                    self.close_mentions();
                    self.reset_directory();
                    self.leave_permission_panel();
                    self.close_original();
                    self.clear_search();
                    #[cfg(target_os = "linux")]
                    self.clear_ring();
                    self.loaded.clear();
                    self.timeline_end = None;
                    self.selected.clear();
                    self.pending
                        .retain(|_, p| !matches!(p, Request::Members(_) | Request::Page(_)));
                }
                if data["rooms"][&self.selected]["reset"] == true {
                    self.timeline_end = None;
                }
                self.live.sync(data);
                self.read_sharing.sync(data);
                if self.mentions.is_some()
                    && (items(&data["left_rooms"]).next().is_some()
                        || data["rooms"]
                            .as_object()
                            .is_some_and(|rs| rs.values().any(|d| self.mention_delta(d))))
                {
                    self.refresh_mentions();
                }
                #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                self.notifications_sync(data);
                if let Some(rooms) = data["rooms"].as_object() {
                    for (room, delta) in rooms {
                        if self.people && *room == self.selected {
                            self.directory.sync(room, delta);
                        }
                        if let Some(c) = self
                            .original
                            .as_mut()
                            .filter(|c| c.room == *room && c.slice.is_some())
                        {
                            for e in items(&delta["events"]) {
                                c.refresh(room, e);
                            }
                        }
                        #[cfg(target_os = "linux")]
                        if data["clear_state"] != true && data["catchup"] != true {
                            self.ring_events(room, delta, false);
                        }
                        if self.outbox.rooms.contains_key(room) {
                            for e in items(&delta["events"]).filter(|e| rooms::server_confirmed(e))
                            {
                                self.media_delivery(e, true);
                            }
                        }
                    }
                }
                for left in items(&data["left_rooms"]).filter_map(Value::as_str) {
                    #[cfg(target_os = "linux")]
                    if self
                        .inbox
                        .ringing
                        .as_ref()
                        .is_some_and(|(i, _)| i.room == left)
                        || self.inbox.handled.as_ref().is_some_and(|(r, _)| r == left)
                    {
                        self.clear_ring()
                    }
                    self.outbox.rooms.remove(left);
                    self.outbox.skipped.remove(left);
                    self.reply_drafts.remove(left);
                    self.pending.retain(|_,p|!matches!(p,Request::Upload(room,_)|Request::MediaSend(room,_) if room==left));
                    if self.batch.as_ref().is_some_and(|b| b.room == left) {
                        self.batch = None;
                    }
                }
                self.reconcile();
            }
            "events_decrypted" => {
                if self.mentions.is_some() && self.mention_delta(data) {
                    self.refresh_mentions();
                }
                #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                if let Some(room) = data["room_id"].as_str() {
                    self.room_alerts(room, data, true);
                    #[cfg(target_os = "linux")]
                    self.ring_events(room, data, true);
                }
                for e in items(&data["events"]) {
                    self.live.event(&data["room_id"], e);
                    if let Some(c) = &mut self.original
                        && let Some(room) = data["room_id"].as_str()
                    {
                        c.refresh(room, e);
                    }
                }
            }
            "send_complete" => {
                let e = &data["event"];
                let accepted = rooms::server_confirmed(e)
                    || e["room_id"]
                        .as_str()
                        .and_then(|id| self.live.list.get(id))
                        .is_some_and(|r| r.already_confirmed(e));
                let ok = accepted || (data["error"].is_null() && !rooms::real_send_error(e));
                if !ok && !rooms::real_send_error(e) && e.is_object() {
                    let mut failed = e.clone();
                    failed["send_error"] = json!("delivery failed");
                    self.live.event(&Value::Null, &failed);
                } else {
                    self.live.event(&Value::Null, e);
                }
                self.media_delivery(e, ok);
            }
            "response" | "error" => {
                let Some(p) = self.pending.remove(&f["request_id"].as_i64().unwrap_or(0)) else {
                    return;
                };
                if f["command"] == "error" {
                    self.status = match p {
                        #[cfg(any(target_os="linux",target_arch="wasm32"))]
                        Request::NotifyRule(_)=>"Could not save the room notification setting. Try again.",
                        #[cfg(any(target_os="linux",target_arch="wasm32"))]
                        Request::SpacePref(_)=> "Could not save space alerts. Try again.",
                        #[cfg(any(target_os="linux",target_arch="wasm32"))]
                        Request::SpaceRule(w)=>{self.finish_space_rule(w,false);self.sync_space_rules();return}
                        #[cfg(any(target_os="linux",target_arch="wasm32"))]
                        Request::Dnd(..)=>"Could not change Do Not Disturb. Check the connection and try again.",
                        #[cfg(target_os="linux")]
                        Request::RingState(i)=>{self.present_ring(i,&Value::Null);return}
                        #[cfg(target_os="linux")]
                        Request::DeclineSent=>"Could not notify the caller that you declined.",
                        Request::MediaSend(ref room,id) if data.as_str()==Some("event was already sent successfully") &&
                            self.outbox.get_mut(room,id).is_some_and(|a|a.txn.is_some()) => {
                            self.finish_delivery(room,id,true);return
                        }
                        Request::Upload(room,id) | Request::MediaSend(room,id) => {
                            self.fail_file(&room,id,"Transfer failed. Retry or remove the attachment.");return
                        }
                        Request::MediaConfig=>return,
                        Request::Download=>"Could not save the attachment.",
                        Request::RtcDiscovery=>{self.rtc_status="MatrixRTC discovery failed or is unsupported on this homeserver.".into();return}
                        Request::Login => {
                            "Sign-in failed. Check your homeserver, username, and password."
                        }
                        Request::Verify(format) => {
                            self.verification_note = account::verify_error(data,format).into();
                            return;
                        }
                        Request::VerifyState => {
                            self.verification_note = "gomuks finished verification, but the device status couldn’t be refreshed.".into();
                            return;
                        }
                        Request::Page(_) => {
                            "Could not load history. Use Earlier messages to retry."
                        }
                        Request::Members(_) => "Could not load room members.",
                        Request::Posting(_)=> "Could not change posting permissions. Check your room role and connection.",
                        Request::Membership(f)=>{self.membership_error(f);return}
                        Request::Report=>{self.report_result(false);return}
                        Request::Setup(step)=>{self.setup_error(step);return}
                        Request::Send(..) => "Could not send. Your draft has been kept.",
                        Request::Context(..)=>"Could not load the selected message. Try opening it again.",
                        Request::Edit(room,id)=>{self.finish_edit(f["request_id"].as_i64().unwrap(),&room,&id,&Value::Null);return}
                        Request::Reaction(op)=>{self.finish_reaction(op,&Value::Null);return}
                        Request::Search(room,kind)=>{self.fail_search(&room,kind);return}
                        Request::Mentions(_)=>{self.mentions_failed();return}
                        Request::ReadState(room,event)=>{self.read_result(room,event,false);return}
                        Request::ShareReads(..)=>"Could not save your read receipt preference. Try again.",
                    }
                    .into();
                    return;
                }
                match p {
                    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                    Request::NotifyRule(c) => self.saved_notifications(c),
                    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                    Request::SpacePref(_) => {}
                    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                    Request::SpaceRule(w) => self.finish_space_rule(w, true),
                    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                    Request::Dnd(on, revision) => self.saved_dnd(on, revision),
                    #[cfg(target_os = "linux")]
                    Request::RingState(i) => {
                        self.present_ring(i, &crate::desktop_call::gomuks_room_state(data))
                    }
                    #[cfg(target_os = "linux")]
                    Request::DeclineSent => self.status = "Call declined.".into(),
                    Request::MediaConfig => {
                        if let Some(n) = data["m.upload.size"].as_u64().filter(|n| *n > 0) {
                            self.media_limit = media::MAX_BYTES.min(n);
                            self.outbox.apply_limit(self.media_limit);
                        }
                    }
                    Request::Upload(room, id) => self.uploaded(&room, id, data),
                    Request::MediaSend(room, id) => self.file_sent(&room, id, data),
                    Request::Download => self.status = "Attachment saved.".into(),
                    Request::RtcDiscovery => {
                        let found: Vec<Value> = items(&data["rtc_transports"]).cloned().collect();
                        self.rtc_discovered(&found)
                    }
                    Request::Context(room, id) => self.show_context(room, id, data),
                    Request::Posting(f) => self.posting_result(f, data),
                    Request::Membership(f) => self.membership_result(f, data),
                    Request::Report => self.report_result(true),
                    Request::Setup(step) => self.setup_result(step, data),
                    Request::Reaction(op) => self.finish_reaction(op, data),
                    Request::Search(room, kind) => self.search_response(room, kind, data),
                    Request::Mentions(kind) => self.mentions_page(kind, data),
                    Request::ReadState(room, event) => self.read_result(room, event, true),
                    Request::ShareReads(value, rev) => self.read_sharing.saved(value, rev),
                    Request::Edit(room, id) => {
                        self.finish_edit(f["request_id"].as_i64().unwrap(), &room, &id, data)
                    }
                    Request::Send(id, sent, reply) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            r.insert(data);
                        }
                        let same = self.reply_drafts.get(&id) == reply.as_ref();
                        if same && self.selected == id && self.editor.text() == sent {
                            self.editor = text_editor::Content::new();
                        }
                        if same && self.drafts.get(&id) == Some(&sent) {
                            self.drafts.remove(&id);
                        }
                        if same {
                            self.reply_drafts.remove(&id);
                        }
                        self.status.clear();
                    }
                    Request::Page(id) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            r.page(&id, data);
                        }
                    }
                    Request::Members(id) => {
                        if let Some(r) = self.live.list.get_mut(&id) {
                            for e in items(data) {
                                r.posting.ingest(&id, e, true);
                            }
                            #[cfg(target_os = "linux")]
                            if r.voice {
                                for e in items(data) {
                                    r.voice_state(&crate::desktop_call::gomuks_state_event(e));
                                }
                            }
                            for e in items(data).filter(|e| e["type"] == "m.room.member") {
                                if let (Some(user), Some(name)) = (
                                    e["state_key"].as_str(),
                                    e["content"]["displayname"].as_str(),
                                ) {
                                    r.members.entry(user.into()).or_insert_with(|| name.into());
                                }
                            }
                        }
                    }
                    Request::Login => self.password.clear(),
                    Request::Verify(_) if !self.verified => {
                        self.rpc("get_state", json!({}), Request::VerifyState);
                    }
                    Request::VerifyState => {
                        self.frame(json!({"command":"client_state","data":data}));
                        if !self.verified {
                            self.verification_note="gomuks finished the request but still reports this device as unverified. You can retry.".into();
                        }
                    }
                    Request::Verify(_) => {}
                }
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            "init_complete" => self.alerts.spaces.initialized = true,
            _ => {}
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        self.sync_space_rules();
        self.open_created();
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        let refresh_inline = match &msg {
            Msg::Frame(f) => matches!(
                f["command"].as_str(),
                Some(
                    "response"
                        | "sync_complete"
                        | "events_decrypted"
                        | "client_state"
                        | "send_complete"
                )
            ),
            #[cfg(target_os = "linux")]
            Msg::ReturnToCall(_) => true,
            Msg::Open(_)
            | Msg::Section(_)
            | Msg::Back
            | Msg::InlineReady(..)
            | Msg::ShowInline(_) => true,
            _ => false,
        };
        if matches!(
            msg,
            Msg::Setup(_) | Msg::Section(_) | Msg::Open(_) | Msg::Account | Msg::OpenMentions
        ) {
            self.server_menu = false;
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if matches!(msg, Msg::SpaceAlerts) {
            self.server_menu = false;
        }
        match msg {
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::NotificationPanel => self.alerts.panel = !self.alerts.panel,
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::NotifyMode(room, mode) => self.set_notifications(room, mode),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::SetDnd(on) => self.change_dnd(on),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::SpaceAlerts => self.alerts.spaces.panel = !self.alerts.spaces.panel,
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::SpaceMode(space, m) => self.set_space_mode(space, m),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::RetrySpaceRules => self.retry_space_rules(),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::WindowFocus(focused) => {
                self.focused = focused;
                #[cfg(target_os = "linux")]
                if !focused {
                    return self.update(Msg::TalkKey(false));
                }
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Msg::DesktopDelivery(ok) => self.alerts.delivery_failed = !ok,
            #[cfg(target_arch = "wasm32")]
            Msg::AllowBrowserAlerts
                if self.alerts.panel && self.verified && !self.alerts.asking =>
            {
                self.alerts.asking = true;
                return notifications::browser::ask();
            }
            #[cfg(target_arch = "wasm32")]
            Msg::BrowserPermissionDone => self.alerts.asking = false,
            #[cfg(target_arch = "wasm32")]
            Msg::OpenBrowserAlert(user, room)
                if self.verified
                    && self.matrix_id == user
                    && self.live.list.contains_key(&room) =>
            {
                self.account = false;
                self.close_mentions();
                self.section = Section::All;
                self.clear_search();
                self.close_original();
                self.open(room);
                return self.anchor_history();
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::PasteComposer if self.can_stage() && !self.pasting => {
                self.paste_seq += 1;
                self.pasting = true;
                let seq = self.paste_seq;
                let room = self.selected.clone();
                return Task::perform(paste::read(self.media_limit), move |result| {
                    Msg::ClipboardReady(seq, room, result)
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::ClipboardReady(seq, room, result) if seq == self.paste_seq => {
                self.pasting = false;
                if self.can_stage() && self.rooms().list.contains_key(&room) {
                    match result {
                        Ok(paste::Content::Image(f)) => {
                            return self.update(Msg::Files(room, vec![f]));
                        }
                        Ok(paste::Content::Text(s)) if self.selected == room => {
                            self.editor.perform(text_editor::Action::Edit(
                                text_editor::Edit::Paste(s.into()),
                            ))
                        }
                        Err(reason) if self.selected == room => self.status = reason.into(),
                        _ => {}
                    }
                }
            }
            Msg::InlineReady(epoch, key, img) => self.inline.complete(epoch, key, img),
            Msg::ShowInline(key) => self.inline.request(key),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Engine(event) => {
                let jumped = matches!(event, engine::Event::Context { .. });
                self.engine_event(event);
                if jumped && self.original.is_some() {
                    return self.anchor_history();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Sdk(action) => return self.sdk_action(action),
            Msg::Frame(f) => {
                let jumped = f["command"] == "response"
                    && f["request_id"].as_i64().is_some_and(|id| {
                        matches!(self.pending.get(&id), Some(Request::Context(..)))
                    });
                self.frame(f);
                if jumped && self.original.is_some() {
                    return self.anchor_history();
                }
            }
            Msg::Stopped(s) => {
                #[cfg(target_os = "linux")]
                {
                    self.call = None;
                    self.clear_ring();
                }
                self.stopped = true;
                self.connected = false;
                self.pending.clear();
                self.mentions_failed();
                if let Some(d) = &mut self.editing {
                    d.pending = None;
                }
                self.batch = None;
                for q in self.outbox.rooms.values_mut() {
                    for a in q {
                        a.busy = false;
                        a.upload = None;
                    }
                }
                self.status = s;
                if self.closing {
                    return iced::exit();
                }
            }
            Msg::Close => {
                self.closing = true;
                #[cfg(target_os = "linux")]
                self.clear_ring();
                #[cfg(target_os = "linux")]
                if let Some(call) = &self.call {
                    call.cancel.close();
                    return Task::none();
                }
                self.commands.close();
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(sdk) = &self.sdk {
                    sdk.close();
                }
                if self.stopped {
                    return iced::exit();
                }
            }
            #[cfg(target_os = "linux")]
            Msg::DecideRing(id, accept) => self.decide_ring(&id, accept),
            #[cfg(target_os = "linux")]
            Msg::DismissCallNotice => self.inbox.handled = None,
            #[cfg(target_os = "linux")]
            Msg::RingExpired(id) => {
                if self
                    .inbox
                    .ringing
                    .as_ref()
                    .is_some_and(|(i, _)| i.event == id)
                {
                    self.clear_ring();
                }
            }
            #[cfg(target_os = "linux")]
            Msg::DialDm(video)
                if self.call.is_none()
                    && self.inbox.ringing.is_none()
                    && self.verified
                    && self.connected =>
            {
                if let Some(peer) = self
                    .live
                    .list
                    .get(&self.selected)
                    .filter(|r| r.encrypted && !r.voice)
                    .and_then(|r| r.dm_peer.clone())
                {
                    self.start_voice(
                        self.selected.clone(),
                        desktop_call::Target::Dm {
                            video,
                            peer,
                            incoming: None,
                        },
                    );
                }
            }
            #[cfg(target_os = "linux")]
            Msg::JoinVoice
                if self.call.is_none()
                    && self.verified
                    && self.connected
                    && self.inbox.ringing.is_none()
                    && self
                        .live
                        .list
                        .get(&self.selected)
                        .is_some_and(|r| r.voice && r.encrypted) =>
            {
                self.start_voice(self.selected.clone(), desktop_call::Target::Channel);
            }
            #[cfg(target_os = "linux")]
            Msg::EndCall => {
                if let Some(call) = &mut self.call {
                    call.cancel.close();
                    call.label = "Ending call…".into();
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ToggleMic => {
                if let Some(call) = &self.call
                    && !call.cancel.is_closed()
                {
                    call.audio.send_if_modified(|s| {
                        if s.deafened {
                            return false;
                        }
                        s.muted = !s.muted;
                        s.hold(false);
                        true
                    });
                }
            }
            #[cfg(target_os = "linux")]
            Msg::SetShareSound(id, enabled) => {
                if let Some(c) = &self.call
                    && c.id == id
                    && !c.cancel.is_closed()
                {
                    c.audio.send_modify(|m| m.share_silent = !enabled);
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ToggleDeafen => {
                if let Some(call) = &self.call
                    && !call.cancel.is_closed()
                {
                    call.audio.send_modify(|s| {
                        s.deafened = !s.deafened;
                        s.hold(false);
                    });
                }
            }
            #[cfg(target_os = "linux")]
            Msg::RoomPtt(enabled) => {
                self.room_ptt = enabled;
                if let Some(call) = &self.call
                    && call.group
                    && !call.cancel.is_closed()
                {
                    call.audio
                        .send_modify(|s| s.talk = desktop_call::TalkGate::from_ptt(enabled));
                }
            }
            #[cfg(target_os = "linux")]
            Msg::TalkKey(down) => {
                if let Some(call) = &self.call
                    && call.group
                    && !call.cancel.is_closed()
                {
                    call.audio.send_if_modified(|s| s.hold(down));
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ReturnToCall(id) => {
                if let Some(c) = &self.call
                    && c.id == id
                    && !c.cancel.is_closed()
                    && self.verified
                    && !self.setup_busy()
                    && self.live.list.contains_key(&c.room)
                {
                    let room = c.room.clone();
                    self.cancel_setup_if_idle();
                    self.close_mentions();
                    self.account = false;
                    if !self.visible().any(|(r, _)| r.as_str() == room) {
                        self.section = Section::All;
                    }
                    self.open(room);
                    if let Some(call) = &mut self.call {
                        call.stage_open = true;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::CallLayout(id, chat) => {
                if let Some(c) = &mut self.call
                    && c.id == id
                    && c.stage_open
                {
                    if chat {
                        c.stage_open = false;
                        c.theater = false;
                    } else {
                        c.theater = !c.theater;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::HideCallPreview(id, hidden) => {
                if let Some(c) = &mut self.call
                    && c.id == id
                {
                    c.preview_hidden = hidden;
                }
            }
            #[cfg(target_os = "linux")]
            Msg::FocusVideo(id, sid) => {
                if let Some(call) = &self.call
                    && call.id == id
                {
                    call.previews.focus(sid);
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ShareApp(id, window, choice) => {
                let slot = desktop_call::VideoInput::Window as usize;
                if let Some(c) = &mut self.call
                    && c.id == id
                    && !c.cancel.is_closed()
                    && c.video_status[slot].live
                    && c.audio.borrow().video[slot] == Some(window)
                    && choice
                        .as_ref()
                        .is_none_or(|s| c.picker && c.devices.streams.contains(s))
                {
                    c.app_note = if choice.is_some() {
                        "Starting application audio…".into()
                    } else {
                        String::new()
                    };
                    c.audio
                        .send_modify(|m| m.send_app = choice.map(|s| (uuid::Uuid::new_v4(), s)));
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ToggleVideo(input) => {
                if let Some(call) = &mut self.call
                    && !call.cancel.is_closed()
                {
                    let slot = input as usize;
                    let stop = call.audio.borrow().video[slot].is_some();
                    call.audio.send_modify(|m| {
                        if input == desktop_call::VideoInput::Window {
                            m.send_app = None;
                        }
                        m.video[slot] = if stop {
                            None
                        } else {
                            Some(uuid::Uuid::new_v4())
                        }
                    });
                    call.video_status[slot].live = false;
                    if input == desktop_call::VideoInput::Window {
                        call.app_note.clear();
                    }
                    call.video_status[slot].label = if stop {
                        String::new()
                    } else {
                        input.starting().into()
                    };
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ToggleMixer => {
                if let Some(call) = &mut self.call
                    && call.group
                    && !call.cancel.is_closed()
                {
                    call.mixer = !call.mixer;
                    if call.mixer {
                        call.picker = false;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::PeerVolume(id, user, percent) => {
                if let Some(call) = &self.call
                    && call.group
                    && call.id == id
                    && !call.cancel.is_closed()
                    && user != self.matrix_id
                {
                    call.audio.send_if_modified(|s| s.set_volume(user, percent));
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ListCameras => {
                if let Some(c) = &mut self.call
                    && c.picker
                    && !c.camera_listing
                    && !c.cancel.is_closed()
                {
                    c.camera_listing = true;
                    c.camera_note = None;
                    let cancel = c.cancel.clone();
                    let id = c.id;
                    return Task::perform(
                        async move {
                            tokio::select! {biased;_=cancel.closed()=>Err("Call ended."),r=desktop_call::list_cameras()=>r}
                        },
                        move |r| Msg::CamerasListed(id, r),
                    );
                }
            }
            #[cfg(target_os = "linux")]
            Msg::CamerasListed(id, list) => {
                if let Some(c) = &mut self.call
                    && c.id == id
                {
                    c.camera_listing = false;
                    match list {
                        Ok(v) => {
                            c.camera_note =
                                (v.len() == 1).then_some("No selectable cameras reported.");
                            c.cameras = v;
                        }
                        Err(why) => {
                            c.cameras.clear();
                            c.camera_note = Some(why);
                        }
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::SelectCamera(id, choice) => {
                if let Some(c) = &self.call
                    && c.id == id
                    && c.picker
                    && !c.cancel.is_closed()
                    && c.cameras.contains(&choice)
                {
                    c.audio.send_modify(|s| s.camera = choice);
                }
            }
            #[cfg(target_os = "linux")]
            Msg::AudioDevices | Msg::RefreshDevices => {
                if let Some(call) = &mut self.call
                    && !call.cancel.is_closed()
                {
                    if matches!(msg, Msg::AudioDevices) {
                        call.picker = !call.picker;
                        if call.picker {
                            call.mixer = false;
                        }
                    }
                    if call.picker && !call.loading {
                        call.loading = true;
                        let id = call.id;
                        return Task::perform(call_devices::Catalog::read(), move |r| {
                            Msg::DevicesListed(id, r)
                        });
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::DevicesListed(id, result) => {
                if let Some(call) = &mut self.call
                    && call.id == id
                {
                    call.loading = false;
                    match result {
                        Ok(devices) => {
                            call.devices = devices;
                            call.list_error = None;
                        }
                        Err(e) => call.list_error = Some(e),
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Msg::SelectDevice(port, device) => {
                if let Some(call) = &mut self.call
                    && !call.cancel.is_closed()
                    && call.devices.list(port).contains(&device)
                {
                    call.audio.send_modify(|s| {
                        let route = match port {
                            call_devices::Port::Mic => &mut s.mic,
                            call_devices::Port::Speaker => &mut s.speaker,
                        };
                        route.name = device.name;
                        route.revision += 1;
                    });
                    match port {
                        call_devices::Port::Mic => call.mic_error = false,
                        call_devices::Port::Speaker => call.speaker_error = false,
                    }
                }
            }
            Msg::CheckCalls
                if self.logged_in
                    && self.connected
                    && !self
                        .pending
                        .values()
                        .any(|p| matches!(p, Request::RtcDiscovery)) =>
            {
                self.rtc_status = "Checking MatrixRTC transports…".into();
                #[cfg(not(target_arch = "wasm32"))]
                if self.sdk.is_some() {
                    self.command(
                        |id| engine::Command::RtcTransports { id },
                        Request::RtcDiscovery,
                    );
                    return Task::none();
                }
                self.rpc("get_rtc_transports", json!({}), Request::RtcDiscovery);
            }
            Msg::HoverFiles(v) => self.hover_files = v,
            #[cfg(target_os = "linux")]
            Msg::WaylandDrop(offer) => {
                self.hover_files = false;
                if self.can_stage()
                    && let Ok(drop) = offer.try_recv()
                {
                    let epoch = self.drop_epoch;
                    let room = self.selected.clone();
                    return Task::perform(wayland_drop::resolve(drop), move |result| {
                        Msg::WaylandDropResolved(epoch, room, result)
                    });
                }
            }
            #[cfg(target_os = "linux")]
            Msg::WaylandDropResolved(epoch, room, result)
                if epoch == self.drop_epoch && self.rooms().list.contains_key(&room) =>
            {
                match result {
                    Ok(files) => return self.stage_files(room, files),
                    Err(reason) => self.status = reason.into(),
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::DropPaths(paths) => {
                self.hover_files = false;
                if self.can_stage() {
                    let room = self.selected.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                paths
                                    .into_iter()
                                    .take(media::MAX_FILES + 1)
                                    .filter_map(media::LocalFile::from_path)
                                    .collect()
                            })
                            .await
                            .unwrap_or_default()
                        },
                        move |files| Msg::Files(room, files),
                    );
                }
            }
            Msg::Files(mut room, files) => {
                self.hover_files = false;
                if room.is_empty() {
                    room = self.selected.clone();
                }
                if self.can_stage() && self.rooms().list.contains_key(&room) {
                    return self.stage_files(room, files);
                }
            }
            Msg::Thumbnail(room, id, thumb) => {
                if let Some(a) = self.outbox.get_mut(&room, id) {
                    a.thumbnail = thumb;
                }
            }
            Msg::PickFiles if self.can_stage() => {
                let room = self.selected.clone();
                #[cfg(not(target_arch = "wasm32"))]
                return Task::perform(
                    async {
                        let paths = rfd::AsyncFileDialog::new().pick_files().await?;
                        Some(
                            tokio::task::spawn_blocking(move || {
                                paths
                                    .into_iter()
                                    .take(media::MAX_FILES + 1)
                                    .filter_map(|p| {
                                        media::LocalFile::from_path(p.path().to_owned())
                                    })
                                    .collect()
                            })
                            .await
                            .unwrap_or_default(),
                        )
                    },
                    move |files| match files {
                        Some(f) => Msg::Files(room, f),
                        None => Msg::HoverFiles(false),
                    },
                );
                #[cfg(target_arch = "wasm32")]
                web_files::pick(&room);
            }
            Msg::DismissSkipped(room) if room == self.selected => {
                self.outbox.skipped.remove(&room);
            }
            Msg::RemoveFile(id) => {
                if self
                    .outbox
                    .get_mut(&self.selected, id)
                    .is_some_and(|a| !a.busy)
                {
                    self.outbox.remove(&self.selected, id);
                }
            }
            Msg::CancelUpload(room, id, request) => self.cancel_upload(&room, id, request),
            Msg::SaveFile(media) if self.connected && self.logged_in => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let name = media.name.clone();
                    return Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_title("Save attachment")
                                .set_file_name(name)
                                .save_file()
                                .await
                                .map(|p| p.path().to_owned())
                        },
                        move |p| Msg::SaveTo(media, p),
                    );
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.seq += 1;
                    if self
                        .commands
                        .try_send(backend::Command::Download {
                            id: self.seq,
                            media,
                        })
                        .is_ok()
                    {
                        self.pending.insert(self.seq, Request::Download);
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::SaveTo(media, Some(destination)) if self.connected && self.logged_in => {
                self.seq += 1;
                if self
                    .commands
                    .try_send(backend::Command::Download {
                        id: self.seq,
                        media,
                        destination,
                    })
                    .is_ok()
                {
                    self.pending.insert(self.seq, Request::Download);
                }
            }
            Msg::Setup(input) => return self.setup_input(input),
            Msg::Section(s) => {
                self.cancel_setup_if_idle();
                self.close_mentions();
                self.section = s;
                self.reconcile();
            }
            Msg::Open(id) => {
                #[cfg(target_os = "linux")]
                self.minimize_call();
                self.cancel_setup_if_idle();
                self.close_mentions();
                self.open(id);
            }
            Msg::OpenMentions => {
                #[cfg(target_os = "linux")]
                self.minimize_call();
                self.cancel_setup_if_idle();
                self.open_mentions();
            }
            Msg::CheckReading => return self.check_reading(),
            Msg::ReadingScroll(room, away) if room == self.selected && self.original.is_none() => {
                self.scrolled_away = away
            }
            Msg::ShareReads(value) => self.set_read_sharing(value),
            Msg::ReadVisible(room, event, row, true)
                if self.auto_target() == Some((event.as_str(), row.as_str()))
                    && room == self.selected
                    && self.reading_focused() =>
            {
                self.mark_room(room, Some(event));
            }
            Msg::MarkRoom(room, event) => self.mark_room(room, event),
            Msg::MoreMentions => {
                if let Some(i) = &mut self.mentions {
                    i.shown = i.shown.saturating_add(50);
                }
            }
            Msg::MentionHit(room, id) => {
                if self.open_mention(room, id) && self.original.is_some() {
                    return self.anchor_history();
                }
            }
            Msg::ServerMenu(open) => self.server_menu = open,
            Msg::Escape => {
                if self.server_menu {
                    self.server_menu = false;
                } else if self.setup.is_some() && !self.setup_busy() {
                    return self.setup_input(community::Input::Close);
                } else if self.account && self.logged_in && self.verified {
                    self.account = false;
                }
            }
            Msg::Collapse(key) => {
                if !self.collapsed.remove(&key) {
                    self.collapsed.insert(key);
                }
            }
            Msg::TogglePeople => {
                self.people = !self.people;
                self.reset_directory();
                if self.people {
                    self.read_directory(None);
                }
            }
            Msg::MemberFilter(s) => {
                self.directory.filter = s;
                self.directory.choose(None);
            }
            Msg::SelectMember(id) => self.directory.choose(Some(id)),
            Msg::MemberReason(reason) => self.directory.reason = reason,
            Msg::RefreshMembers => {
                self.directory.note.clear();
                self.read_directory(None);
            }
            Msg::Moderate(c) => self.moderate(c),
            Msg::Account => {
                #[cfg(target_os = "linux")]
                self.minimize_call();
                self.close_mentions();
                self.account = true;
            }
            Msg::Back => self.account = false,
            Msg::Server(s) => self.server = s,
            Msg::Username(s) => self.username = s,
            Msg::Password(s) => self.password = s,
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Recovery(s) => self.recovery = s,
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Login if self.sdk.is_some() => self.engine_login(),
            Msg::Login
                if !self.logged_in
                    && self.connected
                    && !self.pending.values().any(|p| matches!(p, Request::Login)) =>
            {
                if let Some(url) = account::homeserver(&self.server)
                    && !self.username.trim().is_empty()
                    && !self.password.is_empty()
                {
                    self.server = url;
                    self.status = "Signing in…".into();
                    self.rpc("login",json!({"homeserver_url":self.server,"username":self.username.trim(),"password":self.password}),Request::Login);
                } else {
                    self.status =
                        "Enter your homeserver (for example, matrix.org), username, and password."
                            .into();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Verify if self.sdk.is_some() => self.engine_verify(),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Verify => self.submit_verification(),
            #[cfg(target_arch = "wasm32")]
            Msg::SubmitWebRecovery(s) => {
                if self.logged_in && !self.verified {
                    self.recovery = s;
                    self.submit_verification();
                    self.recovery.clear();
                }
            }
            Msg::Draft(a) => self.editor.perform(a),
            Msg::Permissions => {
                if self.permission_panel {
                    self.leave_permission_panel();
                } else {
                    self.permission_panel = true;
                }
            }
            Msg::SetPosting(c) => self.change_posting(c),
            Msg::OpenSearch => return self.begin_search(),
            Msg::SearchReady(room, b) => return self.search_ready(room, b),
            Msg::SearchInput(input) => {
                if let Some(s) = &mut self.search {
                    s.input = input;
                }
            }
            Msg::RunSearch(more) => self.submit_search(more),
            Msg::SearchCoverage => {
                self.start_coverage();
                return operation::focus("room-search-input");
            }
            Msg::SearchHit(id) => {
                if self
                    .search
                    .as_ref()
                    .is_some_and(|s| s.room == self.selected && s.hits.iter().any(|h| h.id == id))
                {
                    self.jump(id);
                    if self.original.is_some() {
                        return self.anchor_history();
                    }
                }
            }
            Msg::SearchResults => self.close_original(),
            Msg::ReturnToReading => return self.return_to_reading(),
            Msg::ReactionPicker(id) => {
                if self.visible_post(&id).is_some_and(|p| p.reactable) {
                    self.reaction_picker = (!self
                        .reaction_picker
                        .as_ref()
                        .is_some_and(|p| p.target == id))
                    .then(|| reactions::Picker {
                        target: id,
                        custom: String::new(),
                    });
                }
            }
            Msg::ReactionInput(s) => {
                if let Some(p) = &mut self.reaction_picker {
                    p.custom = s;
                }
            }
            Msg::ToggleReaction(op) => self.toggle_reaction(op),
            Msg::Report(room, id) => {
                if self.begin_report(&room, &id) {
                    return operation::focus("report-reason");
                }
            }
            Msg::ReportReason(reason) if !self.report_pending() => {
                if let Some(d) = &mut self.reporting {
                    d.reason = reason;
                }
            }
            Msg::CancelReport if !self.report_pending() => self.reporting = None,
            Msg::SubmitReport => self.submit_report(),
            Msg::Edit(id) => {
                if self.start_edit(id) {
                    return operation::focus("message-edit");
                }
            }
            Msg::ChangeEdit(a) => {
                if let Some(d) = &mut self.editing
                    && d.pending.is_none()
                {
                    d.input.perform(a);
                }
            }
            Msg::SaveEdit => self.save_edit(),
            Msg::CancelEdit => {
                if self.editing.as_ref().is_some_and(|d| d.pending.is_none()) {
                    self.editing = None;
                }
            }
            Msg::Latest => {
                self.close_original();
                self.timeline_end = None;
                return self.anchor_history();
            }
            Msg::Preceding => {
                if let Some(c) = &mut self.original {
                    c.preceding = !c.preceding;
                    return self.anchor_history();
                }
            }
            Msg::Jump(id) => {
                self.jump(id);
                if self.original.is_some() {
                    return self.anchor_history();
                }
            }
            Msg::CancelReply => {
                self.reply_drafts.remove(&self.selected);
            }
            Msg::Reply(id) => {
                if let Some(q) = self
                    .live
                    .list
                    .get(&self.selected)
                    .and_then(|r| r.quote(&id))
                    .or_else(|| self.original.as_ref()?.slice.as_ref()?.quote(&id))
                {
                    self.reply_drafts.insert(self.selected.clone(), q);
                    if self.search.is_some() {
                        return self.return_to_reading().chain(operation::focus("composer"));
                    }
                    self.close_original();
                    return operation::focus("composer");
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Send if self.sdk.is_some() && self.logged_in => self.engine_compose(),
            Msg::Send => {
                if self.logged_in
                    && self.live.list.get(&self.selected).is_some_and(|r| {
                        r.posting.can_post(&self.matrix_id, r.encrypted) == Some(false)
                    })
                {
                    self.status = "This room is read-only for your account.".into();
                    return Task::none();
                }
                // Do not send just the caption if Enter races PNG encoding.
                #[cfg(not(target_arch = "wasm32"))]
                if self.pasting {
                    return Task::none();
                }
                let body = self.editor.text();
                let id = self.selected.clone();
                if self.outbox.rooms.get(&id).is_some_and(|q| !q.is_empty()) {
                    self.send_files(id, body);
                    return Task::none();
                }
                if id.is_empty() || body.trim().is_empty() || self.outbox.skipped.contains_key(&id)
                {
                    return Task::none();
                }
                if self.logged_in {
                    if self.verified
                        && !self
                            .pending
                            .values()
                            .any(|p| matches!(p,Request::Send(r,..) if r==&id))
                    {
                        let reply = self.reply_drafts.get(&id).cloned();
                        let mut params = json!({"room_id":id,"text":body});
                        if let Some(q) = &reply {
                            // Treat a reply as literal message text. In particular a
                            // slash-prefixed reply must never disable room encryption.
                            params = json!({"room_id":id,"text":"","base_content":{"msgtype":"m.text","body":body}});
                            q.relate(&mut params);
                        }
                        self.rpc("send_message", params, Request::Send(id, body, reply));
                    }
                } else if let Some(r) = self.demo.list.get_mut(&id) {
                    r.insert(&json!({"rowid":r.history.keys().next_back().copied().unwrap_or(0)+1,"sender":"You","type":"m.room.message",
                        "timestamp":chrono::Utc::now().timestamp_millis(),"content":{"body":body}}));
                    self.editor = text_editor::Content::new();
                }
            }
            Msg::More => self.page(),
            Msg::TimelineRestored => self.timeline_restoring = false,
            Msg::TimelineEdge(older) => {
                if self.timeline_restoring || self.original.is_some() || self.search.is_some() {
                    return Task::none();
                }
                let Some(room) = self.rooms().list.get(&self.selected) else {
                    return Task::none();
                };
                let range = self.timeline_range(room);
                const STEP: usize = 60;
                let end = if older && range.start > 0 {
                    range.end - STEP.min(range.start)
                } else if !older && range.end < room.posts().len() {
                    range.end + STEP
                } else {
                    return Task::none();
                };
                let to = room.key_at(end);
                let from = self.timeline_end;
                let room = self.selected.clone();
                return search::bookmark::capture(self.timeline_id()).map(move |bookmark| {
                    Msg::MoveTimeline {
                        room: room.clone(),
                        from,
                        to,
                        bookmark,
                    }
                });
            }
            Msg::MoveTimeline {
                room,
                from,
                to,
                bookmark,
            } if room == self.selected
                && from == self.timeline_end
                && self.original.is_none()
                && self.search.is_none() =>
            {
                self.timeline_end = to;
                self.scrolled_away = true;
                self.timeline_restoring = true;
                return search::bookmark::restore_row(self.timeline_id(), bookmark)
                    .chain(Task::done(Msg::TimelineRestored));
            }
            Msg::MoveTimeline { .. } => {}
            Msg::Copy => {
                if let Some(r) = self.rooms().list.get(&self.selected) {
                    let text = r
                        .posts()
                        .map(|p| format!("{}: {}", r.display(&p.sender), p.text()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    #[cfg(not(target_arch = "wasm32"))]
                    return iced::clipboard::write(text);
                    #[cfg(target_arch = "wasm32")]
                    return Task::perform(
                        wasm_bindgen_futures::JsFuture::from(
                            web_sys::window()
                                .unwrap()
                                .navigator()
                                .clipboard()
                                .write_text(&text),
                        ),
                        |r| Msg::ClipboardResult(r.is_ok()),
                    );
                }
            }
            #[cfg(target_arch = "wasm32")]
            Msg::WebPaste(field) => {
                return Task::perform(
                    wasm_bindgen_futures::JsFuture::from(
                        web_sys::window()
                            .unwrap()
                            .navigator()
                            .clipboard()
                            .read_text(),
                    ),
                    move |v| {
                        v.ok()
                            .and_then(|v| v.as_string())
                            .map_or(Msg::ClipboardResult(false), |s| Msg::Pasted(field, s))
                    },
                );
            }
            #[cfg(target_arch = "wasm32")]
            Msg::Pasted(field, s) => match field {
                Field::Composer(room) if self.selected == room && !self.account => self
                    .editor
                    .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        s.into(),
                    ))),
                Field::Password if self.account && !self.logged_in => self.password = s,
                Field::Username if self.account && !self.logged_in => {
                    self.username = s.trim().into()
                }
                Field::Server if self.account && !self.logged_in => self.server = s.trim().into(),
                _ => {}
            },
            #[cfg(target_arch = "wasm32")]
            Msg::ClipboardResult(false) => {
                self.status = "Browser clipboard permission was denied.".into()
            }
            Msg::Tab(reverse) => {
                return if reverse {
                    operation::focus_previous()
                } else {
                    operation::focus_next()
                };
            }
            _ => {}
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            browser_verification::sync(self);
            let title = self.title();
            if doc.title() != title {
                doc.set_title(&title);
            }
        }
        let task = if refresh_inline {
            self.schedule_inline()
        } else {
            Task::none()
        };
        #[cfg(target_os = "linux")]
        let task = Task::batch([task, self.start_ringtone()]);
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        let task = Task::batch([task, self.send_alerts()]);
        task
    }
    /// The MSC4143 transports a homeserver advertises, for "Check call server
    /// support".
    fn rtc_discovered(&mut self, transports: &[Value]) {
        let n = transports
            .iter()
            .filter(|v| {
                v["type"] == "livekit"
                    && v["livekit_service_url"].as_str().is_some_and(|s| {
                        url::Url::parse(s)
                            .is_ok_and(|u| u.scheme() == "https" && u.host().is_some())
                    })
            })
            .count();
        self.rtc_status = if n > 0 {
            format!("Found {n} LiveKit call transport(s).")
        } else {
            "No supported LiveKit transport advertised by this endpoint.".into()
        };
    }
    fn title(&self) -> String {
        let page = if self.account || self.logged_in && !self.verified {
            "Account"
        } else if self.mentions.is_some() {
            "Mentions"
        } else {
            self.rooms()
                .list
                .get(&self.selected)
                .map(|r| r.name.as_str())
                .unwrap_or("Rooms")
        };
        format!(
            "Untethered  ·  {page}{}",
            if self.logged_in { "" } else { "  ·  Preview" }
        )
    }
}
fn main() -> iced::Result {
    #[cfg(not(target_arch = "wasm32"))]
    fonts::configure();
    iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .font(include_bytes!("../assets/LiberationSans-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/LiberationSans-Bold.ttf").as_slice())
        .default_font(iced::Font::with_name("Liberation Sans"))
        .theme(|_: &App| Theme::Dark)
        .window(iced::window::Settings {
            size: iced::Size::new(1180., 760.),
            min_size: Some(iced::Size::new(760., 500.)),
            exit_on_close_request: false,
            ..Default::default()
        })
        .subscription(|app| {
            iced::Subscription::batch([
                if app.auto_target().is_some(){
                    iced::time::every(std::time::Duration::from_millis(750)).map(|_|Msg::CheckReading)
                }else{iced::Subscription::none()},
                #[cfg(not(target_arch = "wasm32"))]
                iced::event::listen_with(|event, _, _| match event {
                    // Right Ctrl is a non-repeating modifier. On X11 the first
                    // repeat of an ordinary key after refocus can look like a
                    // fresh press. Always handle release/blur, even in an editor.
                    #[cfg(target_os = "linux")]
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        physical_key:
                            iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ControlRight),
                        repeat: false,
                        ..
                    }) => Some(Msg::TalkKey(true)),
                    #[cfg(target_os = "linux")]
                    iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                        physical_key:
                            iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ControlRight),
                        ..
                    }) => {
                        Some(Msg::TalkKey(false))
                    }
                    // Escape also closes menus and dialogs while a text field has
                    // focus, as on Discord; the field itself captures the key.
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    }) => Some(Msg::Escape),
                    iced::Event::Window(iced::window::Event::Unfocused)=>Some(Msg::WindowFocus(false)),
                    iced::Event::Window(iced::window::Event::Focused)=>Some(Msg::WindowFocus(true)),
                    iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                        Some(Msg::DropPaths(vec![path]))
                    }
                    iced::Event::Window(iced::window::Event::FileHovered(_)) => {
                        Some(Msg::HoverFiles(true))
                    }
                    iced::Event::Window(iced::window::Event::FilesHoveredLeft) => {
                        Some(Msg::HoverFiles(false))
                    }
                    _ => None,
                }),
                iced::window::close_requests().map(|_| Msg::Close),
                iced::keyboard::listen().filter_map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                        modifiers,
                        ..
                    } => Some(Msg::Tab(modifiers.shift())),
                    #[cfg(target_arch = "wasm32")]
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    } => Some(Msg::Escape),
                    _ => None,
                }),
            ])
        })
        .run()
}
