//! Presence (N4) on the matrix-sdk engine. Simplified sliding sync carries
//! no presence, and Synapse doesn't count it as syncing; a `/sync` loop
//! beside it would also take this device's to-device messages (room keys)
//! away from the sliding sync. So presence goes through the spec's own
//! endpoints: our status is published with `PUT /presence/{me}/status` often
//! enough that the homeserver keeps this device online (Synapse takes a
//! device offline 30 seconds after it last synced or published), and the
//! people the App shows are read with `GET /presence/{user}/status` about
//! every [`POLL`].
//!
//! Whether the homeserver shares presence at all is learned from our own
//! status read back after publishing it: with presence off, Synapse answers
//! a bare "offline", and other homeservers refuse. Until then, and for good
//! once it's off, the App shows no dots.
use super::{Event, Failure, messaging};
use crate::presence::{Choice, Presence, Status, Support};
use async_channel::{Receiver, Sender};
use matrix_sdk::{
    Client,
    config::RequestConfig,
    ruma::{
        UserId,
        api::{
            client::presence::{get_presence, set_presence},
            error::{ErrorKind, RetryAfter},
        },
        events::{AnyGlobalAccountDataEvent, GlobalAccountDataEventType},
        presence::PresenceState,
        serde::Raw,
    },
};
use serde_json::Value;
use std::{collections::HashMap, time::Duration};

use super::rt::{self, Instant};

/// Our status is published again this often while it doesn't change.
const REFRESH: Duration = Duration::from_secs(20);
/// Each person shown is read again this often.
pub const POLL: Duration = Duration::from_secs(40);
/// Between two reads of other people's presence.
const GAP: Duration = Duration::from_millis(150);
/// Someone whose presence we may not see (no room shared) is asked again.
const HIDDEN: Duration = Duration::from_secs(5 * 60);
/// After the homeserver didn't answer.
const RETRY: Duration = Duration::from_secs(10);
/// One request's limit; the next round asks again.
const TIMEOUT: Duration = Duration::from_secs(15);

/// From the command loop.
pub enum Request {
    Publish(Presence),
    Watch(Vec<String>),
}

/// What the presence task does next.
#[derive(Debug, PartialEq)]
enum Job {
    Publish(Presence),
    /// Read our own status back, to learn whether presence is on.
    Check,
    Read(String),
}

/// How a request ended.
#[derive(Debug, PartialEq)]
enum Outcome<T> {
    Ok(T),
    /// Too many requests: not before this long.
    Limited(Duration),
    /// The homeserver refused (a 4xx answer): presence is off, or not for this person.
    Refused,
    /// No answer, or a server error: try again later.
    Failed,
}

/// The presence task's state, apart from the network.
#[derive(Default)]
struct Schedule {
    support: Support,
    own: Option<Presence>,
    published: Option<(Presence, Instant)>,
    /// No publishing before this (rate limited, or no answer).
    hold: Option<Instant>,
    /// Our own status still needs reading back.
    check: bool,
    watched: Vec<String>,
    due: HashMap<String, Instant>,
    known: HashMap<String, Presence>,
    last_read: Option<Instant>,
}

impl Schedule {
    fn apply(&mut self, request: Request, now: Instant) {
        match request {
            Request::Publish(presence) => self.own = Some(presence),
            Request::Watch(users) => {
                self.due.retain(|user, _| users.contains(user));
                for user in &users {
                    self.due.entry(user.clone()).or_insert(now);
                }
                self.watched = users;
            }
        }
    }

    /// The job due at `now`, or when the next one is (None: nothing until a
    /// request).
    fn next(&self, now: Instant) -> Result<Job, Option<Instant>> {
        if self.support == Support::Off {
            return Err(None);
        }
        let held = |at: Instant| self.hold.map_or(at, |hold| at.max(hold));
        let mut wake: Option<Instant> = None;
        let mut soonest = |at: Instant| wake = Some(wake.map_or(at, |w| w.min(at)));
        if let Some(own) = &self.own {
            let at = match &self.published {
                Some((sent, when)) if sent == own => held(*when + REFRESH),
                _ => held(now),
            };
            if at <= now {
                return Ok(Job::Publish(own.clone()));
            }
            soonest(at);
        }
        if self.check {
            let at = held(now);
            if at <= now {
                return Ok(Job::Check);
            }
            soonest(at);
        }
        let next = self
            .watched
            .iter()
            .filter_map(|user| Some((self.due.get(user)?, user)))
            .min_by_key(|(at, _)| **at);
        if let Some((at, user)) = next {
            let at = self.last_read.map_or(*at, |last| (*at).max(last + GAP));
            if at <= now {
                return Ok(Job::Read(user.clone()));
            }
            soonest(at);
        }
        Err(wake)
    }

