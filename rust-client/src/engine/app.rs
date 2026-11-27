//! UI side of the matrix-sdk engine: start it, route the supported requests
//! to typed commands, and apply its events to the same `App` state and
//! `rooms::Rooms` model the gomuks frames fill.
use super::{Account, Command, Connection, Event, Failure, Recovery, run};
use crate::{
    App, Msg, Request, account, community::Step, moderation::Flight,
    permissions::Flight as Flight2, replies::Original, room_state::RoomState, rooms,
};

/// The gomuks path's message for the same failure.
const POSTING_FAILED: &str =
    "Could not change posting permissions. Check your room role and connection.";
use iced::{Task, widget::text_editor};
use serde_json::Value;

impl App {
    pub(crate) fn boot_engine() -> (Self, Task<Msg>) {
        // Transfers (uploads, inline media, downloads) keep the gomuks
        // backend's command shape; the engine answers them. Calls don't run
        // on this engine yet and are ignored there.
        let (transfers, backend) = async_channel::bounded(64);
        let (commands, rx) = async_channel::bounded(64);
        let (tx, events) = async_channel::bounded(64);
        let mut app = Self::new(transfers);
        app.sdk = Some(commands);
        app.status = "Starting the matrix-sdk engine…".into();
        let task = Task::batch([
            Task::perform(run(rx, backend, tx), |result| {
                Msg::Stopped(
                    result
                        .err()
                        .unwrap_or_else(|| "The matrix-sdk engine stopped.".into()),
                )
            }),
            Task::run(events, Msg::Engine),
        ]);
        #[cfg(target_os = "linux")]
        let task = Task::batch([task, crate::wayland_drop::listen()]);
        #[cfg(target_arch = "wasm32")]
        let task = Task::batch([
            task,
            crate::browser_verification::listen(),
            crate::web_files::listen(),
            crate::notifications::browser::listen(),
        ]);
        (app, task)
    }

    pub(crate) fn command(&mut self, make: impl FnOnce(i64) -> Command, request: Request) {
        self.seq += 1;
        if self.connected
            && let Some(sdk) = &self.sdk
            && sdk.try_send(make(self.seq)).is_ok()
        {
            self.pending.insert(self.seq, request);
        } else {
            self.status = "Could not queue the request. Try again.".into();
        }
    }

    /// Requests the App builds as gomuks RPCs, sent to the engine as typed
    /// commands. Until the call sites build commands themselves, this is
    /// the translation between the two.
    pub(crate) fn engine_rpc(&mut self, cmd: &str, data: Value, request: Request) {
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        match &request {
            Request::Reaction(op) => {
                let (room, event, key) = op.parts();
                let (room, event, key) = (room.to_owned(), event.to_owned(), key.to_owned());
                self.command(
                    |id| Command::React {
                        id,
                        room,
                        event,
                        key,
                    },
                    request,
                );
            }
            Request::Edit(room, event) => {
                let (room, event) = (room.clone(), event.clone());
                let body = text(&data["content"]["m.new_content"]["body"]);
                let pills = self
                    .completion
                    .pills(crate::autocomplete::Field::Edit, &event);
                self.command(
                    |id| Command::Edit {
                        id,
                        room,
                        event,
                        body,
                        pills,
                    },
                    request,
                );
            }
            Request::Report => {
                let (room, event, reason) = (
                    text(&data["room_id"]),
                    text(&data["event_id"]),
                    text(&data["reason"]),
                );
                self.command(
                    |id| Command::Report {
                        id,
                        room,
                        event,
                        reason,
                    },
                    request,
                );
            }
            Request::ReadState(room, Some(event)) if cmd == "mark_read" => {
                let (room, event) = (room.clone(), event.clone());
                let public = data["receipt_type"] == "m.read";
                self.command(
                    |id| Command::MarkRead {
                        id,
                        room,
                        event,
                        public,
                    },
                    request,
                );
            }
            Request::ReadState(room, None) => {
                let room = room.clone();
                self.command(|id| Command::MarkUnread { id, room }, request);
            }
            Request::Context(room, event) => {
                let (room, event) = (room.clone(), event.clone());
                self.command(|id| Command::Context { id, room, event }, request);
            }
            Request::ShareReads(public, _) => {
                let public = *public;
                self.command(|id| Command::ShareReads { id, public }, request);
            }
            Request::MediaConfig => self.command(|id| Command::UploadLimit { id }, request),
            Request::Members(room) => {
                let room = room.clone();
                self.command(|id| Command::RoomState { id, room }, request);
            }
            // transfers.rs sends an uploaded attachment's content this way.
            Request::MediaSend(room, _) if cmd == "send_message" => {
                let room = room.clone();
                let content = data["base_content"].clone();
                let reply_to = data["relates_to"]["m.in_reply_to"]["event_id"]
                    .as_str()
                    .map(str::to_owned);
                self.command(
                    |id| Command::SendMedia {
                        id,
                        room,
                        content,
                        reply_to,
                    },
                    request,
                );
            }
            _ => self.engine_unsupported(request),
        }
    }

    /// gomuks RPCs the matrix-sdk engine doesn't implement yet. Background
    /// reads stay quiet; anything the user asked for says why nothing happened.
    pub(crate) fn engine_unsupported(&mut self, request: Request) {
        match request {
            // A retry by transaction ID; this engine never hands one out.
            Request::MediaSend(room, id) => self.fail_file(
                &room,
                id,
                "Transfer failed. Retry or remove the attachment.",
            ),
            Request::Mentions => self.mentions_failed(),
            Request::Search(room, kind) => self.fail_search(&room, kind),
            Request::Setup(step) => self.setup_error(step),
            Request::Report => self.report_result(false),
            _ => {
                self.status = "This isn’t available with the matrix-sdk engine yet.".into();
            }
        }
    }

