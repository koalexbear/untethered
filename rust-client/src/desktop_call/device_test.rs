//! The Voice & Video page's mic test (calls plan U2), Discord's "Let's
//! Check": the microphone through the same processing, gate and volume as a
//! call, played back through a call's mixer to the chosen speaker at the
//! output volume, with its level for the page's meter. No call, no network.
use super::{
    AudioMode, DeviceNotices, Result, Route, TalkGate,
    audio::{self, AudioIo, Kind, Mixer, Source},
    voice::{InputLevel, Voice},
};
use tokio::sync::watch;

/// Run until dropped (the page stopped the test) or a device fails.
/// `mode` carries the chosen devices and the settings, as a call's does;
/// a device change reopens the microphone or the speaker. With push to
/// talk (`mode.talk` not open) the gate is skipped: the test plays
/// everything, as Discord's does.
pub(crate) async fn mic_test(
    mut mode: watch::Receiver<AudioMode>,
    level: watch::Sender<InputLevel>,
) -> Result<()> {
    let io = audio::platform();
    // The mixer reports a speaker that fails; nothing else uses the channel.
    let (events, failures) = async_channel::unbounded();
    let notices = DeviceNotices {
        events,
        room: String::new(),
    };
    let mixer = Mixer::start(io.clone(), mode.clone(), notices);
    let voice = Voice::new(level);
    // The echo canceller hears what the test plays, as in a call, so
    // speakers near the microphone don't feed back.
    mixer.far_end(voice.reference());
    let playback = mixer.add(Kind::Voice, None);
    let settings = mode.clone();
    let test = async {
        loop {
            let route = mode.borrow_and_update().mic.clone();
            tokio::select! {
                biased;
                changed = mode.wait_for(|s| s.mic.revision != route.revision) => {
                    changed.map_err(|_| "The mic test ended.")?;
                }
                result = loopback(&*io, &route, &voice, &settings, &playback) => {
                    return result;
                }
            }
        }
    };
    let speaker = async {
        loop {
            match failures.recv().await {
                Ok(notice) if notice["data"]["failed"] == true => {
                    return Err("Speaker unavailable. Choose another output.");
                }
                Ok(_) => {}
                Err(_) => std::future::pending::<()>().await,
            }
        }
    };
    let result = tokio::select! {
        result = test => result,
        result = speaker => result,
    };
    voice.closed();
    result
}

/// The microphone `route`, processed and gated, into the mixer.
async fn loopback(
    io: &dyn AudioIo,
    route: &Route,
    voice: &Voice,
    settings: &watch::Receiver<AudioMode>,
    playback: &Source,
) -> Result<()> {
    const STOPPED: &str = "Microphone unavailable. Choose another input.";
    let mut input = io.open_input(route.name.as_deref()).map_err(|_| STOPPED)?;
    let mut capture = voice.capture();
    let mut frame = [0; audio::SAMPLES];
    while input.read(&mut frame).await {
        let (voice, push_to_talk) = {
            let mode = settings.borrow();
            (mode.voice, mode.talk != TalkGate::Open)
        };
        capture.frame(&mut frame, &voice, push_to_talk);
        playback.push(&frame);
    }
    Err(STOPPED)
}