    /// A publish ended. Returns the support learned, if it changed.
    fn published(&mut self, sent: Presence, outcome: Outcome<()>, now: Instant) -> Option<Support> {
        match outcome {
            Outcome::Ok(()) => {
                // Our own status reads back only when it isn't offline:
                // Invisible tells nothing.
                self.check = self.support == Support::Unknown && sent.status != Status::Offline;
                self.published = Some((sent, now));
                self.hold = None;
                None
            }
            Outcome::Limited(wait) => {
                self.hold = Some(now + wait);
                None
            }
            // Not again at once if presence turns out to be on after all.
            Outcome::Refused => {
                self.hold = Some(now + RETRY);
                self.learn(Support::Off)
            }
            Outcome::Failed => {
                self.hold = Some(now + RETRY);
                None
            }
        }
    }

    /// Our own status read back.
    fn checked(&mut self, outcome: Outcome<Reading>, now: Instant) -> Option<Support> {
        match outcome {
            Outcome::Ok(reading) => {
                self.check = false;
                self.learn(if reading.tells() {
                    Support::On
                } else {
                    Support::Off
                })
            }
            Outcome::Refused => {
                self.check = false;
                self.learn(Support::Off)
            }
            Outcome::Limited(wait) => {
                self.hold = Some(now + wait);
                None
            }
            Outcome::Failed => {
                self.hold = Some(now + RETRY);
                None
            }
        }
    }

    /// Someone's presence read. Returns it if it changed, and the support
    /// learned (a reading that tells something means presence is on).
    fn read(
        &mut self,
        user: &str,
        outcome: Outcome<Reading>,
        now: Instant,
    ) -> (Option<Presence>, Option<Support>) {
        self.last_read = Some(now);
        let (again, reading) = match outcome {
            Outcome::Ok(reading) => (POLL, Some(reading)),
            Outcome::Limited(wait) => (wait.max(GAP), None),
            Outcome::Refused => (HIDDEN, None),
            Outcome::Failed => (RETRY, None),
        };
        if let Some(due) = self.due.get_mut(user) {
            *due = now + again;
        }
        let Some(reading) = reading else {
            return (None, None);
        };
        let support = if reading.tells() {
            self.learn(Support::On)
        } else {
            None
        };
        let presence = reading.presence;
        let changed = self.known.get(user) != Some(&presence);
        if changed {
            self.known.insert(user.to_owned(), presence.clone());
        }
        (changed.then_some(presence), support)
    }

    fn learn(&mut self, support: Support) -> Option<Support> {
        if self.support == support || self.support == Support::On {
            return None;
        }
        self.support = support;
        if support == Support::Off {
            self.check = false;
        }
        Some(support)
    }
}

/// A presence answer, with whether it says more than a bare "offline".
#[derive(Debug, PartialEq)]
struct Reading {
    presence: Presence,
    /// It carries a last activity, a current one or a status message.
    detail: bool,
}
impl Reading {
    /// Whether this shows the homeserver shares presence: a homeserver with
    /// presence off answers a bare "offline" for everyone.
    fn tells(&self) -> bool {
        self.detail || self.presence.status != Status::Offline
    }
}

fn outcome<T>(e: &matrix_sdk::HttpError) -> Outcome<T> {
    if let Some(ErrorKind::LimitExceeded(data)) = e.client_api_error_kind() {
        let wait = match data.retry_after {
            Some(RetryAfter::Delay(d)) => d,
            _ => RETRY,
        };
        return Outcome::Limited(wait);
    }
    match e.as_client_api_error().map(|e| e.status_code.as_u16()) {
        Some(400..=499) => Outcome::Refused,
        _ => Outcome::Failed,
    }
}

fn config() -> RequestConfig {
    RequestConfig::default().disable_retry().timeout(TIMEOUT)
}

async fn publish(client: &Client, presence: &Presence) -> Outcome<()> {
    let Some(me) = client.user_id() else {
        return Outcome::Failed;
    };
    let mut request =
        set_presence::v3::Request::new(me.to_owned(), PresenceState::from(presence.status.wire()));
    request.status_msg = presence.message.clone();
    match client.send(request).with_request_config(config()).await {
        Ok(_) => Outcome::Ok(()),
        Err(e) => outcome(&e),
    }
}

