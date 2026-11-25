use crate::Msg;
use crate::{App, Request, backend::Command, media};
use iced::widget::text_editor;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct Batch {
    pub room: String,
    ids: VecDeque<u64>,
    caption: String,
    reply: Option<crate::rooms::Quote>,
    // send_complete can race the RPC reply containing the local transaction ID.
    early: std::collections::BTreeMap<String, bool>,
}

impl App {
    pub fn stage_files(&mut self, room: String, files: Vec<media::LocalFile>) -> iced::Task<Msg> {
        if files.is_empty() {
            self.status = "Select regular files to attach.".into();
            return iced::Task::none();
        }
        let jobs = self.outbox.stage(&room, files, self.media_limit);
        iced::Task::batch(jobs.into_iter().map(|(id, file)| {
            let room = room.clone();
            iced::Task::perform(crate::preview::load(file), move |thumb| {
                Msg::Thumbnail(room, id, thumb)
            })
        }))
    }
    pub fn can_stage(&self) -> bool {
        !self.account
            && self.setup.is_none()
            && self.mentions.is_none()
            && !self.selected.is_empty()
            && (!self.logged_in || self.verified)
    }
    pub fn send_files(&mut self, room: String, caption: String) {
        if !self.can_stage() || self.batch.is_some() || self.media_config_pending() {
            return;
        }
        self.outbox.skipped.remove(&room);
        if !self.logged_in {
            if let Some(q) = self.outbox.rooms.remove(&room)
                && let Some(r) = self.demo.list.get_mut(&room)
            {
                for (i, a) in q.into_iter().enumerate() {
                    let body = format!(
                        "📎 {}{} (local preview)",
                        a.source.name,
                        if i == 0 && !caption.is_empty() {
                            format!(" — {caption}")
                        } else {
                            String::new()
                        }
                    );
                    r.insert(&json!({"rowid":r.history.keys().next_back().copied().unwrap_or(0)+1,"type":"m.room.message","sender":"You","timestamp":chrono::Utc::now().timestamp_millis(),"content":{"body":body}}));
                }
                self.editor = text_editor::Content::new();
            }
            return;
        }
        if !self.connected {
            return;
        }
        let ids = self.outbox.rooms[&room].iter().map(|a| a.id).collect();
        self.batch = Some(Batch {
            reply: self.reply_drafts.get(&room).cloned(),
            room,
            ids,
            caption,
            early: Default::default(),
        });
        self.advance_file();
    }
    pub fn media_config_pending(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::MediaConfig))
    }
    fn advance_file(&mut self) {
        let Some(b) = &mut self.batch else { return };
        let Some(&id) = b.ids.front() else {
            self.batch = None;
            return;
        };
        let room = b.room.clone();
        let caption = b.caption.clone();
        let Some(r) = self.live.list.get(&room) else {
            self.batch = None;
            return;
        };
        let encrypt = r.encrypted;
        if !r.has_metadata {
            self.fail_file(
                &room,
                id,
                "Waiting for room encryption information. Retry after sync.",
            );
            return;
        }
        let Some(a) = self.outbox.get_mut(&room, id) else {
            b.ids.pop_front();
            self.advance_file();
            return;
        };
        a.busy = true;
        a.cancelled = false;
        a.error = None;
        if let Some(descriptor) = &a.descriptor {
            let mut content = descriptor.clone();
            if encrypt && !content["file"].is_object() {
                a.descriptor = None;
                self.fail_file(
                    &room,
                    id,
                    "Room encryption changed. Send again to encrypt the file.",
                );
                return;
            }
            if !caption.trim().is_empty() {
                content["filename"] = json!(a.source.name);
                content["body"] = json!(caption);
            }
            // Put captions in base_content, not in the slash-command parser.
            let next = self.seq + 1;
            let (command, mut data) = if let Some(txn) = &a.txn {
                ("resend_event", json!({"transaction_id":txn}))
            } else {
                (
                    "send_message",
                    json!({"room_id":room,"base_content":content,"text":""}),
                )
            };
            if command == "send_message"
                && let Some(reply) = &b.reply
            {
                reply.relate(&mut data);
            }
            self.rpc(command, data, Request::MediaSend(room.clone(), id));
            if !self.pending.contains_key(&next) {
                self.fail_file(&room, id, "Could not queue the message.");
            }
        } else {
            self.seq += 1;
            let (lease, cancel) = async_channel::bounded(1);
            if self
                .commands
                .try_send(Command::Upload {
                    cancel,
                    id: self.seq,
                    file: a.source.clone(),
                    encrypt,
                    limit: self.media_limit,
                })
                .is_ok()
            {
                a.upload = Some(media::UploadAttempt {
                    request: self.seq,
                    percent: None,
                    _lease: lease,
                });
                self.pending.insert(self.seq, Request::Upload(room, id));
            } else {
                self.fail_file(&room, id, "Could not queue the upload.");
            }
        }
    }
    pub fn uploaded(&mut self, room: &str, id: u64, value: &Value) {
        if !self.current_file(room, id) {
            return;
        }
        if media::Media::parse(value).is_none() {
            self.fail_file(room, id, "gomuks returned an incomplete attachment.");
            return;
        }
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.upload = None;
            a.descriptor = Some(value.clone());
        }
        self.advance_file();
    }
    pub fn cancel_upload(&mut self, room: &str, id: u64, request: i64) {
        if !self.current_file(room, id)
            || !matches!(self.pending.get(&request),Some(Request::Upload(r,i)) if r==room && *i==id)
        {
            return;
        }
        let Some(a) = self.outbox.get_mut(room, id) else {
            return;
        };
        if !a.upload.as_ref().is_some_and(|u| u.request == request) {
            return;
        }
        // Invalidate the RPC *before* releasing the transport: even a success
        // already in the event queue must never advance to send_message.
        self.pending.remove(&request);
        a.upload = None;
        a.busy = false;
        a.cancelled = true;
        a.error = None;
        self.batch = None;
    }
    pub fn upload_progress(&mut self, request: i64, fraction: f64) {
        if !(0.0..=1.0).contains(&fraction) {
            return;
        }
        if let Some(Request::Upload(room, id)) = self.pending.get(&request)
            && let Some(a) = self.outbox.get_mut(room, *id)
            && let Some(upload) = &mut a.upload
            && upload.request == request
        {
            upload.percent = Some((fraction * 100.).floor() as u8);
        }
    }
    fn current_file(&self, room: &str, id: u64) -> bool {
        self.batch
            .as_ref()
            .is_some_and(|b| b.room == room && b.ids.front() == Some(&id))
    }
    pub fn file_sent(&mut self, room: &str, id: u64, event: &Value) {
        if !self.current_file(room, id) {
            return;
        }
        if crate::rooms::server_confirmed(event) {
            self.live.event(&json!(room), event);
            self.finish_delivery(room, id, true);
            return;
        }
        let Some(txn) = event["transaction_id"].as_str() else {
            self.fail_file(
                room,
                id,
                "No send confirmation from gomuks. Check the timeline before retrying.",
            );
            return;
        };
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.txn = Some(txn.into());
        }
        if let Some(ok) = self.batch.as_mut().unwrap().early.remove(txn) {
            self.finish_delivery(room, id, ok);
        }
    }
    pub fn media_delivery(&mut self, event: &Value, ok: bool) {
        let (Some(txn), Some(room)) = (event["transaction_id"].as_str(), event["room_id"].as_str())
        else {
            return;
        };
        // A sync echo can arrive after a transient failure. Retire the retry
        // card for precisely this transaction, never another file in the room.
        if ok
            && crate::rooms::server_confirmed(event)
            && let Some(id) = self
                .outbox
                .rooms
                .get(room)
                .and_then(|q| q.iter().find(|a| !a.busy && a.txn.as_deref() == Some(txn)))
                .map(|a| a.id)
        {
            self.outbox.remove(room, id);
        }
        let Some(b) = &mut self.batch else { return };
        let Some(&id) = b.ids.front() else { return };
        if room != b.room {
            return;
        }
        if self
            .outbox
            .get_mut(room, id)
            .is_some_and(|a| a.txn.as_deref() == Some(txn))
        {
            self.finish_delivery(room, id, ok);
        } else if self
            .pending
            .values()
            .any(|p| matches!(p,Request::MediaSend(r,i) if r==room && *i==id))
            && b.early.len() < 32
        {
            b.early
                .entry(txn.into())
                .and_modify(|confirmed| *confirmed |= ok)
                .or_insert(ok);
        }
    }
    pub fn finish_delivery(&mut self, room: &str, id: u64, ok: bool) {
        if !self.current_file(room, id) {
            return;
        }
        if !ok {
            self.fail_file(
                room,
                id,
                "Delivery failed. Send again to retry the same Matrix event.",
            );
            return;
        }
        let b = self.batch.as_mut().unwrap();
        b.early.clear();
        if let Some(reply) = b.reply.take()
            && self.reply_drafts.get(room) == Some(&reply)
        {
            self.reply_drafts.remove(room);
        }
        if !b.caption.is_empty() {
            if room == self.selected && self.editor.text() == b.caption {
                self.editor = text_editor::Content::new();
            }
            if self.drafts.get(room) == Some(&b.caption) {
                self.drafts.remove(room);
            }
            b.caption.clear();
        }
        self.outbox.remove(room, id);
        b.ids.pop_front();
        self.advance_file();
    }
    pub fn fail_file(&mut self, room: &str, id: u64, why: &str) {
        if let Some(a) = self.outbox.get_mut(room, id) {
            a.upload = None;
            a.busy = false;
            a.error = Some(why.into());
        }
        if self.current_file(room, id) {
            self.batch = None;
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::rooms::Room;
    #[test]
    fn late_media_config_rechecks_each_room_without_publishing_an_orphaned_caption() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.connected = true;
        app.verified = true;
        app.logged_in = true;
        app.selected = "!a".into();
        let file = |n| media::LocalFile {
            name: format!("{n}.bin"),
            size: n,
            origin: media::NativeSource::Path("unused".into()),
        };
        for room in ["!a", "!b"] {
            app.live.list.insert(
                room.into(),
                Room {
                    has_metadata: true,
                    ..Default::default()
                },
            );
        }
        app.rpc("get_media_config", json!({}), Request::MediaConfig);
        let cfg = rx.try_recv().unwrap().into_rpc()["request_id"].clone();
        let _ = app.update(Msg::Files("!a".into(), vec![file(11)]));
        let _ = app.update(Msg::Files("!b".into(), vec![file(10), file(11)]));
        app.editor = text_editor::Content::with_text("Private draft");
        let _ = app.update(Msg::Send);
        assert!(rx.is_empty() && app.batch.is_none());
        app.frame(json!({"command":"response","request_id":cfg,"data":{"m.upload.size":10}}));
        assert_eq!(app.outbox.rooms["!b"].len(), 1);
        assert_eq!(app.outbox.rooms["!b"][0].source.size, 10);
        assert!(!app.outbox.rooms.contains_key("!a"));
        let _ = app.update(Msg::DismissSkipped("!b".into())); // stale click for another room
        let _ = app.update(Msg::Send);
        assert!(rx.is_empty() && app.outbox.skipped.contains_key("!b"));
        assert_eq!(app.editor.text(), "Private draft");
        let _ = app.update(Msg::DismissSkipped("!a".into()));
        let _ = app.update(Msg::Send);
        let sent = rx.try_recv().unwrap().into_rpc();
        assert_eq!(
            (&sent["command"], &sent["data"]["text"]),
            (&json!("send_message"), &json!("Private draft"))
        );
    }
    #[test]
    fn cancelled_attempt_can_never_send_or_cancel_its_retry() {
        let (tx, rx) = async_channel::bounded(8);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.connected = true;
        app.verified = true;
        app.selected = "!a".into();
        app.live.list.insert(
            "!a".into(),
            Room {
                has_metadata: true,
                encrypted: true,
                ..Default::default()
            },
        );
        let _ = app.update(Msg::Files(
            "!a".into(),
            ["first.bin", "next.bin"]
                .into_iter()
                .map(|name| media::LocalFile {
                    name: name.into(),
                    size: 1,
                    origin: media::NativeSource::Path("unused".into()),
                })
                .collect(),
        ));
        let _ = app.update(Msg::Send);
        let Command::Upload {
            id: first,
            cancel: lease,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected upload")
        };
        let attachment = app.outbox.rooms["!a"][0].id;
        app.frame(json!({"command":"upload_progress","request_id":first,"data":0.375}));
        assert_eq!(app.outbox.rooms["!a"][0].status_label(), "Upload 37%");
        app.cancel_upload("!a", attachment, first);
        assert!(lease.is_closed());
        assert_eq!(app.outbox.rooms["!a"].len(), 2);
        assert!(
            rx.try_recv().is_err(),
            "cancelling must not start the next file"
        );
        let _ = app.update(Msg::Send);
        let Command::Upload {
            id: second,
            cancel: other,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected retry")
        };
        app.cancel_upload("!a", attachment, first);
        app.frame(json!({"command":"upload_progress","request_id":first,"data":0.99}));
        app.frame(json!({"command":"response","request_id":first,"data":{"body":"demo.bin","msgtype":"m.file","file":{"url":"mxc://s/old"}}}));
        assert!(rx.try_recv().is_err() && !other.is_closed());
        let u = app.outbox.rooms["!a"][0].upload.as_ref().unwrap();
        assert_eq!((u.request, u.percent), (second, None));
    }
    #[test]
    fn accepted_event_with_gomuks_pending_sentinel_clears_the_tray_in_either_order() {
        for completion_first in [true, false] {
            let (tx, rx) = async_channel::bounded(8);
            let mut app = App::new(tx);
            app.logged_in = true;
            app.connected = true;
            app.verified = true;
            app.selected = "!a".into();
            app.live.list.insert(
                "!a".into(),
                Room {
                    has_metadata: true,
                    ..Default::default()
                },
            );
            let _ = app.update(Msg::Files(
                "!a".into(),
                vec![media::LocalFile {
                    name: "photo.png".into(),
                    size: 42,
                    origin: media::NativeSource::Path("unused".into()),
                }],
            ));
            let _ = app.update(Msg::Send);
            let Command::Upload { id, .. } = rx.try_recv().unwrap() else {
                panic!("expected upload")
            };
            let content =
                json!({"msgtype":"m.image","url":"mxc://example/photo","body":"photo.png"});
            app.frame(json!({"command":"response","request_id":id,"data":content}));
            let rpc = rx.try_recv().unwrap().into_rpc();
            let local = json!({"room_id":"!a","rowid":7,"transaction_id":"tx","event_id":"~tx","type":"m.room.message","content":content,"send_error":"not sent"});
            let mut accepted = local.clone();
            accepted["event_id"] = json!("$from-homeserver");
            let done = json!({"command":"send_complete","data":{"event":accepted,"error":null}});
            let reply = json!({"command":"response","request_id":rpc["request_id"],"data":local});
            if completion_first {
                app.frame(done);
                app.frame(reply);
            } else {
                app.frame(reply);
                app.frame(done);
            }
            assert!(app.batch.is_none() && app.outbox.rooms.is_empty());
            assert!(
                rx.try_recv().is_err(),
                "a delivered file must not be resent"
            );
            let r = app.live.list.get_mut("!a").unwrap();
            r.insert(&local); // stale local echo after server confirmation
            assert_eq!(r.history[&7].body, "photo.png");
            assert!(!app.status.contains("failed"));
        }
    }
    #[test]
    fn staged_file_only_sends_on_submit_and_retries_in_original_room() {
        let (tx, rx) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.logged_in = true;
        app.verified = true;
        app.connected = true;
        app.selected = "!a".into();
        app.live.list.insert(
            "!a".into(),
            Room {
                has_metadata: true,
                encrypted: true,
                ..Default::default()
            },
        );
        app.live.list.insert("!b".into(), Room::default());
        let _ = app.update(Msg::Files(
            "!a".into(),
            vec![media::LocalFile {
                name: "report.pdf".into(),
                size: 2,
                origin: media::NativeSource::Path("unused".into()),
            }],
        ));
        assert!(rx.try_recv().is_err());
        let _ = app.update(Msg::Send);
        let Command::Upload { id, encrypt, .. } = rx.try_recv().unwrap() else {
            panic!("not an upload")
        };
        assert!(encrypt);
        app.selected = "!b".into();
        let descriptor =
            json!({"msgtype":"m.file","filename":"report.pdf","file":{"url":"mxc://test/blob"}});
        app.frame(json!({"command":"response","request_id":id,"data":descriptor}));
        let first = rx.try_recv().unwrap().into_rpc();
        assert_eq!(first["data"]["room_id"], "!a");
        app.frame(json!({"command":"error","request_id":first["request_id"]}));
        app.selected = "!a".into();
        let _ = app.update(Msg::Send);
        let retry = rx.try_recv().unwrap().into_rpc();
        assert_eq!(retry["command"], "send_message");
        assert_eq!(retry["data"]["base_content"], descriptor);
        let echo = json!({"room_id":"!a","rowid":9,"transaction_id":"hicli-test","type":"m.room.message","content":descriptor});
        app.frame(json!({"command":"response","request_id":retry["request_id"],"data":echo}));
        assert!(app.batch.is_some(), "a local echo isn't confirmed delivery");
        app.frame(json!({"command":"send_complete","data":{"event":echo,"error":"network"}}));
        let _ = app.update(Msg::Send);
        let resend = rx.try_recv().unwrap().into_rpc();
        assert_eq!(resend["command"], "resend_event");
        assert_eq!(resend["data"]["transaction_id"], "hicli-test");
        app.frame(json!({"command":"send_complete","data":{"event":echo,"error":null}}));
        app.frame(json!({"command":"response","request_id":resend["request_id"],"data":echo}));
        assert!(app.batch.is_none() && app.outbox.rooms.is_empty());
    }
}
