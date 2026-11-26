//! The room-wide voice slot is entered explicitly, even by its first member.
//! Its lifetime is local: a remote unpublish/leave stops only that peer's audio.
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokio::task::AbortHandle;

#[derive(Clone, PartialEq)]
struct Recipient {
    user: String,
    device: String,
    member: String,
    created: u64,
}
type Roster = BTreeMap<String, Recipient>;

pub(super) fn check_room(state: &[Value], user: &str) -> Result<()> {
    let has = |kind: &str| {
        state
            .iter()
            .find(|e| e["type"] == kind && e["state_key"] == "")
    };
    if !has("m.room.create").is_some_and(|e| e["content"]["type"] == crate::rooms::VOICE_ROOM)
        || !has("m.room.encryption")
            .is_some_and(|e| e["content"]["algorithm"] == "m.megolm.v1.aes-sha2")
        || !state.iter().any(|e| {
            e["type"] == "m.room.member"
                && e["state_key"] == user
                && e["content"]["membership"] == "join"
        })
    {
        return Err("Join an encrypted voice room before connecting.");
    }
    Ok(())
}

fn roster(state: &[Value], local: &str, service: &Url) -> Roster {
    let joined: BTreeSet<_> = state
        .iter()
        .filter(|e| e["type"] == "m.room.member" && e["content"]["membership"] == "join")
        .filter_map(|e| e["state_key"].as_str())
        .collect();
    state
        .iter()
        .filter(|e| active_member(e))
        .filter_map(|e| {
            let user = e["sender"].as_str()?;
            let c = &e["content"];
            let device = c["device_id"].as_str()?;
            let identity = format!("{user}:{device}");
            // A room member can write state with arbitrary content. Do not use
            // another user's claimed identity or expand a to-device wildcard.
            if identity == local
                || !joined.contains(user)
                || device.is_empty()
                || device == "*"
                || e["state_key"] != format!("_{user}_{device}_m.call")
                || !crate::rooms::items(&c["foci_preferred"]).any(|f| {
                    f["type"] == "livekit"
                        && f["livekit_service_url"].as_str().is_some_and(|u| {
                            u.trim_end_matches('/') == service.as_str().trim_end_matches('/')
                        })
                })
            {
                return None;
            }
            let member = c["membershipID"].as_str().unwrap_or(&identity).to_owned();
            Some((
                identity,
                Recipient {
                    user: user.into(),
                    device: device.into(),
                    member,
                    created: created(e)?,
                },
            ))
        })
        .collect()
}