async fn read(client: &Client, user: &str) -> Outcome<Reading> {
    let Ok(user) = UserId::parse(user) else {
        return Outcome::Refused;
    };
    match client
        .send(get_presence::v3::Request::new(user))
        .with_request_config(config())
        .await
    {
        Ok(answer) => {
            #[allow(deprecated)]
            let message = answer.status_msg.filter(|m| !m.trim().is_empty());
            #[allow(deprecated)]
            let active = answer.currently_active;
            Outcome::Ok(Reading {
                detail: message.is_some() || active.is_some() || answer.last_active_ago.is_some(),
                presence: Presence {
                    status: Status::from_wire(answer.presence.as_str()),
                    message,
                },
            })
        }
        Err(e) => outcome(&e),
    }
}

/// Publish our status and read the people the App shows, until the command
/// loop goes away. First the App forgets the last session's presence, then
/// learns our saved status, before which it publishes nothing.
pub async fn run(client: Client, events: Sender<Event>, requests: Receiver<Request>) {
    let _ = events.send(Event::PresenceSupport(Support::Unknown)).await;
    saved(client.clone(), events.clone()).await;
    let mut schedule = Schedule::default();
    loop {
        let job = match schedule.next(Instant::now()) {
            Ok(job) => job,
            Err(wake) => {
                let request = match wake {
                    Some(at) => {
                        let wait = at.saturating_duration_since(Instant::now());
                        match rt::timeout(wait, requests.recv()).await {
                            Ok(Ok(request)) => Some(request),
                            Ok(Err(_)) => return,
                            Err(_) => None,
                        }
                    }
                    None => match requests.recv().await {
                        Ok(request) => Some(request),
                        Err(_) => return,
                    },
                };
                if let Some(request) = request {
                    schedule.apply(request, Instant::now());
                }
                continue;
            }
        };
        // Newer requests first: a status changed meanwhile goes out instead.
        while let Ok(request) = requests.try_recv() {
            schedule.apply(request, Instant::now());
        }
        if schedule.next(Instant::now()).as_ref() != Ok(&job) {
            continue;
        }
        let learned = match job {
            Job::Publish(presence) => {
                let outcome = publish(&client, &presence).await;
                schedule.published(presence, outcome, Instant::now())
            }
            Job::Check => {
                let me = client.user_id().map(ToString::to_string);
                let outcome = match me {
                    Some(me) => read(&client, &me).await,
                    None => Outcome::Failed,
                };
                schedule.checked(outcome, Instant::now())
            }
            Job::Read(user) => {
                let outcome = read(&client, &user).await;
                let (changed, learned) = schedule.read(&user, outcome, Instant::now());
                if let Some(presence) = changed {
                    let _ = events.send(Event::Presence(vec![(user, presence)])).await;
                }
                learned
            }
        };
        if let Some(support) = learned {
            let _ = events.send(Event::PresenceSupport(support)).await;
        }
    }
}

/// Save our choice and custom status for all our devices.
pub async fn save(client: &Client, choice: Choice, message: &str) -> Result<(), Failure> {
    messaging::set_preference(
        client,
        crate::presence::SAVED,
        crate::presence::saved(choice, message),
    )
    .await
}

