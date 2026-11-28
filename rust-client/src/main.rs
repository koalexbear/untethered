#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
// matrix-sdk futures nest deeply enough to exceed the default limit when
// rustc proves they are Send.
#![recursion_limit = "256"]
mod account;
#[cfg(target_os = "linux")]
mod app_id;
mod autocomplete;
mod avatars;
mod backend;
#[cfg(target_arch = "wasm32")]
mod browser_verification;
#[cfg(target_os = "linux")]
mod call_devices;
#[cfg(target_os = "linux")]
mod call_settings;
mod channel_order;
mod commands;
mod community;
mod composer;
mod context_menu;
#[cfg(not(target_arch = "wasm32"))]
mod copy_image;
#[cfg(target_os = "linux")]
mod desktop_call;
mod dms;
mod drafts;
mod edits;
mod emoji;
mod emotes;
mod engine;
mod explore;
#[cfg(not(target_arch = "wasm32"))]
mod fonts;
mod forward;
mod highlight;
#[cfg(target_os = "linux")]
mod hotkeys;
#[cfg(target_os = "linux")]
mod incoming;
mod inline;
#[cfg(target_os = "linux")]
mod inline_player;
#[cfg(not(target_arch = "wasm32"))]
mod instance;
mod invites;
mod joining;
mod jumbo;
mod link_previews;
mod media;
mod mentions;
mod moderation;
mod names;
#[cfg(any(target_os = "linux", target_arch = "wasm32"))]
mod notifications;
mod paging;
#[cfg(not(target_arch = "wasm32"))]
mod paste;
mod permissions;
mod pills;
mod places;
#[cfg(target_os = "linux")]
mod player;
mod polls;
mod presence;
mod preview;
#[cfg(target_os = "linux")]
mod qa_layout;
mod rail_order;
mod reactions;
mod reading;
mod replies;
mod reports;
mod rich;
mod room_state;
mod rooms;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
mod search;
mod server_settings;
mod shortcuts;
#[cfg(target_os = "linux")]
mod sounds;
mod switcher;
mod transfers;
#[cfg(not(target_arch = "wasm32"))]
mod tray;
mod typing;
mod upgrades;
mod view;
mod viewer;
#[cfg(target_os = "linux")]
mod wayland_drop;
#[cfg(target_arch = "wasm32")]
mod web_files;

