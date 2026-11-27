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
        let mut draft_removed = false;
        if !b.caption.is_empty() {
            if room == self.selected && self.editor.text() == b.caption {
                self.editor = text_editor::Content::new();
            }
            if self.drafts.get(room) == Some(&b.caption) {
                self.drafts.remove(room);
                draft_removed = true;
            }
            b.caption.clear();
        }
        self.outbox.remove(room, id);
        b.ids.pop_front();
        if draft_removed {
            self.drafts_changed();
        }
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
        app.engine_event(crate::engine::Event::UploadProgress {
            id: first,
            fraction: 0.375,
        });
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
        // Late answers for the cancelled attempt change nothing.
        app.engine_event(crate::engine::Event::UploadProgress {
            id: first,
            fraction: 0.99,
        });
        app.engine_event(crate::engine::Event::Uploaded {
            id: first,
            result: Ok(json!({"body":"demo.bin","msgtype":"m.file","file":{"url":"mxc://s/old"}})),
        });
        assert!(rx.try_recv().is_err() && !other.is_closed());
        let u = app.outbox.rooms["!a"][0].upload.as_ref().unwrap();
        assert_eq!((u.request, u.percent), (second, None));
    }
}