/// Report our saved choice and custom status, now and whenever it syncs.
async fn saved(client: Client, events: Sender<Event>) {
    let kind = GlobalAccountDataEventType::from(crate::presence::SAVED);
    // The homeserver's copy first: on a new device the store may not have
    // synced it yet, and an Invisible account mustn't show online meanwhile.
    let account = client.account();
    let content = match account.fetch_account_data(kind.clone()).await {
        Ok(content) => content,
        Err(_) => account.account_data_raw(kind).await.ok().flatten(),
    };
    let content = content
        .and_then(|raw| raw.deserialize_as::<Value>().ok())
        .unwrap_or(Value::Null);
    let (choice, message) = crate::presence::read_saved(&content);
    let _ = events.send(Event::OwnStatus { choice, message }).await;
    // The handler lives as long as the client, i.e. this session.
    client.add_event_handler(move |raw: Raw<AnyGlobalAccountDataEvent>| {
        let events = events.clone();
        async move {
            let Ok(value) = raw.deserialize_as::<Value>() else {
                return;
            };
            if value["type"] == crate::presence::SAVED {
                let (choice, message) = crate::presence::read_saved(&value["content"]);
                let _ = events.send(Event::OwnStatus { choice, message }).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online() -> Presence {
        Presence {
            status: Status::Online,
            message: Some("At the workshop".into()),
        }
    }
    fn reading(status: Status, detail: bool) -> Outcome<Reading> {
        Outcome::Ok(Reading {
            presence: Presence {
                status,
                message: None,
            },
            detail,
        })
    }

    #[test]
    fn our_status_is_published_then_kept_alive_and_read_back_once() {
        let t = Instant::now();
        let mut s = Schedule::default();
        assert_eq!(s.next(t), Err(None));
        s.apply(Request::Publish(online()), t);
        assert_eq!(s.next(t), Ok(Job::Publish(online())));
        assert_eq!(s.published(online(), Outcome::Ok(()), t), None);
        assert_eq!(s.next(t), Ok(Job::Check));
        assert_eq!(
            s.checked(reading(Status::Online, true), t),
            Some(Support::On)
        );
        // Then again before the homeserver would take the device offline.
        assert_eq!(s.next(t), Err(Some(t + REFRESH)));
        assert_eq!(s.next(t + REFRESH), Ok(Job::Publish(online())));
        // A change goes out at once.
        let idle = Presence {
            status: Status::Idle,
            message: None,
        };
        s.apply(Request::Publish(idle.clone()), t + Duration::from_secs(1));
        assert_eq!(s.next(t + Duration::from_secs(1)), Ok(Job::Publish(idle)));
    }

    #[test]
    fn a_bare_offline_or_a_refusal_means_presence_is_off_and_everything_stops() {
        let t = Instant::now();
        let mut s = Schedule::default();
        s.apply(Request::Publish(online()), t);
        s.apply(Request::Watch(vec!["@mira:x".into()]), t);
        s.published(online(), Outcome::Ok(()), t);
        // Synapse with presence off: our own status reads back as a bare offline.
        assert_eq!(
            s.checked(reading(Status::Offline, false), t),
            Some(Support::Off)
        );
        assert_eq!(s.next(t + HIDDEN), Err(None));

        let mut s = Schedule::default();
        s.apply(Request::Publish(online()), t);
        assert_eq!(
            s.published(online(), Outcome::Refused, t),
            Some(Support::Off)
        );
        assert_eq!(s.next(t + HIDDEN), Err(None));
    }

    #[test]
    fn invisible_learns_from_other_people_instead() {
        let t = Instant::now();
        let mut s = Schedule::default();
        let invisible = Presence {
            status: Status::Offline,
            message: None,
        };
        s.apply(Request::Publish(invisible.clone()), t);
        s.apply(Request::Watch(vec!["@mira:x".into(), "@jo:x".into()]), t);
        s.published(invisible, Outcome::Ok(()), t);
        assert_eq!(s.next(t), Ok(Job::Read("@mira:x".into())));
        // A bare offline tells nothing yet.
        let (changed, learned) = s.read("@mira:x", reading(Status::Offline, false), t);
        assert_eq!(learned, None);
        assert_eq!(changed.map(|p| p.status), Some(Status::Offline));
        assert_eq!(s.next(t), Err(Some(t + GAP)));
        let (_, learned) = s.read("@jo:x", reading(Status::Idle, true), t + GAP);
        assert_eq!(learned, Some(Support::On));
    }

    #[test]
    fn people_shown_are_read_in_turn_and_again_after_the_poll() {
        let t = Instant::now();
        let mut s = Schedule {
            support: Support::On,
            ..Default::default()
        };
        s.apply(Request::Watch(vec!["@mira:x".into(), "@jo:x".into()]), t);
        assert_eq!(s.next(t), Ok(Job::Read("@mira:x".into())));
        let (changed, _) = s.read("@mira:x", reading(Status::Idle, true), t);
        assert_eq!(changed.map(|p| p.status), Some(Status::Idle));
        // Spaced out, then the next person.
        assert_eq!(s.next(t), Err(Some(t + GAP)));
        assert_eq!(s.next(t + GAP), Ok(Job::Read("@jo:x".into())));
        // No room shared: asked again much later.
        s.read("@jo:x", Outcome::Refused, t + GAP);
        assert_eq!(s.next(t + GAP * 2), Err(Some(t + POLL)));
        // The same answer again isn't news.
        let (changed, _) = s.read("@mira:x", reading(Status::Idle, true), t + POLL);
        assert_eq!(changed, None);
        // Someone no longer shown isn't read; someone new is, at once.
        s.apply(Request::Watch(vec!["@sam:x".into()]), t + POLL);
        assert_eq!(s.next(t + POLL + GAP), Ok(Job::Read("@sam:x".into())));
    }

    #[test]
    fn a_rate_limit_holds_publishing_back() {
        let t = Instant::now();
        let mut s = Schedule::default();
        s.apply(Request::Publish(online()), t);
        s.published(online(), Outcome::Limited(Duration::from_secs(7)), t);
        assert_eq!(s.next(t), Err(Some(t + Duration::from_secs(7))));
        s.published(online(), Outcome::Failed, t);
        assert_eq!(s.next(t), Err(Some(t + RETRY)));
    }
}