use async_channel::Sender;
use iced::{
    Task, Theme,
    widget::{operation, text_editor},
};
use rooms::Rooms;
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
    Discussion(view::discussions::Action),
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
    PickThreadImage,
    RemoveFile(u64),
    DismissSkipped(String),
    CancelUpload(String, u64, i64),
    SaveFile(media::Media),
    InlineReady(u64, media::PreviewRef, Option<preview::Picture>),
    /// A failed preview's wait is over: fetch it again.
    RetryInline(u64, media::PreviewRef),
    AvatarReady(String, Option<preview::Picture>),
    ShowInline(media::PreviewRef),
    #[cfg(not(target_arch = "wasm32"))]
    SaveTo(media::Media, Option<std::path::PathBuf>),
    CheckCalls,
    #[cfg(target_os = "linux")]
    DialDm(bool),
    #[cfg(target_os = "linux")]
    JoinVoice,
    /// A voice channel clicked in the room list: open it and join its call.
    OpenVoice(String),
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
    Hotkey(hotkeys::Event),
    #[cfg(target_os = "linux")]
    TalkRelease(tokio::time::Instant),
    #[cfg(target_os = "linux")]
    ToggleVideo(desktop_call::VideoInput),
    #[cfg(target_os = "linux")]
    FocusVideo(uuid::Uuid, Option<String>),
    /// The call stage's thumbnail strip or grid scrolled (desktop_call.rs).
    #[cfg(target_os = "linux")]
    StageScroll(
        uuid::Uuid,
        desktop_call::StageScroll,
        desktop_call::Scrolled,
    ),
    #[cfg(target_os = "linux")]
    ReturnToCall(uuid::Uuid),
    #[cfg(target_os = "linux")]
    CallLayout(uuid::Uuid, bool),
    #[cfg(target_os = "linux")]
    HideCallPreview(uuid::Uuid, bool),
    /// A call video frame is on the GPU, or couldn't be put there.
    #[cfg(target_os = "linux")]
    VideoUploaded(
        String,
        Result<iced::advanced::image::Allocation, iced::advanced::image::Error>,
    ),
    /// Open or close a device menu or the voice settings popup.
    #[cfg(target_os = "linux")]
    VoiceMenu(Option<desktop_call::VoiceMenu>),
    /// Write the QA control map (`None`: walk the window for it).
    #[cfg(target_os = "linux")]
    QaLayout(Option<Vec<qa_layout::Found>>),
    #[cfg(target_os = "linux")]
    ListCameras,
    #[cfg(target_os = "linux")]
    CamerasListed(Result<Vec<desktop_call::CameraChoice>, &'static str>),
    /// The camera chosen on the Voice & Video page (call_settings.rs).
    #[cfg(target_os = "linux")]
    SelectCamera(desktop_call::CameraChoice),
    #[cfg(target_os = "linux")]
    PeerVolume(uuid::Uuid, String, u8),
    #[cfg(target_os = "linux")]
    RefreshDevices,
    #[cfg(target_os = "linux")]
    DevicesListed(Result<call_devices::Catalog, &'static str>),
    #[cfg(target_os = "linux")]
    SelectDevice(call_devices::Port, call_devices::Choice),
    #[cfg(target_os = "linux")]
    DecideRing(String, bool),
    #[cfg(target_os = "linux")]
    DismissCallNotice,
    #[cfg(target_os = "linux")]
    RingExpired(String),
    Engine(engine::Event),
    Sdk(engine::account_ui::Action),
    Stopped(String),
    Section(Section),
    Open(String),
    /// Join the room that replaced this one in an upgrade (upgrades.rs).
    FollowUpgrade(String),
    Setup(community::Input),
    CheckReading,
    /// Scrolled the timeline: away from the bottom, and far enough for Jump to present.
    /// Where the room's timeline is scrolled to now.
    ReadingScroll(String, paging::Scrolled),
    /// Open the room's timeline again after a pause (paging.rs).
    ReopenTimeline(String),
    /// Turn a room's link previews on or off (link_previews.rs).
    LinkPreviews(String, bool),
    /// Show an invite's card, and accept or decline it (invites.rs).
    OpenInvite(String),
    AcceptInvite(String),
    DeclineInvite(String),
    /// A linked room's preview: join it or close it; and Join a Server's
    /// field (joining.rs).
    JoinPreviewed,
    ClosePreview,
    JoinAddressInput(String),
    JoinAddress,
    /// Explore, the public room directory (explore.rs).
    Explore(explore::Input),
    /// New message (dms.rs).
    Dm(dms::Input),
    /// Forwarding a message (forward.rs).
    Forward(forward::Input),
    /// Custom emoji and stickers (emotes.rs), and a picture of one fetched.
    Emote(emotes::Input),
    EmoteReady(String, Option<preview::Picture>),
    /// Voting, ending and creating polls (polls.rs).
    Poll(polls::Input),
    /// Dragging channels and categories in a space's sidebar (channel_order.rs).
    Order(channel_order::Input),
    /// Arranging servers and folders in the rail (rail_order.rs).
    Rail(rail_order::Input),
    /// Edit Server Profile (names.rs).
    Names(names::Input),
    /// Server Settings for a space (server_settings.rs).
    ServerSettings(server_settings::Input),
    /// The composer's + menu: Upload a File, Create Poll.
    ComposerMenu(bool),
    /// The quick switcher (switcher.rs).
    Switcher(switcher::Input),
    /// Note where the open room is scrolled to, and what was measured;
    /// save what's remembered (places.rs).
    NotePlace,
    PlaceNoted(String, Option<(String, f32)>),
    SavePlaces,
    /// Scrolled near the top of what's loaded: fetch older messages.
    /// Page Up (true) or Page Down: scroll the timeline by most of a screen.
    PageScroll(bool),
    ShareReads(bool),
    ReadVisible(String, String, String, bool),
    MarkRoom(String, Option<String>),
    OpenMentions,
    MoreMentions,
    MentionHit(String, String),
    Draft(text_editor::Action),
    #[cfg(not(target_arch = "wasm32"))]
    SaveDrafts,
    /// Composer autocomplete and the emoji button (autocomplete.rs).
    Autocomplete(autocomplete::Input),
    /// Typing notices and their switch (typing.rs).
    Typing(typing::Input),
    /// Our status, idling and the custom status dialog (presence.rs).
    Presence(presence::Input),
    Send,
    Reply(String),
    Report(String, String),
    ReportReason(String),
    SubmitReport,
    CancelReport,
    ReactionPicker(String),
    /// The reaction picker, from the smile beside a message's reactions.
    ReactionPickerChips(String),
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
    RunSearch,
    /// Search this channel, this server or all rooms (search.rs).
    SearchScope(search::Scope),
    /// Load older encrypted history onto this device, then search again.
    IndexSearch,
    /// A hit: its room and event.
    SearchHit(String, String),
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
    /// A link in a message.
    OpenLink(String),
    /// Show a message's spoiler: its event and the spoiler's number.
    RevealSpoiler(String, u16),
    /// A code block's Copy button.
    CopyCode(String),
    TogglePeople,
    /// Open or close the server dropdown in the channel sidebar.
    ServerMenu(bool),
    /// Close the topmost menu, dialog or settings page.
    Escape,
    /// An Escape no widget captured: the same, then cancel a reply or mark
    /// the channel read (composer.rs).
    FreeEscape,
    /// Alt+Shift+↓, Shift+Escape and Ctrl+/ (shortcuts.rs).
    Shortcut(shortcuts::Input),
    /// The sounds' switches (sounds.rs).
    #[cfg(target_os = "linux")]
    Sound(sounds::Event),
    /// Voice processing and input sensitivity (call_settings.rs).
    #[cfg(target_os = "linux")]
    Voice(call_settings::Event),
    /// Redraw the input sensitivity meter.
    #[cfg(target_os = "linux")]
    VoiceMeter,
    /// Right-click menus and what their entries do.
    ContextMenu(context_menu::Event),
    /// Open an image or video in the media viewer.
    OpenViewer(String),
    /// The previous (-1) or next (+1) image or video.
    ViewerStep(i32),
    CloseViewer,
    /// Copy an image attachment's original to the clipboard, and how that
    /// went (copy_image.rs).
    #[cfg(not(target_arch = "wasm32"))]
    CopyImage(media::Media),
    #[cfg(not(target_arch = "wasm32"))]
    ImageCopied(Result<(), &'static str>),
    /// A viewer original (or a sharper one) decoded, or not; and the shown
    /// picture zoomed into (viewer.rs).
    Original(media::PreviewRef, Option<preview::Picture>),
    ViewerZoomed,
    #[cfg(target_os = "linux")]
    ViewerVideo(u64, bool),
    #[cfg(target_os = "linux")]
    Player(player::Event),
    #[cfg(target_os = "linux")]
    PlayerControl(player::Control),
    #[cfg(target_os = "linux")]
    PlayerScrub(f32),
    /// Playing a video or audio file in the timeline (inline_player.rs).
    #[cfg(target_os = "linux")]
    PlayInline(String, media::Media),
    #[cfg(target_os = "linux")]
    InlineFetched(u64, bool),
    #[cfg(target_os = "linux")]
    InlinePlayer(player::Event),
    #[cfg(target_os = "linux")]
    InlineControl(player::Control),
    #[cfg(target_os = "linux")]
    InlineScrub(f32),
    #[cfg(target_os = "linux")]
    InlineTick,
    /// The pointer moved over the playing video, or left it.
    #[cfg(target_os = "linux")]
    InlinePointer(bool),
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
    /// Open a page of User Settings (view/user_settings.rs).
    SettingsPage(view::user_settings::Page),
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
    /// The system tray and its settings (tray.rs).
    #[cfg(not(target_arch = "wasm32"))]
    Tray(tray::Event),
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
    Page(String),
    Members(String),
    Send(String, String, Option<rooms::Quote>),
    Context(String, String),
    Posting(permissions::Flight),
    Membership(moderation::Flight),
    Edit(String, String),
    Redact(String, String),
    Reaction(reactions::Toggle),
    Report,
    Search,
    IndexSearch,
    /// The browser: fetching a video to play in the viewer (viewer.rs).
    #[cfg(target_arch = "wasm32")]
    PlayVideo,
    /// Downloading an image's original to copy it (copy_image.rs).
    #[cfg(not(target_arch = "wasm32"))]
    CopyImage(tempfile::TempPath),
    Mentions,
    ReadState(String, Option<String>),
    ShareReads(bool, u64),
    ShareTyping(bool, u64),
    /// Saving our status; what it was before, to put back on failure
    /// (presence.rs).
    SaveStatus(presence::Choice, String),
    /// Saving a room's link preview switch (link_previews.rs).
    LinkPreviews(String, bool),
    LeaveRoom(String),
    /// Accepting or declining an invite (invites.rs).
    AcceptInvite(String),
    DeclineInvite(String),
    /// Joining a room by link or address (joining.rs).
    JoinRoom(String),
    /// Starting a DM with these people, and blocking or unblocking someone
    /// (dms.rs).
    StartDm,
    Block(String, bool),
    FollowUpgrade(String),
    /// Forwarding a message to this room (forward.rs).
    Forward(String),
    /// A page of the public room directory (explore.rs).
    Explore(engine::explore::DirectoryQuery),
    /// Starting, voting in or ending a poll (polls.rs).
    Poll(polls::Op),
    /// Sending a sticker or using a pack everywhere (emotes.rs).
    Pack(emotes::Op),
    /// Reordering or moving channels and categories (channel_order.rs).
    Arrange,
    /// Reordering the server rail or changing its folders (rail_order.rs).
    ArrangeRail,
    /// Setting a server nickname (names.rs).
    Nickname(names::Write),
    /// Reading or changing a space's settings (server_settings.rs).
    ServerSettings(server_settings::Op),
}
struct App {
    setup: Option<community::Form>,
    read_sharing: reading::Sharing,
    scrolled_away: bool,
    /// A screen or more above the newest messages: show Jump to present.
    scrolled_far: bool,
    /// The room whose automatic history load failed; Earlier messages retries.
    auto_page_failed: Option<String>,
    /// Where the open room's timeline was last scrolled to (paging.rs).
    scrolled: Option<(String, paging::Scrolled)>,
    /// Rooms whose timeline closed since they were loaded (paging.rs).
    released: BTreeSet<String>,
    /// The open room whose timeline is closed for a moment before it opens
    /// again, and the room that last started over at its newest messages.
    reopening: Option<String>,
    restarted: Option<String>,
    /// Another room opened: show its newest messages. The timeline keeps
    /// its scroll offset across rooms (Iced keeps it by position), so the
    /// last room's could open this one scrolled up, stuck at its top.
    snap_to_newest: bool,
    /// Each room's message count when history was last loaded to fill the
    /// window, so a page that brings nothing doesn't repeat.
    filled: BTreeMap<String, usize>,
    #[cfg(not(target_arch = "wasm32"))]
    focused: bool,
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    alerts: notifications::Alerts,
    #[cfg(target_os = "linux")]
    inbox: incoming::Inbox,
    #[cfg(target_os = "linux")]
    call: Option<desktop_call::ActiveCall>,
    /// A voice channel to join once the call that's ending has ended.
    #[cfg(target_os = "linux")]
    join_after: Option<String>,
    /// Mute, deafen and the chosen devices, in and between calls.
    #[cfg(target_os = "linux")]
    local_audio: desktop_call::LocalAudio,
    /// The open device menu or voice settings popup.
    #[cfg(target_os = "linux")]
    voice_popup: Option<desktop_call::VoiceMenu>,
    #[cfg(target_os = "linux")]
    hotkeys: hotkeys::State,
    #[cfg(target_os = "linux")]
    drop_epoch: u64,
    #[cfg(not(target_arch = "wasm32"))]
    paste_seq: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pasting: bool,
    commands: Sender<backend::Command>,
    /// Set when UNTETHERED_ENGINE=matrix-sdk replaces gomuks.
    sdk: Option<Sender<engine::Command>>,
    discussions: view::discussions::State,
    sdk_ui: engine::account_ui::AccountUi,
    outbox: media::Outbox,
    inline: inline::InlineImages,
    link_previews: link_previews::LinkPreviews,
    /// The invite whose card the main area shows, and an accepted invite's
    /// room to open once it's listed (invites.rs).
    invite_open: Option<String>,
    joined_invite: Option<String>,
    /// The message a link pointed at, to show once its room opens.
    joined_event: Option<String>,
    /// A linked room's preview in the main area, and Join a Server's field
    /// (joining.rs).
    room_preview: Option<joining::Preview>,
    join_address: String,
    /// New message, a DM it started to open once listed, and who this
    /// account blocks (dms.rs).
    new_message: Option<dms::NewMessage>,
    /// Forward To, for a message (forward.rs).
    forwarding: Option<forward::Form>,
    /// Edit Server Profile, and rooms that took their server's nickname
    /// (names.rs).
    names: names::State,
    /// The open Server Settings page (server_settings.rs).
    server_settings: server_settings::State,
    /// Explore, and what spaces contain (explore.rs).
    explore: explore::State,
    /// Create a Poll, and the composer's + menu that opens it (polls.rs).
    polling: Option<polls::Form>,
    /// Image packs, their pictures and the sticker picker (emotes.rs).
    emotes: emotes::Emotes,
    /// A channel or category being dragged in the sidebar (channel_order.rs).
    order: channel_order::Drag,
    /// The server rail's folders and dragging in it (rail_order.rs).
    rail: rail_order::Rail,
    composer_menu: bool,
    /// The composer's keys (composer.rs).
    composer: composer::State,
    started_dm: Option<String>,
    /// Ctrl+K's quick switcher (switcher.rs).
    switcher: Option<switcher::Switcher>,
    /// Ctrl+/'s list of shortcuts (shortcuts.rs).
    shortcuts: shortcuts::Shortcuts,
    /// Message and call sounds, and their switches (sounds.rs).
    #[cfg(target_os = "linux")]
    sounds: sounds::Sounds,
    /// The Voice & Video settings and the page's tests (call_settings.rs).
    #[cfg(target_os = "linux")]
    voice: call_settings::Settings,
    /// The page User Settings shows (view/user_settings.rs).
    settings_page: view::user_settings::Page,
    /// How the last Copy Image went, and a fetched original waiting to be
    /// put on the clipboard (copy_image.rs).
    copy_note: Option<&'static str>,
    #[cfg(not(target_arch = "wasm32"))]
    copy_ready: Option<tempfile::TempPath>,
    /// Where you were in each room and section (places.rs): what's
    /// remembered, unsaved, restored after signing in, scrolled since last
    /// noted, and the room being taken back to its place.
    places: places::Places,
    places_dirty: bool,
    places_restored: bool,
    place_unnoted: bool,
    returning: Option<places::Return>,
    blocked: BTreeSet<String>,
    avatars: avatars::Avatars,
    viewer: Option<viewer::Viewer>,
    originals: viewer::Originals,
    /// The video or audio playing in the timeline (inline_player.rs).
    #[cfg(target_os = "linux")]
    inline_player: Option<inline_player::InlinePlayer>,
    /// The account's profile picture (mxc URI).
    own_avatar: Option<String>,
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
    drafts_dirty: bool,
    reply_drafts: BTreeMap<String, rooms::Quote>,
    original: Option<replies::Original>,
    // Exclusive timestamp/row-ID boundary. None follows the latest message.
    timeline_end: Option<(i64, i64)>,
    timeline_restoring: bool,
    search: Option<search::Search>,
    mentions: Option<mentions::Inbox>,
    editing: Option<edits::Editing>,
    /// Spoilers clicked open: each message's spoiler numbers, by event.
    spoilers: BTreeMap<String, BTreeSet<u16>>,
    /// The code block copied last, which shows "Copied".
    copied_code: Option<String>,
    reporting: Option<reports::Draft>,
    reaction_picker: Option<reactions::Picker>,
    editor: text_editor::Content,
    completion: autocomplete::State,
    typing: typing::Typing,
    presence: presence::State,
    loaded: BTreeSet<String>,
    connected: bool,
    logged_in: bool,
    verified: bool,
    account: bool,
    people: bool,
    server_menu: bool,
    context_menu: context_menu::State,
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
    #[cfg(not(target_arch = "wasm32"))]
    tray: tray::Tray,
}
impl App {
    fn boot(
        #[cfg(not(target_arch = "wasm32"))] launches: Option<
            async_channel::Receiver<instance::Launch>,
        >,
    ) -> (Self, Task<Msg>) {
        #[allow(unused_mut)]
        let (mut app, task) = Self::boot_engine();
        // Portal users await this too; starting it here keeps it off their path.
        #[cfg(target_os = "linux")]
        let task = Task::batch([Task::future(app_id::portal_ready()).discard(), task]);
        #[cfg(target_os = "linux")]
        {
            app.sounds.settings = sounds::Settings::load();
            app.load_voice();
        }
        // The window, unless this is a start at sign-in, and the tray icon.
        #[cfg(not(target_arch = "wasm32"))]
        let task = Task::batch([task, app.boot_tray(launches)]);
        (app, task)
    }
    fn new(commands: Sender<backend::Command>) -> Self {
        Self {
            setup: None,
            read_sharing: Default::default(),
            scrolled_away: false,
            scrolled_far: false,
            auto_page_failed: None,
            scrolled: None,
            released: Default::default(),
            reopening: None,
            restarted: None,
            snap_to_newest: false,
            filled: BTreeMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            focused: false,
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            alerts: Default::default(),
            #[cfg(target_os = "linux")]
            call: None,
            #[cfg(target_os = "linux")]
            join_after: None,
            #[cfg(target_os = "linux")]
            local_audio: Default::default(),
            #[cfg(target_os = "linux")]
            voice_popup: None,
            #[cfg(target_os = "linux")]
            hotkeys: Default::default(),
            #[cfg(target_os = "linux")]
            inbox: Default::default(),
            #[cfg(target_os = "linux")]
            drop_epoch: 0,
            #[cfg(not(target_arch = "wasm32"))]
            paste_seq: 0,
            #[cfg(not(target_arch = "wasm32"))]
            pasting: false,
            commands,
            sdk: None,
            discussions: Default::default(),
            sdk_ui: Default::default(),
            outbox: Default::default(),
            inline: Default::default(),
            link_previews: Default::default(),
            invite_open: None,
            joined_invite: None,
            joined_event: None,
            room_preview: None,
            join_address: String::new(),
            new_message: None,
            forwarding: None,
            names: Default::default(),
            server_settings: Default::default(),
            explore: Default::default(),
            polling: None,
            emotes: Default::default(),
            order: Default::default(),
            rail: Default::default(),
            composer_menu: false,
            composer: Default::default(),
            started_dm: None,
            switcher: None,
            shortcuts: Default::default(),
            #[cfg(target_os = "linux")]
            sounds: Default::default(),
            #[cfg(target_os = "linux")]
            voice: Default::default(),
            settings_page: Default::default(),
            copy_note: None,
            #[cfg(not(target_arch = "wasm32"))]
            copy_ready: None,
            places: Default::default(),
            places_dirty: false,
            places_restored: false,
            place_unnoted: false,
            returning: None,
            blocked: BTreeSet::new(),
            avatars: Default::default(),
            viewer: None,
            originals: Default::default(),
            #[cfg(target_os = "linux")]
            inline_player: None,
            own_avatar: None,
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
            drafts_dirty: false,
            reply_drafts: Default::default(),
            original: None,
            timeline_end: None,
            timeline_restoring: false,
            search: None,
            mentions: None,
            editing: None,
            spoilers: BTreeMap::new(),
            copied_code: None,
            reporting: None,
            reaction_picker: None,
            editor: text_editor::Content::new(),
            completion: Default::default(),
            typing: Default::default(),
            presence: Default::default(),
            loaded: Default::default(),
            connected: false,
            logged_in: false,
            verified: false,
            account: false,
            people: false,
            server_menu: false,
            context_menu: Default::default(),
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
            #[cfg(not(target_arch = "wasm32"))]
            tray: Default::default(),
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
    /// Requests still built in gomuks's RPC shape; the engine translates them
    /// (`engine::app::engine_rpc`).
    fn rpc(&mut self, cmd: &str, data: Value, req: Request) {
        self.engine_rpc(cmd, data, req);
    }
    fn page(&mut self) {
        self.engine_page()
    }
    /// A history request for `room` is in flight.
    fn paging(&self, room: &str) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Page(r) if r == room))
    }
    /// Load older messages until the open room has a screenful, as Discord
    /// does on open; stop when a page brings nothing new.
    fn fill_history(&mut self) {
        const SCREENFUL: usize = 40;
        let room = self.selected.clone();
        let Some(r) = self.live.list.get(&room) else {
            return;
        };
        let count = r.posts().len();
        if !self.logged_in
            || !self.connected
            || r.end
            || count >= SCREENFUL
            || self.original.is_some()
            || self.paging(&room)
            || self.auto_page_failed.as_ref() == Some(&room)
            || self.filled.get(&room).is_some_and(|last| *last >= count)
        {
            return;
        }
        self.filled.insert(room, count);
        self.page();
    }
    #[cfg(target_os = "linux")]
    fn minimize_call(&mut self) {
        if let Some(call) = &mut self.call {
            call.stage_open = false;
            call.theater = false;
        }
    }

    fn open(&mut self, id: String) {
        // A room replaced in an upgrade opens as its newest room (upgrades.rs).
        let id = self.rooms().newest(&id).to_owned();
        if self.selected == id {
            return;
        }
        #[cfg(target_os = "linux")]
        self.minimize_call();
        self.close_original();
        self.close_viewer();
        self.scrolled_away = false;
        self.scrolled_far = false;
        self.discussions = Default::default();
        self.reset_directory();
        self.leave_permission_panel();
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        {
            self.alerts.panel = false;
        }
        self.clear_search();
        self.editing = None;
        self.stop_typing();
        self.record_current_draft();
        self.editor =
            text_editor::Content::with_text(self.drafts.get(&id).map(String::as_str).unwrap_or(""));
        let left = std::mem::replace(&mut self.selected, id.clone());
        self.release_timeline(left);
        self.emotes.stickers = false;
        #[cfg(target_os = "linux")]
        self.stop_inline();
        self.snap_to_newest = true;
        self.timeline_end = None;
        if self.people {
            self.read_directory(None);
        }
        if self.logged_in {
            self.load_selected();
        } else if let Some(r) = self.demo.list.get_mut(&id) {
            r.unread = false;
        }
        // Back to where it was scrolled to, and remembered as open here.
        self.start_return();
        self.note_open();
    }
    /// Ask for the selected room's history and members, once per session,
    /// when this device can read them. A room selected before the device was
    /// verified (right after signing in) loads once it is.
    fn load_selected(&mut self) {
        let id = self.selected.clone();
        if !self.logged_in || !self.verified || id.is_empty() {
            return;
        }
        if self.loaded.insert(id.clone()) {
            self.released.remove(&id);
            self.page();
            self.rpc(
                "get_room_state",
                json!({"room_id":id,"include_members":true}),
                Request::Members(id),
            );
        } else if self.released.contains(&id) {
            // Its timeline closed when it was left (paging.rs); offline, this
            // runs again after each update until it opens.
            self.page();
            if self.paging(&id) {
                self.released.remove(&id);
            }
        }
    }
    fn reconcile(&mut self) {
        if let Section::Space(s) = &self.section
            && !self.rooms().list.contains_key(s)
        {
            // A space replaced in an upgrade gives way to the new space.
            let newest = self.rooms().newest(s);
            self.section = match self.rooms().list.get(newest) {
                Some(r) if r.space => Section::Space(newest.to_owned()),
                _ => Section::All,
            };
        }
        if !self.visible().any(|(id, _)| id == &self.selected) {
            // An upgraded room gives way to the room that replaced it.
            let newest = self.rooms().newest(&self.selected);
            let first = self
                .visible()
                .find(|(id, _)| *id == newest)
                .or_else(|| self.visible().next())
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            self.open(first);
        }
        // A space's channels this account hasn't joined (explore.rs).
        self.ask_space_rooms();
    }
    fn verifying(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Verify(_)))
    }
    fn verification_feedback(&self) -> &str {
        if !self.connected {
            &self.status
        } else if self.verifying() {
            "Verifying with your homeserver…"
        } else if !self.verification_note.is_empty() {
            &self.verification_note
        } else if self.recovery_unavailable {
            "Recovery isn’t set up for this account. Check recovery in a trusted Matrix client."
        } else {
            "Use the key or passphrase from Matrix recovery setup. Your sign-in password is separate. Grouping spaces are fine."
        }
    }
    /// Status frames from a running desktop call (`desktop_call.rs`).
    #[cfg(target_os = "linux")]
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
                            self.stop_engine();
                        } else if let Some(room) = self.join_after.take() {
                            self.join_channel(room);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        if matches!(
            msg,
            Msg::Setup(_) | Msg::Section(_) | Msg::Open(_) | Msg::Account | Msg::OpenMentions
        ) {
            self.server_menu = false;
            self.context_menu.open = None;
            self.sdk_ui.profile.dismiss_card();
            self.presence.menu = false;
        }
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        if matches!(msg, Msg::SpaceAlerts) {
            self.server_menu = false;
        }
        match msg {
            Msg::Discussion(action) => {
                let focus = matches!(action, view::discussions::Action::Emoji(_));
                self.discussion(action);
                if focus {
                    return operation::focus("thread-composer");
                }
            }
            Msg::PickThreadImage => return self.pick_thread_image(),
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
                    return self.talk_key(hotkeys::Source::Window, false);
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
                            ));
                            self.composer_changed();
                            self.record_current_draft();
                        }
                        Err(reason) if self.selected == room => self.status = reason.into(),
                        _ => {}
                    }
                }
            }
            Msg::InlineReady(epoch, key, img) => return self.inline.complete(epoch, key, img),
            Msg::RetryInline(epoch, key) => self.inline.retry(epoch, &key),
            Msg::AvatarReady(mxc, img) => self.avatars.complete(mxc, img),
            Msg::OpenViewer(event) => return self.open_viewer(event),
            Msg::ViewerStep(delta) => return self.step_viewer(delta),
            Msg::CloseViewer => self.close_viewer(),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::CopyImage(media) => self.copy_image(media),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::ImageCopied(result) => self.image_copied(result),
            Msg::Original(source, img) => self.original_loaded(source, img),
            Msg::ViewerZoomed => return self.viewer_zoomed(),
            #[cfg(target_os = "linux")]
            Msg::ViewerVideo(epoch, fetched) => self.viewer_video(epoch, fetched),
            #[cfg(target_os = "linux")]
            Msg::Player(event) => self.player_event(event),
            #[cfg(target_os = "linux")]
            Msg::PlayerControl(control) => self.player_control(control),
            #[cfg(target_os = "linux")]
            Msg::PlayInline(event, media) => return self.play_inline(event, media),
            #[cfg(target_os = "linux")]
            Msg::InlineFetched(epoch, ok) => self.inline_fetched(epoch, ok),
            #[cfg(target_os = "linux")]
            Msg::InlinePlayer(event) => self.inline_event(event),
            #[cfg(target_os = "linux")]
            Msg::InlineControl(control) => self.inline_control(control),
            #[cfg(target_os = "linux")]
            Msg::InlineScrub(at) => self.inline_scrub(at),
            #[cfg(target_os = "linux")]
            Msg::InlineTick => self.inline_tick(),
            #[cfg(target_os = "linux")]
            Msg::InlinePointer(over) => self.inline_pointer(over),
            #[cfg(target_os = "linux")]
            Msg::PlayerScrub(at) => self.player_scrub(at),
            Msg::ShowInline(key) => self.inline.request(key),
            Msg::Engine(event) => {
                let jumped = matches!(event, engine::Event::Context { .. });
                #[cfg(target_os = "linux")]
                let video =
                    matches!(&event, engine::Event::CallFrame(f) if f["command"] == "video_redraw");
                self.engine_event(event);
                if jumped && self.original.is_some() {
                    return self.anchor_history();
                }
                // A call video tile has a new frame to upload (desktop_call.rs).
                #[cfg(target_os = "linux")]
                if video {
                    return self.upload_video();
                }
            }
            Msg::Sdk(action) => return self.sdk_action(action),
            Msg::Stopped(s) => {
                #[cfg(target_os = "linux")]
                {
                    self.call = None;
                    self.join_after = None;
                    self.clear_ring();
                }
                self.stopped = true;
                self.connected = false;
                self.pending.clear();
                self.typing.forget();
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
            // The desktop may close to the tray instead (tray.rs).
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Close => {
                self.record_current_draft();
                self.persist_drafts();
                self.persist_places();
                return self.close_requested();
            }
            #[cfg(target_arch = "wasm32")]
            Msg::Close => return self.quit(),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Tray(event) => return self.tray_event(event),
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
                self.join_after = None;
                if let Some(call) = &mut self.call {
                    call.cancel.close();
                    call.label = "Ending call…".into();
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ToggleMic => self.toggle_mute(),
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
            Msg::ToggleDeafen => self.toggle_deafen(),
            #[cfg(target_os = "linux")]
            Msg::RoomPtt(enabled) => {
                self.set_push_to_talk(enabled);
                return self.save_voice();
            }
            #[cfg(target_os = "linux")]
            Msg::TalkKey(down) => return self.talk_key(hotkeys::Source::Window, down),
            #[cfg(target_os = "linux")]
            Msg::Hotkey(event) => return self.hotkey(event),
            #[cfg(target_os = "linux")]
            Msg::TalkRelease(until) => self.talk_release(until),
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
            Msg::VideoUploaded(sid, frame) => return self.video_uploaded(sid, frame),
            #[cfg(target_os = "linux")]
            Msg::StageScroll(id, area, report) => self.stage_scrolled(id, area, report),
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
                        .is_none_or(|s| self.local_audio.devices.streams.contains(s))
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
            Msg::PeerVolume(id, user, percent) => {
                if let Some(call) = &self.call
                    && (call.group || call.peer.as_ref() == Some(&user))
                    && call.id == id
                    && !call.cancel.is_closed()
                    && user != self.matrix_id
                {
                    call.audio.send_if_modified(|s| s.set_volume(user, percent));
                }
            }
            #[cfg(target_os = "linux")]
            Msg::ListCameras => return self.list_cameras(),
            #[cfg(target_os = "linux")]
            Msg::CamerasListed(list) => self.cameras_listed(list),
            #[cfg(target_os = "linux")]
            Msg::SelectCamera(choice) => return self.select_camera(choice),
            #[cfg(target_os = "linux")]
            Msg::VoiceMenu(menu) => return self.voice_menu(menu),
            #[cfg(target_os = "linux")]
            Msg::RefreshDevices => return self.refresh_devices(),
            #[cfg(target_os = "linux")]
            Msg::DevicesListed(result) => self.devices_listed(result),
            #[cfg(target_os = "linux")]
            Msg::SelectDevice(port, device) => return self.select_device(port, device),
            #[cfg(target_os = "linux")]
            Msg::QaLayout(None) => return qa_layout::collect().map(|f| Msg::QaLayout(Some(f))),
            #[cfg(target_os = "linux")]
            Msg::QaLayout(Some(found)) => qa_layout::write(&found),
            Msg::CheckCalls
                if self.logged_in
                    && self.connected
                    && !self
                        .pending
                        .values()
                        .any(|p| matches!(p, Request::RtcDiscovery)) =>
            {
                self.rtc_status = "Checking MatrixRTC transports…".into();
                self.command(
                    |id| engine::Command::RtcTransports { id },
                    Request::RtcDiscovery,
                );
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
                if let Some(picker) = room
                    .strip_prefix("thread-image:")
                    .and_then(|id| id.parse().ok())
                {
                    self.thread_image_picked(picker, files.into_iter().next());
                    return operation::focus("thread-composer");
                }
                if let Some(picker) = room
                    .strip_prefix(server_settings::PICKER)
                    .and_then(|id| id.parse().ok())
                {
                    return self.server_settings_picked(picker, files.into_iter().next());
                }
                #[cfg(target_arch = "wasm32")]
                if let Some(id) = room
                    .strip_prefix(engine::profile::AVATAR_PICKER)
                    .and_then(|id| id.parse().ok())
                {
                    return self.profile_picked(id, files.into_iter().next());
                }
                self.hover_files = false;
                if room.is_empty() {
                    room = self.selected.clone();
                }
                if self.can_stage() && self.rooms().list.contains_key(&room) {
                    // Dropped, pasted or picked files are sent with Enter in
                    // the composer, as on Discord.
                    let focus = if room == self.selected {
                        operation::focus("composer")
                    } else {
                        Task::none()
                    };
                    return Task::batch([self.stage_files(room, files), focus]);
                }
            }
            Msg::Thumbnail(room, id, thumb) => {
                if let Some(a) = self.outbox.get_mut(&room, id) {
                    a.thumbnail = thumb;
                }
            }
            Msg::PickFiles if self.can_stage() => {
                self.composer_menu = false;
                let room = self.selected.clone();
                #[cfg(not(target_arch = "wasm32"))]
                return Task::perform(
                    async {
                        #[cfg(target_os = "linux")]
                        app_id::portal_ready().await;
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
                            #[cfg(target_os = "linux")]
                            app_id::portal_ready().await;
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
                            play: false,
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
                self.invite_open = None;
                self.room_preview = None;
                self.section = s;
                // Back to the room last open there (places.rs).
                self.open_last_in_section();
                self.reconcile();
                self.note_open();
            }
            Msg::Open(id) => {
                #[cfg(target_os = "linux")]
                self.minimize_call();
                self.cancel_setup_if_idle();
                self.close_mentions();
                self.invite_open = None;
                self.room_preview = None;
                self.open(id);
            }
            Msg::OpenVoice(id) => {
                let task = self.update(Msg::Open(id.clone()));
                #[cfg(target_os = "linux")]
                self.join_channel(id);
                return task;
            }
            Msg::OpenMentions => {
                #[cfg(target_os = "linux")]
                self.minimize_call();
                self.cancel_setup_if_idle();
                self.open_mentions();
            }
            Msg::CheckReading => return self.check_reading(),
            Msg::ReopenTimeline(room) => self.reopen_timeline(room),
            Msg::LinkPreviews(room, on) => self.set_link_previews(room, on),
            Msg::OpenInvite(id) => {
                self.room_preview = None;
                self.open_invite(id);
            }
            Msg::JoinPreviewed => self.join_previewed(),
            Msg::ClosePreview => self.close_preview(),
            Msg::JoinAddressInput(s) => self.join_address = s,
            Msg::JoinAddress => self.join_address(),
            Msg::Explore(input) => return self.explore(input),
            Msg::Dm(input) => return self.new_message(input),
            Msg::Forward(input) => return self.forward(input),
            Msg::Poll(input) => return self.poll(input),
            Msg::Emote(input) => self.emote(input),
            Msg::EmoteReady(url, picture) => self.emotes.complete(url, picture),
            Msg::Order(input) => return self.arrange_input(input),
            Msg::Rail(input) => return self.rail_input(input),
            Msg::Names(input) => return self.names(input),
            Msg::ServerSettings(input) => {
                let task = self.server_settings(input);
                return Task::batch([task, self.schedule_settings_pictures()]);
            }
            Msg::ComposerMenu(open) => self.composer_menu = open && self.can_stage(),
            Msg::Switcher(input) => return self.switcher(input),
            Msg::Escape if self.shortcuts.open => self.shortcuts.open = false,
            Msg::Shortcut(input) => return self.shortcut(input),
            #[cfg(target_os = "linux")]
            Msg::Sound(event) => return self.sound_event(event),
            #[cfg(target_os = "linux")]
            Msg::Voice(event) => return self.voice_event(event),
            #[cfg(target_os = "linux")]
            Msg::VoiceMeter => {}
            Msg::NotePlace => return self.note_place(),
            Msg::PlaceNoted(room, hit) => self.place_noted(room, hit),
            Msg::SavePlaces => self.persist_places(),
            Msg::AcceptInvite(id) => self.accept_invite(id),
            Msg::DeclineInvite(id) => self.decline_invite(id),
            Msg::ReadingScroll(room, at) if room == self.selected && self.original.is_none() => {
                if at.away {
                    // Scrolled up: the next return may start over (paging.rs).
                    self.restarted = None;
                }
                self.scrolled_away = at.away;
                self.scrolled_far = at.far;
                self.inline.depth = at.depth;
                self.place_moved(&room, at.away);
                self.scrolled = Some((room, at));
            }
            Msg::PageScroll(up) if self.viewer.is_none() => {
                // Anchored at the bottom, a positive offset moves toward older messages.
                let step = 600.;
                return iced::widget::operation::scroll_by(
                    self.timeline_id(),
                    iced::widget::operation::AbsoluteOffset {
                        x: 0.,
                        y: if up { step } else { -step },
                    },
                );
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
            Msg::Escape if self.context_menu.is_open() => self.context_menu.dismiss(),
            Msg::Escape if self.order.held.is_some() => self.order = Default::default(),
            Msg::Escape if self.rail.busy() => {
                self.rail.escape();
            }
            Msg::Escape if self.sdk_ui.profile.card.is_some() => {
                return self.profile_action(engine::profile::Action::Close);
            }
            Msg::ContextMenu(e) => return self.context_menu_event(e),
            #[cfg(target_os = "linux")]
            Msg::Escape if self.voice.recording && self.voice_page_open() => {
                return self.voice_event(call_settings::Event::RecordKey(false));
            }
            #[cfg(target_os = "linux")]
            Msg::Escape if self.voice_popup.is_some() => self.voice_popup = None,
            Msg::FreeEscape => return self.free_escape(),
            Msg::Escape => {
                if self.switcher.is_some() {
                    return self.switcher(switcher::Input::Close);
                } else if self.viewer.is_some() {
                    self.close_viewer();
                } else if self.new_message.is_some() {
                    return self.new_message(dms::Input::Close);
                } else if self.forwarding.is_some() {
                    return self.forward(forward::Input::Close);
                } else if self.polling.is_some() {
                    return self.poll(polls::Input::Close);
                } else if self.names.form.is_some() {
                    return self.names(names::Input::Close);
                } else if self.composer_menu {
                    self.composer_menu = false;
                } else if self.emotes.stickers {
                    self.emotes.stickers = false;
                } else if self.presence.dialog.is_some() || self.presence.menu {
                    self.presence.dialog = None;
                    self.presence.menu = false;
                } else if self.server_menu {
                    self.server_menu = false;
                } else if self.setup.is_some() && !self.setup_busy() {
                    return self.setup_input(community::Input::Close);
                } else if self.server_settings.page.is_some() {
                    return self.server_settings(server_settings::Input::Escape);
                } else if self.account && self.logged_in && self.verified {
                    self.account = false;
                } else if self.explore.page.is_some() {
                    return self.explore(explore::Input::Close);
                } else if self.reaction_picker.is_some() {
                    self.reaction_picker = None;
                } else if self.completion.picker.take().is_none() && self.escape_is_free() {
                    return self.escape_fallback();
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
                {
                    self.minimize_call();
                    self.reset_voice_page();
                }
                self.close_mentions();
                self.account = true;
                self.settings_page = view::user_settings::Page::Account;
            }
            Msg::SettingsPage(page) => {
                let opened = self.update(Msg::Account);
                self.settings_page = page;
                let top = iced::widget::operation::scroll_to(
                    view::user_settings::SCROLL,
                    iced::widget::operation::AbsoluteOffset {
                        x: None,
                        y: Some(0.),
                    },
                );
                #[cfg(target_os = "linux")]
                {
                    self.voice_popup = None;
                    if page == view::user_settings::Page::Voice {
                        // The page names the chosen devices from the list.
                        return Task::batch([opened, top, self.refresh_devices()]);
                    }
                }
                return Task::batch([opened, top]);
            }
            Msg::Back => self.account = false,
            Msg::Server(s) => self.server = s,
            Msg::Username(s) => self.username = s,
            Msg::Password(s) => self.password = s,
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Recovery(s) => self.recovery = s,
            Msg::Login => self.engine_login(),
            #[cfg(not(target_arch = "wasm32"))]
            Msg::Verify => self.engine_verify(),
            #[cfg(target_arch = "wasm32")]
            Msg::SubmitWebRecovery(s) => {
                self.recovery = s;
                self.engine_verify();
                self.recovery.clear();
            }
            Msg::Draft(a) => {
                let edit = a.is_edit();
                self.draft(a);
                if edit {
                    self.composer_changed();
                    self.record_current_draft();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Msg::SaveDrafts => self.persist_drafts(),
            Msg::Autocomplete(input) => {
                let emoji = matches!(input, autocomplete::Input::PickEmoji(_));
                let task = self.autocomplete(input);
                self.record_current_draft();
                if emoji {
                    self.composer_changed();
                }
                // An opened picker or popup shows custom emoji (emotes.rs).
                return Task::batch([task, self.schedule_emotes()]);
            }
            Msg::Typing(input) => self.typing_input(input),
            Msg::Presence(input) => return self.presence_input(input),
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
            Msg::RunSearch => self.submit_search(),
            Msg::SearchScope(scope) => self.search_scope(scope),
            Msg::IndexSearch => self.index_search(),
            Msg::SearchHit(room, id) => return self.open_hit(room, id),
            Msg::SearchResults => self.close_original(),
            Msg::ReturnToReading => return self.return_to_reading(),
            Msg::ReactionPicker(id) => {
                let task = self.toggle_reaction_picker(id, false);
                return Task::batch([task, self.schedule_emotes()]);
            }
            Msg::ReactionPickerChips(id) => {
                let task = self.toggle_reaction_picker(id, true);
                return Task::batch([task, self.schedule_emotes()]);
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
                    let typed = autocomplete::typed(&a);
                    d.input.perform(a);
                    self.edited(autocomplete::Field::Edit, typed);
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
                self.scrolled_away = false;
                self.scrolled_far = false;
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
            Msg::Send if self.logged_in => {
                self.stop_typing();
                self.engine_compose();
            }
            Msg::Send => {
                // The signed-out preview: a local echo only.
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
                if let Some(r) = self.demo.list.get_mut(&id) {
                    r.insert(&json!({"rowid":r.history.keys().next_back().copied().unwrap_or(0)+1,"sender":"You","type":"m.room.message",
                        "timestamp":chrono::Utc::now().timestamp_millis(),"content":{"body":body}}));
                    self.editor = text_editor::Content::new();
                }
            }
            Msg::More => {
                self.auto_page_failed = None;
                self.page();
            }
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
                    return context_menu::copy(text);
                }
            }
            Msg::CopyCode(code) => {
                self.copied_code = Some(code.clone());
                return context_menu::copy(code);
            }
            Msg::OpenLink(url) => {
                if !self.open_permalink(&url) && !self.open_room_link(&url) {
                    rich::open(&url)
                } else if self.original.is_some() {
                    return self.anchor_history();
                }
            }
            Msg::FollowUpgrade(room) => self.follow_upgrade(room),
            Msg::RevealSpoiler(event, spoiler) => {
                self.spoilers.entry(event).or_default().insert(spoiler);
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
                Field::Composer(room) if self.selected == room && !self.account => {
                    self.editor
                        .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                            s.into(),
                        )));
                    self.composer_changed();
                    self.record_current_draft();
                }
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
        self.prune_context_menu();
        // Previews and pictures for what's on screen now, also once a room's
        // messages arrive after it opened; both skip what they have.
        let snap = if std::mem::take(&mut self.snap_to_newest) {
            self.anchor_history()
        } else {
            Task::none()
        };
        let task = Task::batch([
            snap,
            self.schedule_inline(),
            self.keep_paging(),
            self.bound_history(),
            self.continue_return(),
        ]);
        #[cfg(not(target_arch = "wasm32"))]
        let task = Task::batch([task, self.copy_fetched()]);
        self.fill_history();
        self.sync_presence();
        self.schedule_link_previews();
        let task = Task::batch([
            task,
            self.schedule_avatars(),
            self.schedule_emotes(),
            self.schedule_settings_pictures(),
        ]);
        #[cfg(target_os = "linux")]
        let task = Task::batch([task, self.start_ringtone(), self.play_sounds()]);
        #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
        let task = Task::batch([task, self.send_alerts()]);
        #[cfg(not(target_arch = "wasm32"))]
        let task = Task::batch([task, self.sync_tray()]);
        task
    }
    /// Quit: end the call first (its last frame stops the engine), then stop
    /// the engine, and exit once it has stopped.
    fn quit(&mut self) -> Task<Msg> {
        self.record_current_draft();
        self.persist_drafts();
        self.persist_places();
        self.closing = true;
        #[cfg(target_os = "linux")]
        self.clear_ring();
        #[cfg(target_os = "linux")]
        if let Some(call) = &self.call {
            call.cancel.close();
            return Task::none();
        }
        self.stop_engine();
        if self.stopped {
            return iced::exit();
        }
        Task::none()
    }
    fn stop_engine(&self) {
        self.commands.close();
        if let Some(sdk) = &self.sdk {
            sdk.close();
        }
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
    /// The daemon's view and title for its one window.
    #[cfg(not(target_arch = "wasm32"))]
    fn window_view(&self, _: iced::window::Id) -> iced::Element<'_, Msg> {
        // Call video plays only for the tiles this view draws.
        #[cfg(target_os = "linux")]
        return self.watch_video(|| self.view());
        #[cfg(not(target_os = "linux"))]
        self.view()
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn window_title(&self, _: iced::window::Id) -> String {
        self.title()
    }
}
/// The main window. On the desktop it opens at start and again from the
/// tray (tray.rs).
fn main_window() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1180., 760.),
        min_size: Some(iced::Size::new(760., 500.)),
        exit_on_close_request: false,
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: app_id::APP_ID.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn main() -> iced::Result {
    // The matrix-sdk engine's browser worker loads this module too. It starts
    // through `untethered_engine_worker` instead (engine/worker.rs).
    #[cfg(target_arch = "wasm32")]
    if web_sys::window().is_none() {
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    runtime::limit_arenas();
    // Call fixtures (debug builds): prepare a matrix-sdk profile and exit.
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    if let Some(seed) = std::env::var_os("UNTETHERED_QA_SEED") {
        std::process::exit(engine::qa::seed(&seed.to_string_lossy()));
    }
    // One Untethered per profile: launching it again shows the running
    // window (instance.rs). Kept until the app exits.
    #[cfg(not(target_arch = "wasm32"))]
    let instance = match instance::start(std::env::args().any(|a| a == tray::HIDDEN_FLAG)) {
        instance::Start::First(instance) => Some(instance),
        instance::Start::Handed => {
            eprintln!("Untethered is running already.");
            return Ok(());
        }
        instance::Start::Unguarded => None,
    };
    #[cfg(not(target_arch = "wasm32"))]
    let launches = instance.as_ref().map(|i| i.launches.clone());
    #[cfg(not(target_arch = "wasm32"))]
    fonts::configure();
    // On the desktop Untethered keeps running in the tray with no window
    // (tray.rs), which an iced daemon allows; it opens the window itself.
    #[cfg(not(target_arch = "wasm32"))]
    let app = iced::daemon(
        move || App::boot(launches.clone()),
        App::update,
        App::window_view,
    )
    .title(App::window_title)
    .theme(Theme::Dark)
    .executor::<runtime::Runtime>();
    #[cfg(target_arch = "wasm32")]
    let app = iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .theme(Theme::Dark)
        .window(main_window());
    app.font(include_bytes!("../assets/LiberationSans-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/LiberationSans-Bold.ttf").as_slice())
        // Italics and code in messages (rich.rs), the same on every platform.
        .font(include_bytes!("../assets/LiberationSans-Italic.ttf").as_slice())
        .font(include_bytes!("../assets/LiberationMono-Regular.ttf").as_slice())
        .default_font(iced::Font::with_name("Liberation Sans"))
        .subscription(|app| {
            iced::Subscription::batch([
                if app.auto_target().is_some() {
                    iced::time::every(std::time::Duration::from_millis(750))
                        .map(|_| Msg::CheckReading)
                } else {
                    iced::Subscription::none()
                },
                #[cfg(not(target_arch = "wasm32"))]
                if app.drafts_dirty {
                    iced::time::every(std::time::Duration::from_millis(500))
                        .map(|_| Msg::SaveDrafts)
                } else {
                    iced::Subscription::none()
                },
                app.typing_subscription(),
                app.presence_subscription(),
                app.order_subscription(),
                app.rail_subscription(),
                // A moment after the timeline stops moving, note the place.
                if app.place_unnoted {
                    iced::time::every(std::time::Duration::from_millis(400)).map(|_| Msg::NotePlace)
                } else {
                    iced::Subscription::none()
                },
                #[cfg(not(target_arch = "wasm32"))]
                if app.places_dirty {
                    iced::time::every(std::time::Duration::from_secs(1)).map(|_| Msg::SavePlaces)
                } else {
                    iced::Subscription::none()
                },
                // Ctrl+K opens the quick switcher, also while typing, as on
                // Discord; while it's open, the arrow keys move through it.
                iced::event::listen_with(|event, _, _| match event {
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Character(c),
                        modifiers,
                        ..
                    }) if modifiers.command() && c.as_str().eq_ignore_ascii_case("k") => {
                        Some(Msg::Switcher(switcher::Input::Toggle))
                    }
                    _ => None,
                }),
                // Alt+Shift+↓, Shift+Escape and Ctrl+/ (shortcuts.rs).
                shortcuts::subscription(),
                if app.switcher.is_some() {
                    iced::event::listen_with(|event, _, _| match event {
                        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp),
                            ..
                        }) => Some(Msg::Switcher(switcher::Input::Step(-1))),
                        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown),
                            ..
                        }) => Some(Msg::Switcher(switcher::Input::Step(1))),
                        _ => None,
                    })
                } else {
                    iced::Subscription::none()
                },
                #[cfg(not(target_arch = "wasm32"))]
                iced::event::listen_with(|event, status, _| match event {
                    // Page Up and Down scroll the messages, also while typing, as on Discord.
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::PageUp),
                        ..
                    }) => Some(Msg::PageScroll(true)),
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::PageDown),
                        ..
                    }) => Some(Msg::PageScroll(false)),
                    // Escape also closes menus and dialogs while a text field has
                    // focus, as on Discord; the field itself captures the key.
                    // One no field captured may mark the channel read (composer.rs).
                    // Shift+Escape marks the server read (shortcuts.rs).
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        modifiers,
                        ..
                    }) if !modifiers.shift() => Some(if status == iced::event::Status::Ignored {
                        Msg::FreeEscape
                    } else {
                        Msg::Escape
                    }),
                    iced::Event::Window(iced::window::Event::Unfocused) => {
                        Some(Msg::WindowFocus(false))
                    }
                    iced::Event::Window(iced::window::Event::Focused) => {
                        Some(Msg::WindowFocus(true))
                    }
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
                #[cfg(target_os = "linux")]
                hotkeys::subscription(app.push_to_talk(), app.voice.talk_key.clone())
                    .map(Msg::Hotkey),
                // The push-to-talk key while Untethered has focus.
                #[cfg(target_os = "linux")]
                hotkeys::window(
                    app.push_to_talk(),
                    hotkeys::window_key(app.voice.talk_key.as_deref()),
                ),
                #[cfg(target_os = "linux")]
                app.viewer
                    .as_ref()
                    .and_then(|v| v.video.as_ref())
                    .map_or_else(iced::Subscription::none, |p| {
                        p.subscription().map(Msg::Player)
                    }),
                #[cfg(target_os = "linux")]
                app.inline_player
                    .as_ref()
                    .and_then(|p| p.player.as_ref())
                    .map_or_else(iced::Subscription::none, |p| {
                        p.subscription().map(Msg::InlinePlayer)
                    }),
                #[cfg(target_os = "linux")]
                if app.inline_running() {
                    iced::time::every(std::time::Duration::from_millis(250))
                        .map(|_| Msg::InlineTick)
                } else {
                    iced::Subscription::none()
                },
                // The Voice & Video page's meter, mic test, camera
                // preview and key binding (call_settings.rs).
                #[cfg(target_os = "linux")]
                app.voice_subscription(),
                // The QA control map, when a fixture asked for it.
                #[cfg(target_os = "linux")]
                if qa_layout::enabled() {
                    iced::time::every(std::time::Duration::from_millis(300))
                        .map(|_| Msg::QaLayout(None))
                } else {
                    iced::Subscription::none()
                },
                iced::keyboard::listen().filter_map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                        modifiers,
                        ..
                    } => Some(Msg::Tab(modifiers.shift())),
                    // A hovered message, person or channel takes these first
                    // (view/context_menu.rs); otherwise the newest message.
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F10),
                        modifiers,
                        ..
                    } if modifiers.shift() => Some(Msg::ContextMenu(context_menu::Event::Keyboard)),
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ContextMenu),
                        ..
                    } => Some(Msg::ContextMenu(context_menu::Event::Keyboard)),
                    // The media viewer's keys, while no text field has them.
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft),
                        ..
                    } => Some(Msg::ViewerStep(-1)),
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowRight),
                        ..
                    } => Some(Msg::ViewerStep(1)),
                    #[cfg(target_os = "linux")]
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Space),
                        ..
                    } => Some(Msg::PlayerControl(player::Control::Toggle)),
                    // Only keys no widget captured arrive here.
                    #[cfg(target_arch = "wasm32")]
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        modifiers,
                        ..
                    } if !modifiers.shift() => Some(Msg::FreeEscape),
                    #[cfg(target_arch = "wasm32")]
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::PageUp),
                        ..
                    } => Some(Msg::PageScroll(true)),
                    #[cfg(target_arch = "wasm32")]
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::PageDown),
                        ..
                    } => Some(Msg::PageScroll(false)),
                    _ => None,
                }),
            ])
        })
        .run()
}
