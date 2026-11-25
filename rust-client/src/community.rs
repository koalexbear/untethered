//! A space is an invite-only Matrix room. Its channel is created with the
//! restricted join rule and encryption already in place, then linked into
//! the parent's directory. Retrying the link must never create another room.
use crate::{
    App, Msg, Request, Section,
    permissions::Posting,
    rooms::{VOICE_ROOM, items},
};
use iced::{Task, widget::operation};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Space,
    Text,
    Voice,
    Invite,
}
#[derive(Clone)]
pub enum Input {
    Start(Option<String>),
    InviteMember(String),
    Name(String),
    Invite(String),
    Kind(Kind),
    Submit,
    Close,
}
#[derive(Clone, Copy)]
pub enum Step {
    Check,
    Create,
    Link,
    Invite,
}
pub struct Form {
    pub parent: Option<String>,
    pub kind: Kind,
    pub name: String,
    pub invite: String,
    pub created: Option<String>,
    pub done: bool,
    pub note: String,
}
impl Form {
    pub fn valid(&self, me: &str) -> bool {
        let name = self.name.trim();
        let id = self.invite.trim();
        let valid_id = id != me
            && id.len() <= 255
            && !id.chars().any(|c| c.is_whitespace() || c.is_control())
            && id
                .strip_prefix('@')
                .and_then(|s| s.split_once(':'))
                .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty());
        if self.kind == Kind::Invite {
            return valid_id;
        }
        !name.is_empty()
            && name.len() <= 255
            && !name.chars().any(char::is_control)
            && (id.is_empty() || (self.kind == Kind::Space && valid_id))
    }
    fn params(&self, via: &str) -> Value {
        let mut state = vec![
            json!({"type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"joined"}}),
        ];
        let mut p = json!({"name":self.name.trim(),"visibility":"private","preset":"private_chat",
            "power_level_content_override":{"invite":50}});
        if let Some(space) = &self.parent {
            state.extend([
                json!({"type":"m.room.join_rules","state_key":"","content":{"join_rule":"restricted",
                        "allow":[{"type":"m.room_membership","room_id":space}]}}),
                json!({"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
                json!({"type":"m.space.parent","state_key":space,"content":{"via":[via],"canonical":true}})]);
            if self.kind == Kind::Voice {
                p["creation_content"] = json!({"type":VOICE_ROOM});
                p["power_level_content_override"]["events"] =
                    json!({"org.matrix.msc3401.call.member":0,"org.matrix.msc4143.rtc.slot":0});
            }
        } else {
            p["creation_content"] = json!({"type":"m.space"});
            let id = self.invite.trim();
            if !id.is_empty() {
                p["invite"] = json!([id]);
            }
        }
        p["initial_state"] = Value::Array(state);
        p
    }
}
impl App {
    pub fn setup_busy(&self) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Request::Setup(_)))
    }
    pub fn cancel_setup_if_idle(&mut self) {
        if !self.setup_busy() {
            self.setup = None;
        }
    }
    pub fn setup_input(&mut self, input: Input) -> Task<Msg> {
        if self.setup_busy() {
            return Task::none();
        }
        let start = match &input {
            Input::Start(p) => Some((
                p.clone(),
                if p.is_some() { Kind::Text } else { Kind::Space },
            )),
            Input::InviteMember(p) => Some((Some(p.clone()), Kind::Invite)),
            _ => None,
        };
        if let Some((parent, kind)) = start {
            if !self.logged_in || !self.verified || !self.connected || self.setup.is_some() {
                return Task::none();
            }
            if parent
                .as_ref()
                .is_some_and(|p| !self.live.list.get(p).is_some_and(|r| r.space))
            {
                return Task::none();
            }
            #[cfg(target_os = "linux")]
            self.minimize_call();
            self.close_mentions();
            self.close_original();
            self.clear_search();
            self.setup = Some(Form {
                kind,
                parent,
                name: String::new(),
                invite: String::new(),
                created: None,
                done: false,
                note: String::new(),
            });
            return operation::focus(if kind == Kind::Invite {
                "setup-invite"
            } else {
                "setup-name"
            });
        }
        let Some(f) = &mut self.setup else {
            return Task::none();
        };
        match input {
            Input::Close => self.setup = None,
            Input::Submit
                if self.connected
                    && self.logged_in
                    && self.verified
                    && !f.done
                    && f.valid(&self.matrix_id) =>
            {
                if let Some(parent) = f.parent.clone() {
                    if self.live.list.get(&parent).is_some_and(|r| r.space) {
                        f.note = "Checking space permissions…".into();
                        self.setup_rpc(
                            "get_room_state",
                            json!({"room_id":parent,"refetch":true,"include_members":true}),
                            Step::Check,
                        );
                    } else {
                        f.note = "You are no longer in this space.".into();
                    }
                } else {
                    self.create_community_room();
                }
            }
            Input::Name(v) if f.created.is_none() => f.name = v,
            Input::Invite(v)
                if f.created.is_none() && matches!(f.kind, Kind::Space | Kind::Invite) =>
            {
                f.invite = v
            }
            Input::Kind(k)
                if f.created.is_none()
                    && matches!(f.kind, Kind::Text | Kind::Voice)
                    && matches!(k, Kind::Text | Kind::Voice) =>
            {
                f.kind = k
            }
            _ => {}
        }
        Task::none()
    }
    fn setup_rpc(&mut self, cmd: &str, params: Value, step: Step) {
        self.rpc(cmd, params, Request::Setup(step));
        if !self.pending.contains_key(&self.seq) {
            self.setup_error(step);
        }
    }
    fn create_community_room(&mut self) {
        let Some(f) = &mut self.setup else { return };
        let Some((_, via)) = self.matrix_id.split_once(':') else {
            return;
        };
        if f.created.is_some() || !f.valid(&self.matrix_id) {
            return;
        }
        f.note = "Creating room…".into();
        let params = f.params(via);
        self.setup_rpc("create_room", params, Step::Create);
    }
    fn link_community_room(&mut self) {
        let Some(f) = &mut self.setup else { return };
        let (Some(parent), Some(child), Some((_, via))) =
            (&f.parent, &f.created, self.matrix_id.split_once(':'))
        else {
            return;
        };
        f.note = "Linking channel into space…".into();
        let params = json!({"room_id":parent,"type":"m.space.child","state_key":child,"content":{"via":[via]}});
        self.setup_rpc("set_state", params, Step::Link);
    }
    pub fn setup_result(&mut self, step: Step, data: &Value) {
        let Some(f) = &mut self.setup else { return };
        match step {
            Step::Check => {
                let Some(parent) = &f.parent else { return };
                let mut levels = Posting::default();
                for e in items(data) {
                    levels.ingest(parent, e, false);
                }
                let allowed = if f.kind == Kind::Invite {
                    levels.can_invite(&self.matrix_id)
                } else {
                    levels.can_link_channel(&self.matrix_id)
                };
                if !self.live.list.get(parent).is_some_and(|r| r.space)
                    || !allowed
                    || !items(data).any(|e| {
                        e["room_id"] == *parent
                            && e["type"] == "m.room.member"
                            && e["state_key"] == self.matrix_id
                            && e["content"]["membership"] == "join"
                    })
                {
                    f.note = if f.kind == Kind::Invite {
                        "Your role cannot invite members to this space."
                    } else {
                        "Your role cannot add channels to this space."
                    }
                    .into();
                    return;
                }
                if f.kind == Kind::Invite {
                    if items(data).any(|e| {
                        e["room_id"] == *parent
                            && e["type"] == "m.room.member"
                            && e["state_key"] == f.invite.trim()
                            && matches!(
                                e["content"]["membership"].as_str(),
                                Some("invite" | "join" | "ban")
                            )
                    }) {
                        f.note = "This person is already invited, joined, or banned.".into();
                        return;
                    }
                    let body =
                        json!({"room_id":parent,"user_id":f.invite.trim(),"action":"invite"});
                    f.note = "Sending invitation…".into();
                    self.setup_rpc("set_membership", body, Step::Invite);
                } else if f.created.is_some() {
                    self.link_community_room()
                } else {
                    self.create_community_room()
                }
            }
            Step::Create => {
                let Some(id) = data["room_id"]
                    .as_str()
                    .filter(|s| s.starts_with('!') && s.len() > 1)
                else {
                    self.setup_error(step);
                    return;
                };
                f.created = Some(id.into());
                if f.parent.is_some() {
                    self.link_community_room();
                } else {
                    f.done = true;
                    f.note = "Space created. Waiting for sync…".into();
                }
            }
            Step::Link => {
                if data.as_str().is_none_or(|id| !id.starts_with('$')) {
                    self.setup_error(step);
                    return;
                }
                f.done = true;
                f.note = "Channel linked. Waiting for sync…".into();
            }
            Step::Invite => {
                f.done = true;
                f.note = format!("Invitation sent to {}.", f.invite.trim());
            }
        }
    }
    pub fn setup_error(&mut self, step: Step) {
        if let Some(f) = &mut self.setup {
            f.note=match step{
                Step::Check=>"Could not check your space permissions. Try again.",
                Step::Create=>"Creation was not confirmed. Check All rooms before trying again.",
                Step::Link=>"The room exists, but the space link was not confirmed. Retry linking this room.",
                Step::Invite=>"Could not invite this person. Check the Matrix ID and your space permissions."
            }.into();
        }
    }
    pub fn open_created(&mut self) {
        let Some(f) = &self.setup else { return };
        let Some(id) = f
            .created
            .as_ref()
            .filter(|id| f.done && self.live.list.get(*id).is_some_and(|r| r.has_metadata))
        else {
            return;
        };
        if f.parent.as_ref().is_some_and(|p| {
            !self
                .live
                .children
                .get(p)
                .is_some_and(|kids| kids.contains(id))
        }) {
            return;
        }
        let id = id.clone();
        let parent = f.parent.clone();
        self.setup = None;
        self.section = Section::Space(parent.clone().unwrap_or_else(|| id.clone()));
        if parent.is_some() {
            self.open(id);
        } else {
            self.reconcile();
        }
        self.status.clear();
    }
}
