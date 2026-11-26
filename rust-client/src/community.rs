//! A space is an invite-only Matrix room. Its channel is created with the
//! restricted join rule and encryption already in place, then linked into
//! the parent's directory. Retrying the link must never create another room.
use crate::{
    App, Msg, Request, Section, moderation::Kind as MemberKind, room_state::RoomState,
    rooms::VOICE_ROOM,
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
                        self.fetch_room_state(&parent, Request::Setup(Step::Check));
                        self.setup_queued(Step::Check);
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
    /// Report a setup request that couldn't be queued.
    fn setup_queued(&mut self, step: Step) {
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
        self.send_create_room(params, Request::Setup(Step::Create));
        self.setup_queued(Step::Create);
    }
    fn link_community_room(&mut self) {
        let Some(f) = &mut self.setup else { return };
        let (Some(parent), Some(child), Some((_, via))) =
            (&f.parent, &f.created, self.matrix_id.split_once(':'))
        else {
            return;
        };
        f.note = "Linking channel into space…".into();
        let (parent, child, via) = (parent.clone(), child.clone(), via.to_owned());
        self.send_space_child(&parent, &child, &via, Request::Setup(Step::Link));
        self.setup_queued(Step::Link);
    }
    /// The parent space's state: may we add a channel or invite, and is the
    /// invitee new to it?
    pub fn setup_checked(&mut self, state: RoomState) {
        let Some(f) = &mut self.setup else { return };
        let Some(parent) = f.parent.clone() else {
            return;
        };
        let levels = state.posting();
        let allowed = if f.kind == Kind::Invite {
            levels.can_invite(&self.matrix_id)
        } else {
            levels.can_link_channel(&self.matrix_id)
        };
        if !self.live.list.get(&parent).is_some_and(|r| r.space)
            || !allowed
            || state.membership(&self.matrix_id) != Some(MemberKind::Joined)
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
            let invitee = f.invite.trim().to_owned();
            if matches!(
                state.membership(&invitee),
                Some(MemberKind::Invited | MemberKind::Joined | MemberKind::Banned)
            ) {
                f.note = "This person is already invited, joined, or banned.".into();
                return;
            }
            f.note = "Sending invitation…".into();
            self.send_membership(
                &parent,
                &invitee,
                crate::moderation::Action::Invite,
                "",
                Request::Setup(Step::Invite),
            );
            self.setup_queued(Step::Invite);
        } else if f.created.is_some() {
            self.link_community_room()
        } else {
            self.create_community_room()
        }
    }
    pub fn setup_created(&mut self, id: String) {
        let Some(f) = &mut self.setup else { return };
        if !(id.starts_with('!') && id.len() > 1) {
            self.setup_error(Step::Create);
            return;
        }
        f.created = Some(id);
        if f.parent.is_some() {
            self.link_community_room();
        } else {
            f.done = true;
            f.note = "Space created. Waiting for sync…".into();
        }
    }
    pub fn setup_linked(&mut self, event_id: &str) {
        let Some(f) = &mut self.setup else { return };
        if !event_id.starts_with('$') {
            self.setup_error(Step::Link);
            return;
        }
        f.done = true;
        f.note = "Channel linked. Waiting for sync…".into();
    }
    pub fn setup_invited(&mut self) {
        let Some(f) = &mut self.setup else { return };
        f.done = true;
        f.note = format!("Invitation sent to {}.", f.invite.trim());
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
