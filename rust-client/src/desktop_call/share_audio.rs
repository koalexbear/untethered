//! Share one explicitly selected playback stream, never the whole output mix.
use super::*;
use crate::call_devices::AppStream;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Publisher {
    window: Option<Uuid>,
    attempt: Option<Uuid>,
    retired: Option<Uuid>,
    sid: Option<TrackSid>,
    worker: JoinSet<Result<()>>,
}
impl Publisher {
    async fn stop(&mut self, lk: &Room) -> Result<()> {
        self.worker.shutdown().await;
        if let Some(sid) = self.sid.take() {
            lk.local_participant()
                .unpublish_track(&sid)
                .await
                .map_err(|_| "Could not stop sharing application audio.")?;
        }
        Ok(())
    }
    pub(super) async fn sync(
        &mut self,
        capture: Option<(Uuid, watch::Receiver<()>)>,
        lk: &Room,
        key: u8,
        mode: &watch::Receiver<AudioMode>,
        events: &Sender<Value>,
        room: &str,
    ) -> Result<()> {
        let window = capture.as_ref().map(|(id, _)| *id);
        let request = {
            let m = mode.borrow();
            if window.is_some() && window == m.video[VideoInput::Window as usize] {
                m.send_app.clone()
            } else {
                None
            }
        };
        let attempt = request.as_ref().map(|(id, _)| *id);
        if self.window == window && self.attempt == attempt {
            return Ok(());
        }
        if self.window != window && self.attempt.is_some() {
            self.retired = self.attempt;
        }
        self.stop(lk).await?;
        self.attempt = attempt;
        self.window = window;
        if let (Some(window), Some((id, app))) = (capture, request) {
            if self.retired == Some(id) {
                self.finish(
                    Err("Window share changed. Select application audio again."),
                    lk,
                    events,
                    room,
                )
                .await?;
                return Ok(());
            }
            let result = self
                .start(lk, key, mode.clone(), window, id, app.clone())
                .await;
            if result.is_err() {
                self.stop(lk).await?;
            }
            let _=events.send(json!({"command":"share_audio_status","data":{"room":room,"attempt":id,
                "live":result.is_ok(),"label":result.map(|()|format!("Sharing audio: {app}")).unwrap_or_else(str::to_owned)}})).await;
        }
        Ok(())
    }
    async fn start(
        &mut self,
        lk: &Room,
        key: u8,
        mode: watch::Receiver<AudioMode>,
        window: (Uuid, watch::Receiver<()>),
        id: Uuid,
        app: AppStream,
    ) -> Result<()> {
        if !app.present().await {
            return Err("Playback stream disappeared. Refresh and choose again.");
        }
        let src = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 50);
        let track = LocalAudioTrack::create_audio_track(
            "Application audio",
            RtcAudioSource::Native(src.clone()),
        );
        let p = lk
            .local_participant()
            .publish_track(
                LocalTrack::Audio(track),
                TrackPublishOptions {
                    source: TrackSource::ScreenshareAudio,
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Could not publish application audio.")?;
        self.sid = Some(p.sid());
        let fc = lk
            .e2ee_manager()
            .frame_cryptors()
            .remove(&(lk.local_participant().identity(), p.sid()))
            .ok_or("Application audio encryption unavailable.")?;
        if p.encryption_type() != EncryptionType::Gcm || !fc.enabled() {
            return Err("Application audio encryption unavailable.");
        }
        fc.set_key_index(key.into());
        if fc.key_index() != i32::from(key) {
            return Err("Cannot encrypt application audio.");
        }
        self.worker.spawn(record(src, app, mode, window, id));
        Ok(())
    }
    pub(super) async fn ended(&mut self) -> Result<()> {
        if self.worker.is_empty() {
            return std::future::pending().await;
        }
        self.worker
            .join_next()
            .await
            .and_then(|j| j.ok())
            .unwrap_or(Err("Application audio interrupted."))
    }
    pub(super) async fn finish(
        &mut self,
        result: Result<()>,
        lk: &Room,
        events: &Sender<Value>,
        room: &str,
    ) -> Result<()> {
        self.stop(lk).await?;
        if let Some(id) = self.attempt {
            let _=events.send(json!({"command":"share_audio_status","data":{"room":room,"attempt":id,"live":false,
                "label":result.err().unwrap_or("Application audio stopped.")}})).await;
        }
        Ok(())
    }
}
async fn record(
    src: NativeAudioSource,
    app: AppStream,
    mut mode: watch::Receiver<AudioMode>,
    (window, mut capture_lifetime): (Uuid, watch::Receiver<()>),
    id: Uuid,
) -> Result<()> {
    let stop = |m: &AudioMode| {
        m.video[VideoInput::Window as usize] != Some(window)
            || m.send_app.as_ref().is_none_or(|r| r.0 != id)
    };
    let capture = async {
        let mut child = Command::new("parec")
            .args([
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=1",
                "--latency-msec=40",
                "--client-name=Untethered",
                "--stream-name=Shared application audio",
            ])
            .arg(format!("--monitor-stream={}", app.index))
            .arg(format!("--device={}", app.monitor))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Cannot open application audio. Is parec installed?")?;
        let mut pipe = child.stdout.take().unwrap();
        // Verify again after binding. The recorder stays bound to this individual sink input.
        if !app.present().await {
            return Err("Selected playback stream is no longer available.");
        }
        let read = async {
            let mut buf = [0; 960];
            loop {
                pipe.read_exact(&mut buf)
                    .await
                    .map_err(|_| "Selected application audio ended.")?;
                let data = buf
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .copied()
                    .map(i16::from_le_bytes)
                    .collect::<Vec<_>>();
                src.capture_frame(&AudioFrame {
                    data: data.into(),
                    sample_rate: 48000,
                    num_channels: 1,
                    samples_per_channel: 480,
                })
                .await
                .map_err(|_| "Application audio capture failed.")?;
            }
        };
        let watch_source = async {
            loop {
                time::sleep(SECOND * 2).await;
                if !app.present().await {
                    return Err("Selected playback stream ended. Choose another to resume.");
                }
            }
        };
        tokio::select! {r=read=>r,r=watch_source=>r}
    };
    tokio::select! {biased;_=capture_lifetime.changed()=>Ok(()),_=mode.wait_for(stop)=>Ok(()),r=capture=>r}
}