    pub(crate) fn engine_login(&mut self) {
        if self.logged_in
            || !self.connected
            || self.sdk_ui.browser
            || self.pending.values().any(|p| matches!(p, Request::Login))
        {
            return;
        }
        let (Some(url), false, false) = (
            account::homeserver(&self.server),
            self.username.trim().is_empty(),
            self.password.is_empty(),
        ) else {
            self.status =
                "Enter your homeserver (for example, matrix.org), username, and password.".into();
            return;
        };
        self.server = url;
        self.status = "Signing in…".into();
        let (homeserver, username, password) = (
            self.server.clone(),
            self.username.trim().to_owned(),
            self.password.clone(),
        );
        self.command(
            |id| Command::Login {
                id,
                homeserver,
                username,
                password,
            },
            Request::Login,
        );
    }

    pub(crate) fn engine_verify(&mut self) {
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
        let (format, secret) = account::recovery_input(&self.recovery);
        self.command(
            |id| Command::Recover { id, secret },
            Request::Verify(format),
        );
        if !self.verifying() {
            self.verification_note.clone_from(&self.status);
        }
    }

    pub(crate) fn engine_page(&mut self) {
        let room = self.selected.clone();
        if !self.logged_in
            || !self.verified
            || !self.connected
            || self
                .pending
                .values()
                .any(|p| matches!(p, Request::Page(r) if *r == room))
            || !self.live.list.get(&room).is_some_and(|r| !r.end)
        {
            return;
        }
        let target = room.clone();
        self.command(
            |id| Command::Paginate { id, room: target },
            Request::Page(room),
        );
    }

    /// Enter in the composer while signed in.
    pub(crate) fn engine_compose(&mut self) {
        let room = self.selected.clone();
        if self
            .live
            .list
            .get(&room)
            .is_some_and(|r| r.posting.can_post(&self.matrix_id, r.encrypted) == Some(false))
        {
            self.status = "This room is read-only for your account.".into();
            return;
        }
        if self
            .live
            .list
            .get(&room)
            .is_some_and(|r| r.successor.is_some())
        {
            self.status = "This room was upgraded. Join the new room to keep chatting.".into();
            return;
        }
        // Do not send just the caption if Enter races PNG encoding.
        #[cfg(not(target_arch = "wasm32"))]
        if self.pasting {
            return;
        }
        let body = self.editor.text();
        // Staged files go out with the text as their caption (transfers.rs).
        if self.outbox.rooms.get(&room).is_some_and(|q| !q.is_empty()) {
            self.send_files(room, body);
            return;
        }
        if room.is_empty()
            || self.outbox.skipped.contains_key(&room)
            || body.trim().is_empty()
            || !self.verified
            || self
                .pending
                .values()
                .any(|p| matches!(p, Request::Send(r, ..) if *r == room))
        {
            return;
        }
        let reply = self.reply_drafts.get(&room).cloned();
        let (target, text, reply_to) = (
            room.clone(),
            body.clone(),
            reply.as_ref().map(|q| q.id.clone()),
        );
        let pills = self
            .completion
            .pills(crate::autocomplete::Field::Composer, &room);
        self.command(
            |id| Command::Send {
                id,
                pills,
                room: target,
                body: text,
                reply_to,
            },
            Request::Send(room, body, reply),
        );
    }