struct GroupKeys {
    provider: KeyProvider,
    local: String,
    outbound: SenderKey,
    members: Roster,
    ready: BTreeSet<String>,
    /// The members may not have the current outbound key yet.
    unsent: bool,
}
impl GroupKeys {
    /// A fresh provider with a fresh key at `index`. A rejoin starts past the
    /// previous index, so a key never reuses one the peers may still hold.
    fn new(session: &Session, index: u8) -> Result<Self> {
        let local = session.identity();
        let outbound = SenderKey::fresh(index)?;
        let provider = rtc_keys::provider();
        if !provider.set_key(&local.clone().into(), index.into(), outbound.bytes.to_vec()) {
            return Err("Cannot install voice room key.");
        }
        recovery::trace_key(index);
        Ok(Self {
            provider,
            local,
            outbound,
            members: Roster::new(),
            ready: BTreeSet::new(),
            unsent: false,
        })
    }
    fn next_index(&self) -> u8 {
        (self.outbound.index + 1) % 255
    }
    async fn reconcile<S: CallSignal>(
        &mut self,
        w: &mut Wire<'_, S>,
        s: &Session,
        next: Roster,
        lk: &Room,
        mic: &LocalTrackPublication,
    ) -> Result<()> {
        if next == self.members {
            return Ok(());
        }
        self.ready
            .retain(|id| next.contains_key(id) && next.get(id) == self.members.get(id));
        // Fresh, independent key for each roster change, including departures.
        // Select it on the existing publication *before* distributing it: do
        // not publish new frames for a departed device with the old key.
        let key = SenderKey::fresh(self.next_index())?;
        let local = lk.local_participant().identity();
        let cryptors = lk.e2ee_manager().frame_cryptors();
        if !cryptors.contains_key(&(local.clone(), mic.sid()))
            || !self
                .provider
                .set_key(&local, key.index.into(), key.bytes.to_vec())
        {
            return Err("Cannot rotate voice room key.");
        }
        select_key(lk, key.index)?;
        recovery::trace_key(key.index);
        self.outbound = key;
        self.members = next;
        self.unsent = true;
        self.flush(w, s).await;
        Ok(())
    }
    /// Send the current key to every member. A failure (offline, or a device
    /// whose keys are unknown yet) is retried on the next heartbeat.
    async fn flush<S: CallSignal>(&mut self, w: &mut Wire<'_, S>, s: &Session) {
        if !self.unsent {
            return;
        }
        let targets: Vec<(String, String)> = self
            .members
            .values()
            .filter(|p| !p.device.is_empty() && p.device != "*")
            .map(|p| (p.user.clone(), p.device.clone()))
            .collect();
        if targets.is_empty() {
            self.unsent = false;
            return;
        }
        let content = self.outbound.content(w.room, &self.local, &s.device);
        self.unsent = !matches!(
            time::timeout(SECOND * 5, w.signal.send_to_device(KEY, content, &targets)).await,
            Ok(Ok(()))
        );
    }
    fn receive<S>(&mut self, w: &mut Wire<'_, S>) -> Result<()> {
        let mut early = Vec::new();
        for (received, e) in w.keys.drain(..) {
            let c = &e.content;
            let user = e.sender.as_str();
            let Some(device) = c["member"]["claimed_device_id"].as_str() else {
                continue;
            };
            let id = format!("{user}:{device}");
            let Some(peer) = self.members.get(&id) else {
                // The Olm sync can beat the matching room-state sync.
                if received.elapsed() < SECOND * 8 {
                    early.push((received, e))
                }
                continue;
            };
            if peer.user != user || peer.device != device {
                continue;
            }
            if let Some((index, bytes)) = rtc_keys::receive(c, device, &peer.member)? {
                if !self.provider.set_key(&id.clone().into(), index, bytes) {
                    return Err("Cannot install participant key.");
                }
                self.ready.insert(id);
            }
        }
        w.keys = early;
        Ok(())
    }
}

#[derive(Default)]
struct Subscriptions {
    jobs: JoinSet<Result<()>>,
    tracks: HashMap<TrackSid, AbortHandle>,
}
impl Subscriptions {
    /// Stop all playback, as the SFU connection drops.
    fn stop(&mut self) {
        self.jobs.abort_all();
        self.tracks.clear();
    }
    /// The track behind `sid` was (re)subscribed: play the current one.
    fn forget(&mut self, sid: &TrackSid) {
        if let Some(job) = self.tracks.remove(sid) {
            job.abort();
        }
    }
    fn sync(
        &mut self,
        lk: &Room,
        keys: &GroupKeys,
        mixer: &Mixer,
        notices: &DeviceNotices,
        previews: &Previews,
    ) -> Result<BTreeSet<String>> {
        let mut live = BTreeSet::new();
        let mut speaking = BTreeSet::new();
        for (id, p) in lk.remote_participants() {
            // Speaker notifications may be deltas. Use each device's current
            // flag, and attribute only admitted, encrypted microphone tracks.
            let active = p.is_speaking();
            for (sid, publication) in p.track_publications() {
                let wanted = keys.ready.contains(&id.0)
                    && matches!(
                        (publication.kind(), publication.source()),
                        (
                            TrackKind::Audio,
                            TrackSource::Microphone | TrackSource::ScreenshareAudio
                        ) | (
                            TrackKind::Video,
                            TrackSource::Screenshare | TrackSource::Camera
                        )
                    );
                if wanted && publication.encryption_type() != EncryptionType::Gcm {
                    return Err("Unencrypted voice room track.");
                }
                if publication.is_desired() != wanted {
                    publication.set_subscribed(wanted);
                }
                if wanted && let Some(track) = publication.track() {
                    if active
                        && publication.source() == TrackSource::Microphone
                        && !publication.is_muted()
                    {
                        speaking.insert(keys.members[&id.0].user.clone());
                    }
                    live.insert(sid.clone());
                    self.tracks.entry(sid).or_insert_with(|| match track {
                        RemoteTrack::Audio(track) => self.jobs.spawn(mixer.play(
                            track,
                            publication.source().into(),
                            Some(keys.members[&id.0].user.clone()),
                        )),
                        RemoteTrack::Video(track) => self.jobs.spawn(room_video::display(
                            track,
                            keys.members[&id.0].user.clone(),
                            if publication.source() == TrackSource::Camera {
                                VideoInput::Camera
                            } else {
                                VideoInput::Window
                            },
                            previews.clone(),
                            notices.events.clone(),
                        )),
                    });
                }
            }
        }
        self.tracks.retain(|sid, job| {
            if live.contains(sid) {
                true
            } else {
                job.abort();
                false
            }
        });
        Ok(speaking)
    }
}

