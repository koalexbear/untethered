//! Calls that outlive a network loss, for voice rooms and DMs alike.
//!
//! A short outage keeps the membership and the LiveKit room: the SDK resumes
//! or restarts its own connection. Once the MSC4140 delayed leave may have
//! fired, the call rejoins with a new membership (fresh `created_ts` and
//! sender key) and a new SFU token. It never revives the expired membership.
use super::*;

/// How long a lost call keeps trying to rejoin.
pub(super) const REJOIN_WINDOW: Duration = Duration::from_secs(300);
const RENEW_EVERY: Duration = Duration::from_secs(36);

/// Why a media session ended without the user leaving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lost {
    /// The SFU connection is gone. The membership may still be valid.
    Media,
    /// The delayed leave fired, or may have: the membership must be replaced.
    Lease,
}

/// Our membership's lease: the homeserver sends the delayed leave [`LEASE`]
/// after the last heartbeat that reached it.
pub(super) struct LeaseWatch {
    confirmed: Instant,
    renewed: Instant,
}
impl LeaseWatch {
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            confirmed: now,
            renewed: now,
        }
    }
    /// No heartbeat was confirmed for a whole lease: the leave may have fired.
    pub(super) fn lost(&self) -> bool {
        self.confirmed.elapsed() >= LEASE
    }
    /// When the lease runs out unless a heartbeat is confirmed first.
    pub(super) fn deadline(&self) -> Instant {
        self.confirmed + LEASE
    }
    /// Restart the delayed leave, and renew the absolute expiry every 36 s.
    /// A failure is not an error: [`lost`](Self::lost) decides when it is.
    pub(super) async fn keep<S: CallSignal>(
        &mut self,
        w: &mut Wire<'_, S>,
        m: &mut Membership,
    ) -> bool {
        // Never replace a membership whose leave may have fired, or that has
        // no scheduled leave: a renewal would revive it with its old content.
        if self.lost() || !m.armed() {
            return false;
        }
        let started = Instant::now();
        // Late replies have unique request IDs; a timeout is just a miss.
        if !matches!(
            time::timeout(SECOND * 2, m.heartbeat(&mut w.signal)).await,
            Ok(Ok(()))
        ) {
            return false;
        }
        self.confirmed = started;
        // Renew only right after a heartbeat got through: a renewal cut short
        // after cancelling the old leave would leave no lease at all.
        if self.renewed.elapsed() > RENEW_EVERY
            && matches!(
                time::timeout(SECOND * 8, m.renew(&mut w.signal)).await,
                Ok(Ok(()))
            )
        {
            self.renewed = started;
        }
        true
    }
}

/// `user`'s membership event `event_id` is still active in the room state.
pub(super) fn ours(state: &[Value], user: &str, event_id: &str) -> bool {
    state
        .iter()
        .any(|e| e["sender"] == user && e["event_id"] == event_id && active_member(e))
}

/// One attempt's outcome.
pub(super) enum Step {
    /// Not yet (offline, or the homeserver or SFU did not answer).
    Retry,
    /// Signaling is ready for a new LiveKit room. `fresh`: the membership was
    /// replaced, so the caller starts a fresh sender key at a new index.
    Ready { fresh: bool },
}

/// Rejoining after [`Lost`], with backoff, for [`REJOIN_WINDOW`].
pub(super) struct Rejoin {
    since: Instant,
    attempts: u32,
    fresh: bool,
}
impl Rejoin {
    pub(super) fn new(lost: Lost) -> Self {
        Self {
            since: Instant::now(),
            attempts: 0,
            fresh: lost == Lost::Lease,
        }
    }

    /// Wait out the backoff before the next attempt (none before the first),
    /// still collecting call signals. Fails once the window is over.
    pub(super) async fn wait<S: CallSignal>(&mut self, w: &mut Wire<'_, S>) -> Result<()> {
        if self.since.elapsed() >= REJOIN_WINDOW {
            return Err("Could not reconnect to the call.");
        }
        let delay = SECOND * [0, 1, 2, 4, 5][self.attempts.min(4) as usize];
        self.attempts += 1;
        let until = (Instant::now() + delay).min(self.since + REJOIN_WINDOW);
        loop {
            tokio::select! {
                _ = time::sleep_until(until) => return Ok(()),
                r = w.pump() => r?,
            }
        }
    }

    /// Check the room, replace the membership if its lease is gone, and get a
    /// new SFU token. `check` rejects room state the call cannot continue in;
    /// its error ends the call.
    pub(super) async fn attempt<S: CallSignal>(
        &mut self,
        w: &mut Wire<'_, S>,
        s: &mut Session,
        m: &mut Membership,
        lease: &mut LeaseWatch,
        video: bool,
        check: impl Fn(&[Value]) -> Result<()>,
    ) -> Result<Step> {
        if !self.fresh && !lease.keep(w, m).await && lease.lost() {
            self.fresh = true;
        }
        let Ok(Ok(state)) = time::timeout(SECOND * 5, w.state(true)).await else {
            return Ok(Step::Retry);
        };
        check(&state)?;
        let still = ours(&state, &s.user, &m.event_id);
        if !still {
            self.fresh = true;
        }
        let fresh = self.fresh;
        if fresh {
            // A leave that has not fired yet must not end the new membership.
            if still
                && !matches!(
                    time::timeout(SECOND * 3, m.retire(&mut w.signal)).await,
                    Ok(Ok(()))
                )
            {
                return Ok(Step::Retry);
            }
            let Ok(Ok(joined)) = time::timeout(
                SECOND * 20,
                Membership::join_with_video(
                    w.room,
                    &s.user,
                    &s.device,
                    &s.focus,
                    LEASE.as_millis() as u32,
                    video,
                    &mut w.signal,
                ),
            )
            .await
            else {
                return Ok(Step::Retry);
            };
            *m = joined;
            *lease = LeaseWatch::new();
            self.fresh = false;
            // Peers replace their keys when they see the new membership.
            w.keys.clear();
        }
        let Ok(Ok((sfu, token))) = time::timeout(SECOND * 10, authorize(w, s)).await else {
            return Ok(Step::Retry);
        };
        s.sfu = sfu;
        s.token = token;
        Ok(Step::Ready { fresh })
    }
}