    pub(crate) fn engine_event(&mut self, event: Event) {
        match event {
            Event::Account(a) => self.engine_account(a),
            Event::ProfileSaved { id, result } => self.profile_saved(id, result),
            Event::ProfileDirect { id, result } => self.profile_direct(id, result),
            Event::Rooms(list) => {
                for left in self.live.apply_engine(list) {
                    self.reply_drafts.remove(&left);
                    if self.drafts.remove(&left).is_some() {
                        self.drafts_changed();
                    }
                    self.loaded.remove(&left);
                }
                if self.logged_in {
                    self.reconcile();
                }
                #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
                self.sync_space_rules();
                // A room community setup created opens once it's listed.
                self.open_created();
                self.open_profile_dm();
            }
            Event::Timeline(t) => {
                if let Some(r) = self.live.list.get_mut(&t.room) {
                    r.apply_engine(t.messages, t.start_reached);
                    r.read.set_latest(t.latest_event);
                    r.seams = t.seams;
                }
            }
            Event::Done { id, result } => self.engine_done(id, result),
            Event::Connection(connection) => {
                const OFFLINE: &str = "Offline. Untethered will reconnect automatically.";
                const RETRYING: &str = "Sync stopped after an error. Retrying…";
                // Who is typing is unknown until the sync is back.
                if connection != Connection::Online {
                    self.typing.forget();
                }
                match connection {
                    Connection::Offline => self.status = OFFLINE.into(),
                    Connection::Retrying => self.status = RETRYING.into(),
                    // Clear only our own notice, not an unrelated message.
                    Connection::Online if self.status == OFFLINE || self.status == RETRYING => {
                        self.status.clear()
                    }
                    Connection::Online => {}
                }
            }
            Event::Status(text) => {
                if self.connected {
                    self.status = text;
                }
            }
            Event::Verification(view) => self.sdk_verification(view),
            Event::RecoveryKey(key) => {
                self.sdk_ui.recovery_key = Some(key);
                self.sdk_ui.key_copied = false;
            }
            Event::Devices(list) => self.sdk_ui.devices = Some(list),
            Event::LoginOptions(options) => self.sdk_login_options(options),
            Event::Context {
                room,
                event,
                messages,
            } => self.engine_context(room, event, messages),
            Event::ReadSharing(public) => self.read_sharing.set_from_engine(public),
            Event::Typing { room, users } => self.typing_seen(room, users),
            Event::TypingSharing(send) => self.typing_shared(send),
            Event::UploadProgress { id, fraction } => self.upload_progress(id, fraction),
            Event::Uploaded { id, result } => {
                // Absent after a cancel: that attempt must never send.
                if let Some(Request::Upload(room, attachment)) = self.pending.remove(&id) {
                    match result {
                        Ok(content) => self.uploaded(&room, attachment, &content),
                        Err(_) => self.fail_file(
                            &room,
                            attachment,
                            "Transfer failed. Retry or remove the attachment.",
                        ),
                    }
                }
            }
            Event::RoomState { id, result } => self.engine_room_state(id, result),
            Event::RtcTransports { id, result } => {
                if matches!(self.pending.remove(&id), Some(Request::RtcDiscovery)) {
                    match result {
                        Ok(found) => self.rtc_discovered(&found),
                        Err(_) => {
                            self.rtc_status =
                                "MatrixRTC discovery failed or is unsupported on this homeserver."
                                    .into()
                        }
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Event::CallFrame(frame) => self.frame(frame),
            #[cfg(target_os = "linux")]
            Event::CallActivity { room, event } => self.call_activity(&room, &event),
            #[cfg(target_os = "linux")]
            Event::CallState { id, result } => {
                if let Some(Request::RingState(offer)) = self.pending.remove(&id) {
                    let state = result.map_or(Value::Null, Value::Array);
                    self.present_ring(offer, &state);
                }
            }
            Event::Member { room, user, member } => {
                // Someone who joined or changed their name since the room
                // loaded: the timeline and the typing line use it before
                // their next message brings it.
                if matches!(member.kind, crate::moderation::Kind::Joined)
                    && let Some(r) = self.live.list.get_mut(&room)
                {
                    r.update_member_profile(&user, &member);
                }
                if self.people && self.selected == room {
                    self.directory.observe(&user, member);
                }
            }
            Event::SearchResults { id, result } => match (self.pending.remove(&id), result) {
                (Some(Request::Search(room, _)), Ok((found, next))) => {
                    self.search_found(room, found, next)
                }
                (Some(Request::Search(room, kind)), Err(_)) => self.fail_search(&room, kind),
                _ => {}
            },
            Event::SearchCoverage { id, result } => match (self.pending.remove(&id), result) {
                (Some(Request::Search(room, _)), Ok((messages, oldest))) => {
                    self.search_counted(room, messages, oldest)
                }
                (Some(Request::Search(room, kind)), Err(_)) => self.fail_search(&room, kind),
                _ => {}
            },
            Event::Mentions { id, result } => match (self.pending.remove(&id), result) {
                (Some(Request::Mentions), Ok(found)) => self.mentions_found(found),
                (Some(Request::Mentions), Err(_)) => self.mentions_failed(),
                _ => {}
            },
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Event::PushRules(content) => self.engine_push_rules(&content),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            Event::Notification(notice) => self.engine_notice(notice),
            #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
            Event::PushRules(_) | Event::Notification(_) => {}
            Event::Created { id, result } => self.engine_created(id, result),
            Event::UploadLimit { id, result } => {
                if matches!(self.pending.remove(&id), Some(Request::MediaConfig))
                    && let Ok(limit) = result
                    && limit > 0
                {
                    self.media_limit = crate::media::MAX_BYTES.min(limit);
                    self.outbox.apply_limit(self.media_limit);
                }
            }
        }
    }

    fn engine_account(&mut self, a: Account) {
        // A state report without a display name keeps the known one.
        let profile = a.display_name.clone().unwrap_or_else(|| {
            if a.user_id == self.matrix_id && !self.profile.is_empty() {
                self.profile.clone()
            } else {
                a.user_id.clone()
            }
        });
        // Like the name, a report without a picture keeps the known one.
        if a.avatar.is_some() || !a.logged_in {
            self.own_avatar.clone_from(&a.avatar);
        }
        let unavailable = a.logged_in && a.recovery == Recovery::Disabled;
        let was_verified = self.verified;
        if !a.logged_in && self.logged_in {
            self.sdk_ui = Default::default();
        }
        self.sdk_ui.identity = a.identity;
        self.sdk_ui.recovery = a.recovery;
        self.apply_account(account::State {
            logged_in: a.logged_in,
            verified: a.logged_in && a.verified,
            recovery_unavailable: unavailable,
            profile,
            user_id: a.user_id,
        });
        if unavailable && !self.verified {
            self.verification_note = if a.identity {
                "Recovery isn’t set up for this account. Verify with another device below, or reset encryption."
            } else {
                "This account doesn’t have encryption set up yet. Set up recovery below."
            }
            .into();
        }
        if self.verified && !was_verified {
            let _ = self.sdk_action(super::account_ui::Action::RefreshDevices);
        }
    }

    fn engine_done(&mut self, id: i64, result: Result<(), Failure>) {
        let Some(request) = self.pending.remove(&id) else {
            self.sdk_done(id, result);
            return;
        };
        match (request, result) {
            (Request::Login, Ok(())) => self.password.clear(),
            (Request::Login, Err(failure)) => {
                self.status = match failure {
                    Failure::Unreachable => {
                        "No Matrix homeserver answered at that address. Check it and try again."
                    }
                    Failure::NoSlidingSync => {
                        "This homeserver doesn’t offer Simplified Sliding Sync, which the matrix-sdk engine needs. Start without UNTETHERED_ENGINE to use gomuks."
                    }
                    _ => "Sign-in failed. Check your homeserver, username, and password.",
                }
                .into()
            }
            (Request::Verify(_), Ok(())) => {
                if !self.verified {
                    self.verification_note =
                        "Recovery finished, but this device isn’t cross-signed yet. You can retry."
                            .into();
                }
            }
            (Request::Verify(format), Err(failure)) => {
                self.verification_note = recovery_error(failure, format).into();
            }
            (Request::Page(_), Ok(())) => {}
            (Request::Page(room), Err(_)) => {
                self.status = "Could not load history. Use Earlier messages to retry.".into();
                self.auto_page_failed = Some(room);
            }
            (Request::Send(room, sent, reply), Ok(())) => {
                // Same draft handling as the gomuks send_message response.
                let same = self.reply_drafts.get(&room) == reply.as_ref();
                if same && self.selected == room && self.editor.text() == sent {
                    self.editor = text_editor::Content::new();
                }
                if same && self.drafts.get(&room) == Some(&sent) {
                    self.drafts.remove(&room);
                    self.drafts_changed();
                }
                if same {
                    self.reply_drafts.remove(&room);
                }
                self.status.clear();
            }
            (Request::Send(..), Err(_)) => {
                self.status = "Could not send. Your draft has been kept.".into();
            }
            // The timeline snapshot carries the new chips and text.
            (Request::Reaction(_), Ok(())) => {}
            (Request::Reaction(_), Err(_)) => {
                self.status = "Could not change this reaction. Try again.".into();
            }
            (Request::Edit(..), Ok(())) => {
                if let Some(d) = &self.editing
                    && d.pending == Some(id)
                {
                    self.editing = None;
                    self.status.clear();
                }
            }
            (Request::Edit(room, event), Err(_)) => {
                self.finish_edit(id, &room, &event, &Value::Null)
            }
            (Request::Redact(..), Ok(())) => {
                self.status.clear();
            }
            (Request::Redact(..), Err(_)) => {
                self.status = "Could not delete this message. Try again.".into();
            }
            (Request::Report, result) => self.report_result(result.is_ok()),
            (Request::ReadState(room, event), result) => {
                self.read_result(room, event, result.is_ok())
            }
            (Request::LeaveRoom(room), result) => self.left_room(&room, result.is_ok()),
            (Request::FollowUpgrade(_), result) => self.followed(result.is_ok()),
            (Request::Context(..), Ok(())) => {}
            (Request::Context(..), Err(_)) => {
                self.status =
                    "Could not load the selected message. Try opening it again.".into();
            }
            (Request::ShareReads(public, revision), Ok(())) => {
                self.read_sharing.saved(public, revision)
            }
            (Request::ShareReads(..), Err(_)) => {
                self.status =
                    "Could not save your read receipt preference. Try again.".into();
            }
            (Request::ShareTyping(send, revision), result) => {
                self.typing_saved(send, revision, result.is_ok())
            }
            (Request::MediaSend(room, attachment), Ok(())) => {
                self.finish_delivery(&room, attachment, true)
            }
            (Request::MediaSend(room, attachment), Err(_)) => self.fail_file(
                &room,
                attachment,
                "Transfer failed. Retry or remove the attachment.",
            ),
            (Request::Membership(Flight::Write(c)), Ok(())) => self.membership_written(c),
            (Request::Membership(f), Err(_)) => self.membership_error(f),
            (Request::Setup(Step::Invite), Ok(())) => self.setup_invited(),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::Dnd(on, revision), Ok(())) => self.saved_dnd(on, revision),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::Dnd(..), Err(_)) => {
                self.status =
                    "Could not change Do Not Disturb. Check the connection and try again.".into()
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::NotifyRule(change), Ok(())) => self.saved_notifications(change),
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::NotifyRule(_), Err(_)) => {
                self.status = "Could not save the room notification setting. Try again.".into()
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::SpaceRule(write), result) => {
                self.finish_space_rule(write, result.is_ok());
                self.sync_space_rules();
            }
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::SpacePref(_), Ok(())) => {}
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            (Request::SpacePref(_), Err(_)) => {
                self.status = "Could not save space alerts. Try again.".into()
            }
            (Request::Setup(step), Err(_)) => self.setup_error(step),
            #[cfg(target_os = "linux")]
            (Request::DeclineSent, Ok(())) => self.status = "Call declined.".into(),
            #[cfg(target_os = "linux")]
            (Request::DeclineSent, Err(_)) => {
                self.status = "Could not notify the caller that you declined.".into()
            }
            (Request::Download, Ok(())) => self.status = "Attachment saved.".into(),
            (Request::Download, Err(_)) => {
                self.status = "Could not save the attachment.".into();
            }
            _ => {}
        }
    }
}