/// The voice room's lifetime: one LiveKit session after another, rejoining
/// whenever one is lost, until the user leaves.
pub(super) async fn joined<S: CallSignal>(
    w: &mut Wire<'_, S>,
    s: &mut Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Media,
    mode: watch::Receiver<AudioMode>,
    views: GroupViews,
) -> Result<&'static str> {
    let mut keys = GroupKeys::new(s, 0)?;
    let mut lease = LeaseWatch::new();
    let mut rejoin: Option<Rejoin> = None;
    loop {
        if let Some(r) = &mut rejoin {
            r.wait(w)
                .await
                .map_err(|_| "Could not reconnect to the voice room.")?;
            let user = s.user.clone();
            let step = r
                .attempt(w, s, m, &mut lease, false, |state| check_room(state, &user))
                .await?;
            if let Step::Ready { fresh } = step {
                recovery::trace(format_args!("RTC call rejoined fresh={fresh}"));
            }
            match step {
                Step::Retry => continue,
                // Peers see a new member, rotate, and send us their keys.
                Step::Ready { fresh: true } => keys = GroupKeys::new(s, keys.next_index())?,
                Step::Ready { fresh: false } => (),
            }
        }
        let link = match publish_mic(s, &keys.provider, &mut media.live, false).await {
            Ok(link) => link,
            Err(why) if rejoin.is_none() => return Err(why),
            Err(_) => {
                close(&mut media.live).await;
                continue;
            }
        };
        let rejoined = rejoin.take().is_some();
        let lost = live(
            w, s, m, events, link, &mut keys, &mut lease, &mode, &views, rejoined,
        )
        .await?;
        recovery::trace(format_args!("RTC call lost={lost:?}"));
        close(&mut media.live).await;
        if views.speakers.send_if_modified(|shown| {
            let changed = !shown.is_empty();
            shown.clear();
            changed
        }) {
            let _ = events.send(json!({"command":"voice_redraw"})).await;
        }
        status(events, w.room, "Reconnecting…").await;
        rejoin = Some(Rejoin::new(lost));
    }
}

