//! Notifications from the SDK's push rule evaluation, and the push rule and
//! room account data changes the alert settings make.
use super::{
    Alert, Event, Failure,
    messaging::{http, room},
};
use async_channel::Sender;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::RawAnySyncOrStrippedTimelineEvent,
    ruma::{
        MilliSecondsSinceUnixEpoch, UserId,
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, RoomAccountDataEventType,
            SyncMessageLikeEvent, room::message::Relation,
        },
        push::{Action, Tweak},
        serde::Raw,
    },
    sync::Notification,
};
use serde_json::Value;

/// Events from longer than this before the session started don't alert:
/// the first sync after signing in or starting brings recent history, which
/// the SDK evaluates like new messages. Allows for some clock skew.
const SKEW_MS: u64 = 2 * 60 * 1000;

/// Pass on what the account's push rules say to notify about. The SDK
/// evaluates them after decryption, so encrypted messages have their text.
pub async fn register(client: &Client, events: Sender<Event>) {
    let since = u64::from(MilliSecondsSinceUnixEpoch::now().0).saturating_sub(SKEW_MS);
    client
        .register_notification_handler(
            move |notification: Notification, room: Room, client: Client| {
                let events = events.clone();
                async move {
                    let Some(me) = client.user_id() else {
                        return;
                    };
                    if let Some(notice) = notice(&notification, &room, me, since) {
                        #[cfg(debug_assertions)]
                        super::qa::alert(&notice);
                        // Sync waits for this handler; an alert isn't worth
                        // holding it up for.
                        let _ = events.try_send(Event::Notification(notice));
                    }
                }
            },
        )
        .await;
}

/// Messages and stickers from others; not edits, reactions, state, or
/// messages that are still undecryptable.
fn notice(notification: &Notification, room: &Room, me: &UserId, since: u64) -> Option<Alert> {
    let RawAnySyncOrStrippedTimelineEvent::Sync(raw) = &notification.event else {
        return None;
    };
    let event = raw.deserialize().ok()?;
    if event.sender() == me || u64::from(event.origin_server_ts().0) < since {
        return None;
    }
    let body = match &event {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(m),
        )) => {
            if matches!(m.content.relates_to, Some(Relation::Replacement(_))) {
                return None;
            }
            m.content.body().to_owned()
        }
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Sticker(
            SyncMessageLikeEvent::Original(s),
        )) => s.content.body.clone(),
        _ => return None,
    };
    Some(Alert {
        room: room.room_id().to_string(),
        event_id: event.event_id().to_string(),
        sender: event.sender().to_string(),
        body,
        sound: notification
            .actions
            .iter()
            .any(|a| matches!(a, Action::SetTweak(Tweak::Sound(_)))),
        highlight: notification.actions.iter().any(Action::is_highlight),
    })
}

/// Make one push rule change through the Matrix push rules API.
#[cfg(any(target_os = "linux", target_arch = "wasm32"))]
pub async fn push_rule(
    client: &Client,
    kind: crate::notifications::RuleKind,
    rule_id: &str,
    change: crate::notifications::RuleChange,
) -> Result<(), Failure> {
    use crate::notifications::{RuleChange, RuleKind};
    use matrix_sdk::ruma::{
        RoomId,
        api::client::push::{delete_pushrule, set_pushrule, set_pushrule_enabled},
        push::{NewConditionalPushRule, NewPushRule, NewSimplePushRule, PushCondition},
    };
    let wire = match kind {
        RuleKind::Override => matrix_sdk::ruma::push::RuleKind::Override,
        RuleKind::Room => matrix_sdk::ruma::push::RuleKind::Room,
        RuleKind::Underride => matrix_sdk::ruma::push::RuleKind::Underride,
    };
    let rule_id = rule_id.to_owned();
    let sent = match change {
        RuleChange::Enabled(on) => client
            .send(set_pushrule_enabled::v3::Request::new(wire, rule_id, on))
            .await
            .map(drop),
        RuleChange::Delete => client
            .send(delete_pushrule::v3::Request::new(wire, rule_id))
            .await
            .map(drop),
        RuleChange::Put {
            actions,
            conditions,
        } => {
            let actions: Vec<Action> =
                serde_json::from_value(actions).map_err(|_| Failure::Failed)?;
            let conditions: Vec<PushCondition> = match conditions {
                Some(c) => serde_json::from_value(c).map_err(|_| Failure::Failed)?,
                None => Vec::new(),
            };
            let rule = match kind {
                RuleKind::Room => NewPushRule::Room(NewSimplePushRule::new(
                    RoomId::parse(&rule_id).map_err(|_| Failure::Failed)?,
                    actions,
                )),
                RuleKind::Override => {
                    NewPushRule::Override(NewConditionalPushRule::new(rule_id, conditions, actions))
                }
                RuleKind::Underride => NewPushRule::Underride(NewConditionalPushRule::new(
                    rule_id, conditions, actions,
                )),
            };
            client
                .send(set_pushrule::v3::Request::new(rule))
                .await
                .map(drop)
        }
    };
    sent.map_err(|e| {
        if e.client_api_error_kind().is_some() {
            Failure::Failed
        } else {
            Failure::Unreachable
        }
    })
}

pub async fn room_account_data(
    client: &Client,
    room: &str,
    kind: &str,
    content: Value,
) -> Result<(), Failure> {
    let room = self::room(client, room)?;
    let raw = serde_json::value::to_raw_value(&content).map_err(|_| Failure::Failed)?;
    room.set_account_data_raw(RoomAccountDataEventType::from(kind), Raw::from_json(raw))
        .await
        .map(drop)
        .map_err(|e| http(&e))
}