impl App {
    fn engine_room_state(&mut self, id: i64, result: Result<RoomState, Failure>) {
        let Some(request) = self.pending.remove(&id) else {
            return;
        };
        match (request, result) {
            (Request::Membership(f), Ok(state)) => self.membership_state(f, state),
            (Request::Membership(f), Err(_)) => self.membership_error(f),
            (Request::Posting(Flight2::Check(c)), Ok(state)) => self.posting_checked(c, state),
            (Request::Posting(_), Err(_)) => self.status = POSTING_FAILED.into(),
            (Request::Setup(Step::Check), Ok(state)) => self.setup_checked(state),
            (Request::Setup(step), Err(_)) => self.setup_error(step),
            (Request::Members(room), Ok(state)) => self.members_loaded(&room, state),
            _ => {}
        }
    }

    /// Display names of a room's members, read when it opens as gomuks's room
    /// state was: people who haven't written yet get their names too, for
    /// example in a voice room's participant list.
    fn members_loaded(&mut self, room: &str, state: RoomState) {
        self.completion.learn_roster(room, &state.members);
        if let Some(r) = self.live.list.get_mut(room) {
            for (user, member) in &state.members {
                if member.kind == crate::moderation::Kind::Joined
                    && !r.profile_members.contains(user)
                {
                    r.update_member_profile(user, member);
                }
            }
        }
    }

    /// Answers with the ID of a new room or event.
    fn engine_created(&mut self, id: i64, result: Result<String, Failure>) {
        let Some(request) = self.pending.remove(&id) else {
            return;
        };
        match (request, result) {
            (Request::Posting(Flight2::Write(c, content)), Ok(event)) => {
                self.posting_written(c, content, event)
            }
            (Request::Posting(_), Err(_)) => self.status = POSTING_FAILED.into(),
            (Request::Setup(Step::Create), Ok(room)) => self.setup_created(room),
            (Request::Setup(Step::Link), Ok(event)) => self.setup_linked(&event),
            (Request::Setup(step), Err(_)) => self.setup_error(step),
            _ => {}
        }
    }

    /// Messages around a reply's original, for "View original" when it is
    /// outside the loaded timeline.
    fn engine_context(&mut self, room: String, event: String, messages: Vec<super::Message>) {
        if self.selected != room {
            return;
        }
        let mut slice = rooms::Room::default();
        slice.apply_engine(messages, false);
        if slice.by_id(&event).is_none() {
            self.status = "The original message is unavailable in this room.".into();
            return;
        }
        self.status.clear();
        self.original = Some(Original {
            room,
            event,
            slice: Some(slice),
            preceding: false,
        });
    }
}