/// One LiveKit session of a voice room. It rides out the SDK's own
/// reconnects and ends when the room or our membership is lost.
#[allow(clippy::too_many_arguments)]
async fn live<S: CallSignal>(
    w: &mut Wire<'_, S>,
    s: &Session,
    m: &mut Membership,
    events: &Sender<Value>,
    link: (
        &Room,
        tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
        NativeAudioSource,
        LocalTrackPublication,
    ),
    keys: &mut GroupKeys,
    lease: &mut LeaseWatch,
    mode: &watch::Receiver<AudioMode>,
    views: &GroupViews,
    rejoined: bool,
) -> Result<Lost> {
    let (lk, mut rtc, src, mut mic) = link;
    let mut mode = mode.clone();
    // A rejoin's key is not at the cryptor's default index.
    select_key(lk, keys.outbound.index)?;
    // The members may have rotated while we were away; they need ours too.
    keys.unsent |= rejoined;
    let notices = DeviceNotices {
        events: events.clone(),
        room: w.room.to_owned(),
    };
    let devices = mode.clone();
    // Keep capture paced by the audio device while membership and key RPCs
    // are in flight. Dropping the task set releases its pacat child; every
    // peer plays through the one mixer output.
    let io = audio::platform();
    let mixer = Mixer::start(io.clone(), mode.clone(), notices.clone());
    let capture = |mic: &LocalTrackPublication| {
        controlled_capture(
            io.clone(),
            src.clone(),
            mic.clone(),
            devices.clone(),
            notices.clone(),
        )
    };
    let mut input = JoinSet::new();
    input.spawn(capture(&mic));
    let mut subscriptions = Subscriptions::default();
    let mut share = room_video::Publisher::new(VideoInput::Window);
    let mut sound = share_audio::Publisher::default();
    let mut camera = room_video::Publisher::new(VideoInput::Camera);
    share.local_preview = Some(views.previews.clone());
    camera.local_preview = Some(views.previews.clone());
    let mut heartbeat = time::interval(SECOND * 4);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    #[cfg(debug_assertions)]
    let trace_video = room_video::trace_video_enabled();
    let mut label = String::new();
    // Between the SDK's Reconnecting and Reconnected.
    let mut online = true;
    w.roster_dirty = true;
    loop {
        if lease.lost() {
            return Ok(Lost::Lease);
        }
        // Roster changes and keys can arrive alongside request answers.
        w.absorb();
        if online && std::mem::take(&mut w.roster_dirty) {
            // Offline, the next heartbeat asks again.
            if let Ok(Ok(state)) = time::timeout(SECOND * 5, w.state(false)).await {
                check_room(&state, &s.user)?;
                if !recovery::ours(&state, &s.user, &m.event_id) {
                    return Ok(Lost::Lease);
                }
                keys.reconcile(w, s, roster(&state, &keys.local, &s.focus), lk, &mic)
                    .await?;
                let next = format!("In voice · {} connected", keys.members.len() + 1);
                if label != next {
                    status(events, w.room, &next).await;
                    label = next;
                }
            }
        }
        keys.receive(w)?;
        let mut speaking = BTreeSet::new();
        if online {
            speaking = subscriptions.sync(lk, keys, &mixer, &notices, &views.previews)?;
            if lk.local_participant().is_speaking() && !mic.is_muted() && !mode.borrow().mic_off() {
                speaking.insert(s.user.clone());
            }
        }
        if views.speakers.send_if_modified(|shown| {
            if *shown == speaking {
                return false;
            }
            *shown = speaking;
            true
        }) {
            let _ = events.send(json!({"command":"voice_redraw"})).await;
        }
        if online {
            // Publishing waits for the SDK's reconnect; do it once it is back.
            let (requests, route) = {
                let m = mode.borrow_and_update();
                (m.video, m.camera.clone())
            };
            share
                .select(
                    requests[VideoInput::Window as usize],
                    lk,
                    CameraChoice::default(),
                )
                .await?;
            camera
                .select(requests[VideoInput::Camera as usize], lk, route)
                .await?;
            sound
                .sync(
                    share.live_capture(),
                    lk,
                    keys.outbound.index,
                    &mode,
                    events,
                    w.room,
                )
                .await?;
        }
        tokio::select! {
            changed=mode.changed()=>{changed.map_err(|_|ENDED)?;}
            update=share.next()=>share.update(update,lk,keys.outbound.index,events,w.room,&mode).await?,
            ended=sound.ended()=>sound.finish(ended,lk,events,w.room).await?,
            update=camera.next()=>camera.update(update,lk,keys.outbound.index,events,w.room,&mode).await?,
            r=input.join_next()=>return Err(r.and_then(|j|j.ok()).and_then(|v|v.err()).unwrap_or("Microphone stream ended.")),
            job=subscriptions.jobs.join_next(),if !subscriptions.jobs.is_empty()=>match job {
                Some(Ok(result))=>result?,
                Some(Err(e)) if e.is_cancelled()=>(),
                _=>return Err("Voice room playback failed."),
            },
            frame=w.pump()=>{frame?;}
            // Notice the end of our lease as it happens, not at the next tick.
            _=time::sleep_until(lease.deadline())=>(),
            _=heartbeat.tick()=>{
                #[cfg(debug_assertions)]
                if trace_video && online {room_video::trace_video_stats(lk).await;}
                lease.keep(w, m).await;
                if online {keys.flush(w, s).await;}
                w.roster_dirty=true;
            }
            e=rtc.recv()=>match e {
                Some(RoomEvent::Reconnecting)=>{
                    // Keep the room, keys, membership and microphone; only
                    // playback stops until the SDK is back.
                    online=false;
                    subscriptions.stop();
                    label.clear();
                    status(events,w.room,"Reconnecting…").await;
                }
                Some(RoomEvent::Reconnected)=>{
                    online=true;
                    if let Some(current)=recovery::local_mic(lk) && current.sid()!=mic.sid() {
                        mic=current;
                        input.shutdown().await;
                        input.spawn(capture(&mic));
                    }
                    // A restart republished our tracks with cryptors at index 0.
                    if select_key(lk,keys.outbound.index).is_err() {return Ok(Lost::Media);}
                    keys.unsent=true;
                    w.roster_dirty=true;
                    heartbeat.reset_immediately();
                }
                Some(RoomEvent::LocalTrackRepublished{previous_sid,publication,..})=>{
                    if previous_sid==mic.sid() {
                        mic=publication.clone();
                        input.shutdown().await;
                        input.spawn(capture(&mic));
                    }
                    share.rebind(&previous_sid,&publication);
                    camera.rebind(&previous_sid,&publication);
                    sound.rebind(&previous_sid,&publication);
                    if select_one(lk,&publication,keys.outbound.index).is_err() {return Ok(Lost::Media);}
                }
                // A resubscribed track replaces the one a playback job holds.
                Some(RoomEvent::TrackSubscribed{publication,..})=>subscriptions.forget(&publication.sid()),
                Some(RoomEvent::Disconnected{..})|None=>return Ok(Lost::Media),
                // Remote joins, subscriptions and leaves are reconciled independently.
                _=>(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_only_go_to_joined_users_active_devices_in_this_slot() {
        let focus = Url::parse("https://rtc.example/").unwrap();
        let user = "@other:example";
        let joined =
            json!({"type":"m.room.member","state_key":user,"content":{"membership":"join"}});
        let device = |d: &str| {
            json!({"type":EVENT_TYPE,"sender":user,"origin_server_ts":now(),
            "state_key":format!("_{user}_{d}_m.call"),"content":{"application":"m.call","scope":"m.room",
            "call_id":"","device_id":d,"expires":60000,"foci_preferred":[{"type":"livekit","livekit_service_url":focus.as_str()}]}})
        };
        let mut expired = device("E");
        expired["origin_server_ts"] = 1.into();
        let mut wrong_slot = device("W");
        wrong_slot["content"]["call_id"] = "another".into();
        let mut spoofed = device("S");
        spoofed["state_key"] = "_@someone:example_S_m.call".into();
        let mut unjoined = device("X");
        unjoined["sender"] = "@left:example".into();
        unjoined["state_key"] = "_@left:example_X_m.call".into();
        let mut other_sfu = device("F");
        other_sfu["content"]["foci_preferred"][0]["livekit_service_url"] =
            "https://elsewhere.example/".into();
        let state = vec![
            joined,
            device("A"),
            device("B"),
            device("*"),
            expired,
            wrong_slot,
            spoofed,
            unjoined,
            other_sfu,
        ];
        let recipients = roster(&state, "@me:example:D", &focus);
        assert_eq!(recipients.len(), 2);
        let key = SenderKey::fresh(0).unwrap();
        let cmd = key.to_devices(
            "!room",
            "@me:example:D",
            "D",
            recipients
                .values()
                .map(|p| (p.user.as_str(), p.device.as_str())),
        );
        assert_eq!(cmd["encrypted"], true);
        assert_eq!(cmd["messages"].as_object().unwrap().len(), 1);
        assert_eq!(
            cmd["messages"][user]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["A", "B"])
        );
    }
}
