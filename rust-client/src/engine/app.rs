//! UI side of the matrix-sdk engine: start it, route the supported requests
//! to typed commands, and apply its events to the same `App` state and
//! `rooms::Rooms` model the gomuks frames fill.
use super::{Account, Command, Connection, Event, Failure, Recovery, run};
use crate::{App, Msg, Request, account};
use iced::{Task, widget::text_editor};

impl App {
    pub(crate) fn boot_engine() -> (Self, Task<Msg>) {
        // gomuks-only paths (uploads, inline media, calls) fail fast on a
        // closed channel instead of reaching a backend that isn't running.
        let (gomuks, _) = async_channel::bounded(1);
        let (commands, rx) = async_channel::bounded(64);
        let (tx, events) = async_channel::bounded(64);
        let mut app = Self::new(gomuks);
        app.sdk = Some(commands);
        app.status = "Starting the matrix-sdk engine…".into();
        let task = Task::batch([
            Task::perform(run(rx, tx), |result| {
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
        (app, task)
    }

    fn command(&mut self, make: impl FnOnce(i64) -> Command, request: Request) {
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

    /// gomuks RPCs the matrix-sdk engine doesn't implement yet. Background
    /// reads stay quiet; anything the user asked for says why nothing happened.
    pub(crate) fn engine_unsupported(&mut self, request: Request) {
        match request {
            Request::MediaConfig | Request::Members(_) => {}
            Request::Mentions(_) => self.mentions_failed(),
            Request::Search(room, kind) => self.fail_search(&room, kind),
            Request::Setup(step) => self.setup_error(step),
            Request::Report => self.report_result(false),
            Request::RtcDiscovery => {
                self.rtc_status =
                    "MatrixRTC discovery isn’t available with the matrix-sdk engine yet.".into();
            }
            _ => {
                self.status = "This isn’t available with the matrix-sdk engine yet.".into();
            }
        }
    }

    pub(crate) fn engine_login(&mut self) {
        if self.logged_in
            || !self.connected
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
        if self.outbox.rooms.get(&room).is_some_and(|q| !q.is_empty()) {
            self.status = "Attachments aren’t available with the matrix-sdk engine yet.".into();
            return;
        }
        let body = self.editor.text();
        if room.is_empty()
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
        self.command(
            |id| Command::Send {
                id,
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
            Event::Rooms(list) => {
                for left in self.live.apply_engine(list) {
                    self.reply_drafts.remove(&left);
                    self.drafts.remove(&left);
                    self.loaded.remove(&left);
                }
                if self.logged_in {
                    self.reconcile();
                }
            }
            Event::Timeline(t) => {
                if let Some(r) = self.live.list.get_mut(&t.room) {
                    r.apply_engine(t.messages, t.start_reached);
                }
            }
            Event::Done { id, result } => self.engine_done(id, result),
            Event::Connection(connection) => {
                const OFFLINE: &str = "Offline. Untethered will reconnect automatically.";
                const RETRYING: &str = "Sync stopped after an error. Retrying…";
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
                    self.status = text.into();
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
        let unavailable = a.logged_in && a.recovery == Recovery::Disabled;
        self.apply_account(account::State {
            logged_in: a.logged_in,
            verified: a.logged_in && a.verified,
            recovery_unavailable: unavailable,
            profile,
            user_id: a.user_id,
        });
        if unavailable && !self.verified {
            self.verification_note = "Recovery isn’t set up for this account, so a recovery key can’t verify this device. Set up recovery in a trusted Matrix client.".into();
        }
    }

    fn engine_done(&mut self, id: i64, result: Result<(), Failure>) {
        let Some(request) = self.pending.remove(&id) else {
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
            (Request::Page(_), Err(_)) => {
                self.status = "Could not load history. Use Earlier messages to retry.".into();
            }
            (Request::Send(room, sent, reply), Ok(())) => {
                // Same draft handling as the gomuks send_message response.
                let same = self.reply_drafts.get(&room) == reply.as_ref();
                if same && self.selected == room && self.editor.text() == sent {
                    self.editor = text_editor::Content::new();
                }
                if same && self.drafts.get(&room) == Some(&sent) {
                    self.drafts.remove(&room);
                }
                if same {
                    self.reply_drafts.remove(&room);
                }
                self.status.clear();
            }
            (Request::Send(..), Err(_)) => {
                self.status = "Could not send. Your draft has been kept.".into();
            }
            _ => {}
        }
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
            verified,
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
        app.engine_event(Event::Timeline(TimelineSnapshot {
            room: room.clone(),
            start_reached: true,
            messages: vec![Message {
                row: 1,
                event_id: Some("$1".into()),
                sender: "@you:example".into(),
                sender_name: Some("You".into()),
                time: 1,
                body: "hello".into(),
                kind: crate::engine::Kind::Text,
                encrypted: true,
                delivery: crate::engine::Delivery::Sent,
                edited: false,
                reply_to: None,
            }],
        }));
        app.engine_event(Event::Done { id, result: Ok(()) });
        let r = &app.live.list[&room];
        assert!(r.end && r.posts().next().unwrap().body == "hello");

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

        // Unsupported gomuks RPCs never reach a backend.
        let _ = app.update(Msg::CheckCalls);
        assert!(rx.try_recv().is_err());
        assert!(app.rtc_status.contains("matrix-sdk engine"));
    }
}