fn recovery_error(failure: Failure, format: account::RecoveryFormat) -> &'static str {
    use account::RecoveryFormat;
    match (failure, format) {
        (Failure::WrongSecret, RecoveryFormat::Key) => {
            "The recovery key’s format and checksum are valid, but it didn’t unlock this account’s secret storage. Check the Matrix ID above and test the same key in another client."
        }
        (Failure::WrongSecret, RecoveryFormat::FreeText) => {
            "This entry isn’t a checksum-valid Matrix recovery key, and it didn’t unlock recovery as a passphrase. Use the full recovery key or the passphrase chosen when recovery was set up, not your sign-in password."
        }
        (Failure::NoRecovery, _) => {
            "This account’s secret storage is missing on the homeserver. Check recovery in your trusted Matrix client."
        }
        (Failure::Unreachable, _) => {
            "Couldn’t reach the homeserver to finish verification. Try again when it’s reachable."
        }
        _ => {
            "This device could not be verified. Check that this is the current key or recovery passphrase for this account."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Message, RoomList, RoomSummary, TimelineSnapshot};

    fn app() -> (App, async_channel::Receiver<Command>) {
        let (gomuks, _) = async_channel::bounded(1);
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(gomuks);
        app.sdk = Some(tx);
        (app, rx)
    }
    fn signed_in(verified: bool) -> Event {
        Event::Account(Account {
            logged_in: true,
            user_id: "@me:example".into(),
            display_name: Some("Me".into()),
            avatar: None,
            verified,
            identity: true,
            recovery: Recovery::Enabled,
        })
    }

    #[test]
    fn login_and_recovery_use_typed_commands_and_pending_requests() {
        let (mut app, rx) = app();
        app.engine_event(Event::Account(Account::default()));
        assert!(app.connected && !app.logged_in);
        app.server = "127.0.0.1:8008".into();
        app.username = " me ".into();
        app.password = "secret".into();
        let _ = app.update(Msg::Login);
        let Ok(Command::Login {
            id,
            homeserver,
            username,
            ..
        }) = rx.try_recv()
        else {
            panic!("expected a login command")
        };
        assert_eq!(
            (homeserver.as_str(), username.as_str()),
            ("https://127.0.0.1:8008/", "me")
        );
        assert!(app.pending.values().any(|p| matches!(p, Request::Login)));
        let _ = app.update(Msg::Login);
        assert!(
            rx.try_recv().is_err(),
            "no duplicate login while one is pending"
        );
        app.engine_event(signed_in(false));
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert!(app.logged_in && !app.verified && app.password.is_empty());

        app.recovery = "not a key".into();
        let _ = app.update(Msg::Verify);
        let Ok(Command::Recover { id, secret }) = rx.try_recv() else {
            panic!("expected a recovery command")
        };
        assert_eq!(secret, "not a key");
        assert!(app.verifying());
        app.engine_event(Event::Done {
            id,
            result: Err(Failure::WrongSecret),
        });
        assert!(!app.verifying());
        assert!(app.verification_feedback().contains("as a passphrase"));
        app.engine_event(signed_in(true));
        assert!(app.verified && app.recovery.is_empty());
    }

    /// After a fresh sign-in the room list arrives before the recovery key
    /// verifies the device; the room selected then loads once it is verified.
    #[test]
    fn room_selected_before_verification_loads_after_it() {
        let (mut app, rx) = app();
        app.engine_event(signed_in(false));
        app.engine_event(Event::Rooms(RoomList {
            rooms: vec![RoomSummary {
                id: "!r:example".into(),
                name: "general".into(),
                encrypted: true,
                ..Default::default()
            }],
            children: Default::default(),
        }));
        assert_eq!(app.selected, "!r:example");
        let early: Vec<Command> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(!early.iter().any(|c| matches!(c, Command::Paginate { .. })));
        app.engine_event(signed_in(true));
        let later: Vec<Command> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            later
                .iter()
                .any(|c| matches!(c, Command::Paginate { room, .. } if room == "!r:example")),
            "the selected room's history was requested"
        );
    }

    fn message(row: i64) -> Message {
        Message {
            row,
            event_id: Some(format!("${row}")),
            sender: "@you:example".into(),
            sender_name: None,
            sender_avatar: None,
            time: row,
            body: format!("message {row}"),
            formatted: None,
            kind: crate::engine::Kind::Text,
            encrypted: false,
            delivery: crate::engine::Delivery::Sent,
            edited: false,
            reply_to: None,
            reactions: Vec::new(),
            text: true,
            editable: false,
            mentions: None,
            edit_id: None,
            edit_time: None,
            receipts: Vec::new(),
            media: None,
            from: None,
        }
    }

    /// Leaving a room closes its timeline and keeps its newest posts; coming
    /// back opens it again. Back at the newest message with a long history
    /// loaded, the open room starts over the same way (paging.rs).
    #[test]
    fn a_room_left_lets_its_history_go_and_loads_it_again_on_return() {
        let (mut app, rx) = app();
        app.engine_event(signed_in(true));
        let room = |id: &str| RoomSummary {
            id: id.into(),
            name: id.into(),
            ..Default::default()
        };
        app.engine_event(Event::Rooms(RoomList {
            rooms: vec![room("!a:example"), room("!b:example")],
            children: Default::default(),
        }));
        let _ = app.update(Msg::Open("!a:example".into()));
        let drain = |rx: &async_channel::Receiver<Command>| -> Vec<Command> {
            std::iter::from_fn(|| rx.try_recv().ok()).collect()
        };
        drain(&rx);
        app.pending.clear();
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: "!a:example".into(),
            messages: (1..=500).map(message).collect(),
            start_reached: false,
            latest_event: None,
            seams: Vec::new(),
        }));
        // Scrolled up, nothing goes.
        app.scrolled_away = true;
        let _ = app.bound_history();
        assert_eq!(app.live.list["!a:example"].posts().len(), 500);
        assert!(drain(&rx).is_empty());
        // Back at the newest: the room starts over from its newest posts,
        // opening its timeline again after a pause.
        app.scrolled_away = false;
        let _ = app.bound_history();
        let commands = drain(&rx);
        assert!(matches!(&commands[..], [Command::CloseTimeline { room }] if room == "!a:example"));
        let a = &app.live.list["!a:example"];
        assert_eq!(a.posts().len(), 60);
        assert_eq!(a.posts().last().unwrap().body, "message 500");
        assert!(!a.end);
        let _ = app.bound_history();
        assert!(drain(&rx).is_empty(), "nothing before the pause is over");
        let _ = app.update(Msg::ReopenTimeline("!a:example".into()));
        assert!(
            matches!(&drain(&rx)[..], [Command::Paginate { room, .. }] if room == "!a:example")
        );
        app.pending.clear();
        // Opening with everything again doesn't start it over again.
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: "!a:example".into(),
            messages: (1..=500).map(message).collect(),
            start_reached: false,
            latest_event: None,
            seams: Vec::new(),
        }));
        let _ = app.bound_history();
        assert!(
            drain(&rx).is_empty(),
            "no second start-over without scrolling away"
        );
        // Leaving closes it; returning opens it again.
        let _ = app.update(Msg::Open("!b:example".into()));
        assert!(
            drain(&rx)
                .iter()
                .any(|c| matches!(c, Command::CloseTimeline { room } if room == "!a:example"))
        );
        app.pending.clear();
        let _ = app.update(Msg::Open("!a:example".into()));
        assert!(
            drain(&rx)
                .iter()
                .any(|c| matches!(c, Command::Paginate { room, .. } if room == "!a:example"))
        );
        assert!(!app.released.contains("!a:example"));
    }

    /// A restored session's account data comes from the engine's store before
    /// the first account state, which waits on the homeserver.
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    #[test]
    fn account_data_before_the_restored_account_state_stays() {
        let (mut app, _rx) = app();
        app.engine_event(Event::ReadSharing(true));
        app.engine_event(Event::PushRules(serde_json::json!({"global": {
            "override": [], "content": [], "room": [], "sender": [], "underride": []}})));
        app.engine_event(signed_in(false));
        app.engine_event(signed_in(true));
        assert!(app.alerts.ready, "the push rules survived");
        assert!(
            app.read_sharing.public,
            "the read receipt preference survived"
        );
        app.engine_event(Event::Account(Account::default()));
        assert!(
            !app.alerts.ready && !app.read_sharing.public,
            "signing out forgets them"
        );
    }

    #[test]
    fn engine_notices_explain_and_clear_only_themselves() {
        let (mut app, rx) = app();
        app.engine_event(Event::Account(Account::default()));
        app.server = "http://127.0.0.1:8008".into();
        app.username = "me".into();
        app.password = "secret".into();
        let _ = app.update(Msg::Login);
        let Ok(Command::Login { id, .. }) = rx.try_recv() else {
            panic!("expected a login command")
        };
        app.engine_event(Event::Done {
            id,
            result: Err(Failure::NoSlidingSync),
        });
        assert!(app.status.contains("Simplified Sliding Sync") && !app.logged_in);

        app.engine_event(signed_in(true));
        app.engine_event(Event::Connection(Connection::Offline));
        assert!(app.status.starts_with("Offline"));
        app.engine_event(Event::Connection(Connection::Online));
        assert!(app.status.is_empty());
        app.status = "Could not send. Your draft has been kept.".into();
        app.engine_event(Event::Connection(Connection::Online));
        assert!(app.status.starts_with("Could not send"));
    }

    #[test]
    fn rooms_timeline_paging_and_send_round_trip() {
        let (mut app, rx) = app();
        app.engine_event(signed_in(true));
        // A verified device loads its device list for the account page, and
        // the upload size limit.
        let sent: Vec<Command> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(sent.iter().any(|c| matches!(c, Command::Devices { .. })));
        let Some(Command::UploadLimit { id: limit }) = sent
            .into_iter()
            .find(|c| matches!(c, Command::UploadLimit { .. }))
        else {
            panic!("expected an upload limit request")
        };
        app.engine_event(Event::UploadLimit {
            id: limit,
            result: Ok(1024),
        });
        assert_eq!(app.media_limit, 1024);
        app.engine_event(Event::Rooms(RoomList {
            rooms: vec![RoomSummary {
                id: "!r:example".into(),
                name: "general".into(),
                encrypted: true,
                ..Default::default()
            }],
            children: Default::default(),
        }));
        // reconcile() opens the only room and asks for its history.
        assert_eq!(app.selected, "!r:example");
        let Ok(Command::Paginate { id, room }) = rx.try_recv() else {
            panic!("expected pagination")
        };
        assert_eq!(room, "!r:example");
        // ...and its members, so people who haven't written yet have names.
        let Ok(Command::RoomState {
            id: members,
            room: asked,
        }) = rx.try_recv()
        else {
            panic!("expected the room's members")
        };
        assert_eq!(asked, room);
        let mut state = RoomState::default();
        state.members.insert(
            "@quiet:example".into(),
            crate::moderation::Member {
                kind: crate::moderation::Kind::Joined,
                name: "Quiet One".into(),
                avatar: None,
                event: "$member".into(),
            },
        );
        app.engine_event(Event::RoomState {
            id: members,
            result: Ok(state),
        });
        assert_eq!(app.live.list[&room].members["@quiet:example"], "Quiet One");
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: room.clone(),
            start_reached: true,
            messages: vec![Message {
                row: 1,
                event_id: Some("$1".into()),
                sender: "@you:example".into(),
                sender_name: Some("You".into()),
                sender_avatar: None,
                time: 1,
                body: "hello".into(),
                formatted: None,
                kind: crate::engine::Kind::Text,
                encrypted: true,
                delivery: crate::engine::Delivery::Sent,
                edited: false,
                reply_to: None,
                reactions: vec![("👍".into(), 1, false)],
                text: true,
                editable: false,
                mentions: None,
                edit_id: None,
                edit_time: None,
                receipts: vec![("@you:example".into(), 1)],
                media: None,
                from: None,
            }],
            // An edit of "hello" came last; it has no message of its own.
            latest_event: Some("$edit".into()),
            seams: Vec::new(),
        }));
        app.engine_event(Event::Done { id, result: Ok(()) });
        let r = &app.live.list[&room];
        assert!(r.end && r.posts().next().unwrap().body == "hello");
        assert_eq!(
            r.read.target(),
            Some("$edit"),
            "reading marks the edit read"
        );
        let post = r.posts().next().unwrap();
        assert!(post.reactable && post.chips["👍"].count == 1 && post.chips["👍"].mine().is_none());
        assert!(
            !post.may_edit("@me:example"),
            "only our own messages are editable"
        );
        assert_eq!(post.report_id("@me:example"), Some("$1"));

        app.editor = text_editor::Content::with_text("hi there");
        let _ = app.update(Msg::Reply("$1".into()));
        let _ = app.update(Msg::Send);
        let Ok(Command::Send {
            id, body, reply_to, ..
        }) = rx.try_recv()
        else {
            panic!("expected a send command")
        };
        assert_eq!(
            (body.as_str(), reply_to.as_deref()),
            ("hi there", Some("$1"))
        );
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert!(app.editor.text().trim().is_empty() && app.reply_drafts.is_empty());

        // Step 3: the existing controls reach typed commands.
        let op = crate::reactions::Toggle::for_post(&room, app.visible_post("$1").unwrap(), "👍")
            .unwrap();
        app.toggle_reaction(op);
        let Ok(Command::React { event, key, .. }) = rx.try_recv() else {
            panic!("expected a reaction")
        };
        assert_eq!((event.as_str(), key.as_str()), ("$1", "👍"));
        assert!(app.begin_report(&room, "$1"));
        let _ = app.update(Msg::ReportReason("spam".into()));
        app.submit_report();
        let Ok(Command::Report { event, reason, .. }) = rx.try_recv() else {
            panic!("expected a report")
        };
        assert_eq!((event.as_str(), reason.as_str()), ("$1", "spam"));
        app.engine_event(Event::Context {
            room: room.clone(),
            event: "$0".into(),
            messages: vec![Message {
                row: 1,
                event_id: Some("$0".into()),
                sender: "@you:example".into(),
                sender_name: None,
                sender_avatar: None,
                time: 0,
                body: "original".into(),
                formatted: None,
                kind: crate::engine::Kind::Text,
                encrypted: true,
                delivery: crate::engine::Delivery::Sent,
                edited: false,
                reply_to: None,
                reactions: vec![],
                text: true,
                editable: false,
                mentions: None,
                edit_id: None,
                edit_time: None,
                receipts: vec![],
                media: None,
                from: None,
            }],
        });
        let original = app.original.as_ref().expect("the original opens");
        assert_eq!(
            original.slice.as_ref().unwrap().by_id("$0").unwrap().body,
            "original"
        );

        // Call server discovery is a typed request.
        let _ = app.update(Msg::CheckCalls);
        let Ok(Command::RtcTransports { id }) = rx.try_recv() else {
            panic!("expected RTC discovery")
        };
        app.engine_event(Event::RtcTransports {
            id,
            result: Ok(vec![serde_json::json!({"type":"livekit",
                "livekit_service_url":"https://rtc.example"})]),
        });
        assert_eq!(app.rtc_status, "Found 1 LiveKit call transport(s).");
    }

    #[test]
    fn attachments_upload_then_send_and_a_cancelled_upload_never_sends() {
        let (transfers, backend) = async_channel::bounded(8);
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(transfers);
        app.sdk = Some(tx);
        app.engine_event(signed_in(true));
        // Files wait for the upload limit.
        while let Ok(command) = rx.try_recv() {
            if let Command::UploadLimit { id } = command {
                app.engine_event(Event::UploadLimit {
                    id,
                    result: Ok(1 << 20),
                });
            }
        }
        app.engine_event(Event::Rooms(RoomList {
            rooms: vec![RoomSummary {
                id: "!r:example".into(),
                name: "general".into(),
                encrypted: true,
                ..Default::default()
            }],
            children: Default::default(),
        }));
        while rx.try_recv().is_ok() {}
        let file = |name: &str| crate::media::LocalFile {
            name: name.into(),
            size: 3,
            origin: crate::media::NativeSource::Clipboard(b"abc".to_vec().into()),
        };
        let _ = app.update(Msg::Files(
            "!r:example".into(),
            vec![file("a.png"), file("b.txt")],
        ));
        app.editor = text_editor::Content::with_text("look");
        let _ = app.update(Msg::Send);
        let Ok(crate::backend::Command::Upload {
            id,
            encrypt,
            cancel,
            ..
        }) = backend.try_recv()
        else {
            panic!("expected an upload")
        };
        assert!(encrypt);
        app.engine_event(Event::UploadProgress { id, fraction: 0.5 });
        assert_eq!(
            app.outbox.rooms["!r:example"][0].status_label(),
            "Upload 50%"
        );
        let content = serde_json::json!({"msgtype": "m.image", "body": "a.png",
            "file": {"url": "mxc://example/a"}, "info": {"size": 3}});
        app.engine_event(Event::Uploaded {
            id,
            result: Ok(content),
        });
        let Ok(Command::SendMedia {
            id: send,
            room,
            content,
            reply_to,
        }) = rx.try_recv()
        else {
            panic!("expected the attachment message")
        };
        assert_eq!(room, "!r:example");
        assert_eq!(reply_to, None);
        assert_eq!(
            (&content["body"], &content["filename"]),
            (&serde_json::json!("look"), &serde_json::json!("a.png")),
            "the caption goes with the first file"
        );
        drop(cancel);
        app.engine_event(Event::Done {
            id: send,
            result: Ok(()),
        });
        assert_eq!(app.outbox.rooms["!r:example"].len(), 1);
        assert!(app.editor.text().trim().is_empty());

        // The second file: cancelled mid-upload, so a late result is ignored.
        let Ok(crate::backend::Command::Upload { id, cancel, .. }) = backend.try_recv() else {
            panic!("expected the second upload")
        };
        let attachment = app.outbox.rooms["!r:example"][0].id;
        app.cancel_upload("!r:example", attachment, id);
        assert!(cancel.is_closed(), "the engine sees the cancel");
        app.engine_event(Event::Uploaded {
            id,
            result: Ok(serde_json::json!({"msgtype": "m.file", "body": "b.txt",
                "file": {"url": "mxc://example/b"}})),
        });
        assert!(rx.try_recv().is_err(), "a cancelled upload never sends");
        assert_eq!(
            app.outbox.rooms["!r:example"][0].status_label(),
            "Cancelled · retry"
        );
    }

    /// Signed in as an admin of one room, with its state answered by `state`.
    fn admin() -> (App, async_channel::Receiver<Command>) {
        let (mut app, rx) = app();
        app.engine_event(signed_in(true));
        app.engine_event(Event::Rooms(RoomList {
            rooms: vec![RoomSummary {
                id: "!r:example".into(),
                name: "general".into(),
                ..Default::default()
            }],
            children: Default::default(),
        }));
        while rx.try_recv().is_ok() {}
        (app, rx)
    }
    fn state(power: &str, bob: &str) -> crate::room_state::RoomState {
        use crate::{moderation::Member, permissions::StateDoc};
        let doc = |id: &str, content| StateDoc {
            event_id: id.into(),
            sender: "@me:example".into(),
            content,
            redacted: false,
        };
        let member = |event: &str| Member {
            kind: crate::moderation::Kind::Joined,
            name: String::new(),
            avatar: None,
            event: event.into(),
        };
        crate::room_state::RoomState {
            create: Some(doc("$create", serde_json::json!({"room_version": "11"}))),
            power: Some(doc(
                power,
                serde_json::json!({"users": {"@me:example": 100}, "kick": 50, "ban": 50}),
            )),
            members: [
                ("@me:example".to_owned(), member("$me")),
                ("@bob:example".to_owned(), member(bob)),
            ]
            .into(),
        }
    }

    #[test]
    fn moderation_rechecks_typed_room_state_before_acting() {
        let (mut app, rx) = admin();
        app.people = true;
        app.read_directory(None);
        let Ok(Command::RoomState { id, room }) = rx.try_recv() else {
            panic!("expected a room state read")
        };
        assert_eq!(room, "!r:example");
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$pl", "$bob")),
        });
        assert_eq!(app.directory.users.len(), 2);
        app.directory.choose(Some("@bob:example".into()));
        let change = app
            .member_change(crate::moderation::Action::Kick)
            .expect("an admin may kick bob");
        app.moderate(change);
        let Ok(Command::RoomState { id, .. }) = rx.try_recv() else {
            panic!("expected a fresh read before acting")
        };
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$pl", "$bob")),
        });
        let Ok(Command::Moderate {
            id, user, action, ..
        }) = rx.try_recv()
        else {
            panic!("expected the kick")
        };
        assert_eq!(user, "@bob:example");
        assert_eq!(action, crate::moderation::Action::Kick);
        app.engine_event(Event::Done { id, result: Ok(()) });
        assert_eq!(app.directory.note, "Kick accepted by homeserver.");
        // Until sync delivers the kick, bob's member event is unchanged, so
        // the directory reads the state again.
        let Ok(Command::RoomState { id, .. }) = rx.try_recv() else {
            panic!("expected a re-read")
        };
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$pl", "$bob")),
        });

        // A change that went stale in the meantime isn't sent.
        app.directory.choose(Some("@bob:example".into()));
        let change = app.member_change(crate::moderation::Action::Ban).unwrap();
        app.moderate(change);
        let Ok(Command::RoomState { id, .. }) = rx.try_recv() else {
            panic!("expected a fresh read")
        };
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$pl", "$bob-rejoined")),
        });
        assert!(rx.try_recv().is_err(), "a stale change is not sent");
        assert!(
            app.directory
                .note
                .starts_with("Membership or permissions changed")
        );
    }

    #[test]
    fn posting_changes_write_the_patched_document_and_keep_its_event() {
        let (mut app, rx) = admin();
        app.permission_panel = true;
        let summary = state("$pl", "$bob");
        app.live.list.get_mut("!r:example").unwrap().posting = summary.posting();
        app.change_posting(crate::permissions::Change {
            room: "!r:example".into(),
            base: "$pl".into(),
            action: crate::permissions::Action::Restrict,
        });
        let Ok(Command::RoomState { id, .. }) = rx.try_recv() else {
            panic!("expected a fresh read")
        };
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$pl", "$bob")),
        });
        let Ok(Command::SetPowerLevels { id, room, content }) = rx.try_recv() else {
            panic!("expected the power levels write")
        };
        assert_eq!(room, "!r:example");
        assert_eq!(content["events_default"], 50);
        assert_eq!(
            content["users"]["@me:example"], 100,
            "unrelated fields stay"
        );
        app.engine_event(Event::Created {
            id,
            result: Ok("$locked".into()),
        });
        let posting = &app.live.list["!r:example"].posting;
        assert_eq!(posting.id(), Some("$locked"));
        assert_eq!(posting.can_post("@bob:example", false), Some(false));
    }

    #[test]
    fn posting_preflight_drops_a_change_when_the_document_moved_on() {
        let (mut app, rx) = admin();
        app.permission_panel = true;
        app.live.list.get_mut("!r:example").unwrap().posting = state("$pl", "$bob").posting();
        app.change_posting(crate::permissions::Change {
            room: "!r:example".into(),
            base: "$pl".into(),
            action: crate::permissions::Action::Restrict,
        });
        let Ok(Command::RoomState { id, .. }) = rx.try_recv() else {
            panic!("expected a fresh read")
        };
        // Someone else changed the power levels since the panel opened.
        app.engine_event(Event::RoomState {
            id,
            result: Ok(state("$new", "$bob")),
        });
        assert!(rx.try_recv().is_err(), "no write over the newer document");
        assert_eq!(app.live.list["!r:example"].posting.id(), Some("$new"));
    }
}