/// Close a lost LiveKit room. Closing stops the SDK's own reconnect loop, so
/// it cannot come back as a second participant with our identity.
pub(super) async fn close(media: &mut Option<Room>) {
    if let Some(lk) = media.take() {
        let _ = time::timeout(SECOND * 4, lk.close()).await;
    }
}

/// Encrypt every local publication with sender key `index`. A full SDK
/// restart republishes tracks with new cryptors, which start at index 0.
pub(super) fn select_key(lk: &Room, index: u8) -> Result<()> {
    let local = lk.local_participant().identity();
    let cryptors = lk.e2ee_manager().frame_cryptors();
    for (sid, p) in lk.local_participant().track_publications() {
        let fc = cryptors
            .get(&(local.clone(), sid))
            .ok_or("Missing call track cryptor.")?;
        select(fc, &p, index)?;
    }
    Ok(())
}
/// [`select_key`] for one (re)published track.
pub(super) fn select_one(lk: &Room, p: &LocalTrackPublication, index: u8) -> Result<()> {
    let cryptors = lk.e2ee_manager().frame_cryptors();
    let fc = cryptors
        .get(&(lk.local_participant().identity(), p.sid()))
        .ok_or("Missing call track cryptor.")?;
    select(fc, p, index)
}
fn select(
    fc: &livekit::webrtc::native::frame_cryptor::FrameCryptor,
    p: &LocalTrackPublication,
    index: u8,
) -> Result<()> {
    if p.encryption_type() != EncryptionType::Gcm || !fc.enabled() {
        return Err("Call track encryption unavailable.");
    }
    fc.set_key_index(index.into());
    if fc.key_index() != i32::from(index) {
        return Err("Cannot select the call key.");
    }
    Ok(())
}

/// Our current microphone publication: a restart republishes it with a new SID.
pub(super) fn local_mic(lk: &Room) -> Option<LocalTrackPublication> {
    lk.local_participant()
        .track_publications()
        .into_values()
        .find(|p| p.source() == TrackSource::Microphone && p.track().is_some())
}

/// A resume may preserve tracks; a full SDK restart may assign new SIDs.
/// Only rebind the authenticated peer's current encrypted audio in this Room.
pub(super) fn encrypted_tracks(
    lk: &Room,
    me: &str,
    peer: &str,
) -> Result<Option<(LocalTrackPublication, RemoteAudioTrack)>> {
    let local = lk.local_participant();
    if local.identity().0 != me || !lk.e2ee_manager().enabled() {
        return Err("Could not restore encrypted call tracks.");
    }
    let theirs = lk
        .remote_participants()
        .get(&peer.to_owned().into())
        .and_then(|p| {
            p.track_publications()
                .into_values()
                .find(|p| p.source() == TrackSource::Microphone && p.is_subscribed())
        });
    let (Some(mic), Some(pubn)) = (local_mic(lk), theirs) else {
        return Ok(None);
    };
    if mic.encryption_type() != EncryptionType::Gcm || pubn.encryption_type() != EncryptionType::Gcm
    {
        return Err("Could not restore encrypted call tracks.");
    }
    Ok(match pubn.track() {
        Some(RemoteTrack::Audio(track)) => Some((mic, track)),
        _ => None,
    })
}

/// Opt-in acceptance diagnostics (debug builds, `UNTETHERED_QA_RECOVERY=1`):
/// lost sessions, rejoins and the index of each sender key this device starts
/// using. Never a key, token or address.
pub(super) fn trace(line: std::fmt::Arguments) {
    #[cfg(debug_assertions)]
    if std::env::var("UNTETHERED_QA_RECOVERY").as_deref() == Ok("1") {
        eprintln!("{line}");
    }
    #[cfg(not(debug_assertions))]
    let _ = line;
}
pub(super) fn trace_key(index: u8) {
    trace(format_args!("RTC sender key index={index}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoin_backs_off_to_five_seconds() {
        let delays: Vec<_> = (0..7u32)
            .map(|n| [0, 1, 2, 4, 5][n.min(4) as usize])
            .collect();
        assert_eq!(delays, [0, 1, 2, 4, 5, 5, 5]);
        assert!(Rejoin::new(Lost::Lease).fresh);
        assert!(!Rejoin::new(Lost::Media).fresh);
    }

    #[test]
    fn only_our_current_membership_counts() {
        let m = |event: &str, content: Value| json!({"type":EVENT_TYPE,"sender":"@a:h","event_id":event,"origin_server_ts":now(),"content":content});
        let live = json!({"application":"m.call","scope":"m.room","call_id":"","device_id":"D","expires":60000});
        assert!(ours(&[m("$mine", live.clone())], "@a:h", "$mine"));
        // A leave (the delayed one, or our explicit one) replaced it.
        assert!(!ours(&[m("$mine", json!({}))], "@a:h", "$mine"));
        // Another event in the same slot is not this membership.
        assert!(!ours(&[m("$other", live.clone())], "@a:h", "$mine"));
        assert!(!ours(&[m("$mine", live)], "@b:h", "$mine"));
    }
}
