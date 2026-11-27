# Calls, streaming and screen sharing: Discord parity plan

**2026-11-25 · reviewed at `main` `70d5d0a` · research and planning only, no code changed**

This compares Untethered's Linux MatrixRTC calls with Discord's voice, video and
Go Live features. It ranks the gaps, then lays out a delivery order of small,
shippable steps. Each step lists the files involved, the approach and how to
verify it with the existing harnesses. The non-call UI restyle (canvas icon set in
`view/icons.rs`, hover toolbar in `view/hover.rs`, Discord palette in `view.rs`,
Discord-style user panel) is still uncommitted in the main checkout. Every UI
step below builds on it and waits until it lands.

## Summary: what to do first

1. **Fix video quality.** Today a screen share is captured at 960×540 and at most
   12 fps. LiveKit then *encodes it at 5 fps and 400 kbps*, because we never set an
   encoding and its screen-share preset table picks `H720_FPS5` for a 960-px source.
   Receivers redraw every 100 ms (10 fps). Explicit encodings, source-size capture
   and a faster render path are mostly in `desktop_call/room_video.rs`. This is the
   cheapest large visible win. (Step V1.)
2. **Add voice processing.** No echo cancellation, noise suppression or gain
   control runs on the microphone (`AudioSourceOptions::default()` is all `false`),
   and there is no input-sensitivity gate. `libwebrtc` 0.4.2, already linked, ships
   an `AudioProcessingModule` with AEC, AGC, HPF and NS. It needs the far-end mix as
   its reference signal, so first mix playback in-process into one output stream.
   (Steps A1 and A2.)
3. **Make voice rooms survive a network blip.** DMs already recover from short
   outages, but a voice room treats `RoomEvent::Reconnecting` as fatal
   (`channel.rs:371`). Neither path rejoins after the 16 s delayed leave fires.
   (Step R1.)
4. **Adopt Discord's call layout once the restyle lands:** a compact Voice
   Connected panel above the user panel, with Mute/Deafen moving into the user
   panel and a persisted Voice & Video settings page replacing the per-call
   Devices/Mixer drawer. (Steps U1 and U2.)
5. **Build a real Go Live flow and a global push-to-talk key.** Add *Screen* as well
   as *Window*, quality/fps choice, inline audio choice and an opt-in **Watch
   stream**. Use the XDG GlobalShortcuts portal for push-to-talk while another app
   has focus. Discord itself only shipped that on Wayland this month. (Steps U4
   and K1.)

Windows and web calls are large, separate projects (P1, P2). The engine move to
matrix-rust-sdk affects how call signaling is written, not what the user sees
(see [Engine boundary](#engine-boundary-gomuks-today-matrix-rust-sdk-next)).

---

## 1. Where Untethered is today

Verified on Linux with Element Web 1.12.35 and between native clients (see
`ROADMAP.md` milestone 3 and `experiments/rtc-audio/README.md`):

- **Lifecycle.** Encrypted DM voice/video, ringing, decline, cancel, no answer,
  answered on another device, TURN-only, and short-outage DM recovery (6.5 s link
  loss, no redial).
- **Voice rooms.** Join without ringing. The roster appears under the channel with a
  text **Speaking** label. Per-user mixer (0–200 %), push-to-talk on Right Ctrl while
  the window is focused, key rotation on every roster change.
- **Video.** Portal camera with live switching, grid/focus stage, theater view, a
  dock preview while browsing.
- **Sharing.** Portal **window** share. One selected app's playback stream is sent as
  an encrypted `ScreenshareAudio` track. Incoming tab audio is playable separately.

How the media path is built, with the facts that matter for parity:

| Area | Implementation | Consequence |
|---|---|---|
| Video capture | `gst-launch-1.0 pipewiresrc … videorate max-rate=12 … video/x-raw,format=I420,width=960,height=540` piped raw to the app ([room_video.rs:342-377](../rust-client/src/desktop_call/room_video.rs)) | Every source is letterboxed to 960×540 at ≤12 fps |
| Video encode | `TrackPublishOptions { simulcast: false, ..Default::default() }` with no `video_encoding` ([room_video.rs:619-628](../rust-client/src/desktop_call/room_video.rs)). LiveKit 0.12 `compute_appropriate_encoding` chooses `screenshare::H720_FPS5` (400 kbps, 5 fps) for a 960-px share and `video::H720` (1.7 Mbps, 30 fps) for a 960-px camera | **Screen share: 5 fps.** Camera: 12 fps capture. VP8 software encoder |
| Video render | One decode task per track. Every 100 ms it scales to ≤960×540, converts to RGBA on the CPU and creates a new `image::Handle` ([room_video.rs:147-193](../rust-client/src/desktop_call/room_video.rs)) | ≤10 fps on screen. A new texture per frame per tile |
| Subscription | All admitted camera/screen tracks are subscribed at full quality. `adaptive_stream` and `dynacast` are off (SDK defaults) | Bandwidth and CPU grow with every camera, even when the stage is hidden |
| Mic | `pacat --record` mono s16le 48 kHz, 40 ms latency, into `NativeAudioSource(AudioSourceOptions::default())` ([desktop_call.rs:1040](../rust-client/src/desktop_call.rs), [:1217-1241](../rust-client/src/desktop_call.rs)) | No AEC/NS/AGC, no VAD gate. PulseAudio/PipeWire-Pulse only |
| Playback | One `pacat --playback` child **per remote audio track** with gain applied in Rust ([desktop_call.rs:1284-1346](../rust-client/src/desktop_call.rs)). DM playback passes `user: None` ([desktop_call.rs:786-793](../rust-client/src/desktop_call.rs)) | N processes for N speakers. No far-end mix for AEC. **No per-user volume in DMs** |
| Recovery | DM: keeps the membership, waits for SDK `Reconnected` and rebinds tracks, up to 60 s. Fails once the 16 s delayed leave lapses (`LEASE`, `RECOVERY_LIMIT` at [desktop_call.rs:60-61](../rust-client/src/desktop_call.rs)). Voice room: returns "Voice room connection lost." on the first `Reconnecting` ([channel.rs:370-371](../rust-client/src/desktop_call/channel.rs)) | Voice channels drop on any SFU hiccup |
| Push-to-talk | Iced window keyboard events for `ControlRight` only ([main.rs:2132-2149](../rust-client/src/main.rs)). The setting lives in memory (`room_ptt`) | Fixed key, focused window only, forgotten on quit |
| Share source | `SourceType::Window` only, `CursorMode::Hidden`, `PersistMode::DoNot` ([room_video.rs:302-327](../rust-client/src/desktop_call/room_video.rs)) | No full-monitor share, no cursor, picker every time |
| Platforms | `livekit` is a `cfg(target_os = "linux")` dependency ([Cargo.toml](../rust-client/Cargo.toml)). The call stage and controls modules are Linux-only | No calls on Windows or web |

The [evidence screenshots](../experiments/rtc-audio/evidence/) predate commit
`76d6e43`, which moved the controls from a bottom pill bar into the sidebar dock
(`call_bar` in `view.rs`). In theater view that 211-px dock now floats over the
bottom-left of the stage.

---

## 2. Gap analysis

### 2.1 UX and UI against Discord

| Area | Discord | Untethered today | Gap |
|---|---|---|---|
| **Voice Connected panel** | A compact block above the user panel. Green "Voice Connected" with a signal icon (click for ping and packet loss), "#channel / Server" link, Disconnect, noise-suppression toggle. Then Camera / Screen (/Activity / Soundboard) buttons | `call_bar` stacks status, room name, Mic/Camera/Screen/Hang-up icons, text buttons Deafen/Devices/Mixer, the PTT checkbox and a scrolling drawer of pick-lists and sliders ([view.rs:246-560](../rust-client/src/view.rs)) | Too tall and settings-heavy. Mute sits in the call block. No connection quality. No link back to the channel |
| **Mute / Deafen** | In the user panel next to Settings, always visible, each with a chevron menu for input/output device. Also used outside calls (pre-mute before joining) | In the call block only. Deafen is a text button | Move into the restyled user panel. Allow muting before joining |
| **Voice channel rows** | 24-px avatar, green speaking ring, red slashed mic/headphones icons, camera icon, red **LIVE** pill. Click LIVE to watch | Indented name. Bold green "Speaking" text ([view.rs:740-757](../rust-client/src/view.rs)) | Avatars, ring, state icons, LIVE, click-to-watch |
| **Per-user volume** | Right-click a user → **User Volume** 0–200 %, Mute, Disable Video, Profile/Message | **Mixer** drawer in the call bar, voice rooms only | Context menu on rows and tiles. Local mute. DMs too |
| **Call stage** | Grid or focus, speaking border, name + mute badge, hover controls on each tile (pin/focus, fullscreen, popout for streams, volume). Bottom bar (camera, screen, mute, deafen, disconnect). Chat toggle, "hide non-video members", full screen, popout window | Grid/focus, green border while speaking, name + LIVE badge, click to focus, **Theater**, **Open chat** ([call_stage.rs](../rust-client/src/view/call_stage.rs)) | Tile hover actions, mute badges, stage control bar, fullscreen, popout, hide-non-video |
| **Go Live** | Screen button → sheet with **Screens / Applications** thumbnails, resolution (720p / 1080p / Source) and fps (15 / 30 / 60). Then Go Live. Own preview tile, viewer count and avatars, Stream settings / Stop. Viewers opt in with **Watch Stream** | **Share window** goes straight to the portal picker. App audio is a pick-list in the call drawer after sharing starts. Everyone auto-subscribes | Screen source, quality/fps, inline audio choice, opt-in watching, viewer list |
| **Incoming call** | A ringing card with a large avatar and pulse, **Join Call** (with or without video) / Decline. Ringing header in the DM. OS notification when unfocused | A one-line banner, "Incoming voice call · @raw:mxid", Decline / Accept ([view.rs:577-637](../rust-client/src/view.rs)). Foreground only | Display name/avatar, accept-with-video, ring card, desktop notification |
| **Voice & Video settings** | Persistent input/output device and volume, mic test, **Voice Activity vs Push-to-Talk**, input sensitivity (auto or slider with a live meter), PTT keybind and release delay, echo cancellation, noise suppression (Krisp/standard/off), AGC, attenuation, camera preview, hardware acceleration | Per-call pick-lists, lost after the call. PTT checkbox kept until quit | A whole settings page plus persistence |
| **Keybinds** | Global PTT, toggle mute, toggle deafen. Wayland support via the GlobalShortcuts portal landed in November 2026 | Right Ctrl, focused window only | Configurable global bindings |

### 2.2 Technical capability

| Capability | Discord | Untethered | What exists to build on |
|---|---|---|---|
| Screen-share quality | Free 720p30. Nitro 1080p60 up to 4K60. H.264/AV1 with hardware encoders | 960×540, 5 fps encoded, VP8 software | LiveKit presets up to `H1080_FPS30`, explicit `VideoEncoding`, `VideoEncoderBackend::{Hardware,Vaapi,Nvenc}`, VP9/AV1/H264, `DegradationPreference` |
| Camera quality | 720p30 typical, simulcast | 960×540 at ≤12 fps, no simulcast | Simulcast with custom layers, `set_video_quality` on remote publications |
| Adaptive bitrate | Simulcast plus per-viewer layer choice. Streams are opt-in | None. Full-quality auto-subscribe | `RoomOptions { dynacast, adaptive_stream }`, `RemoteTrackPublication::{set_video_quality, set_enabled, set_subscribed}`. Server-side congestion control |
| Full-monitor capture | Yes | No | Portal `SourceType::Monitor` and restore tokens (ashpd 0.11.1). libwebrtc `DesktopCapturer` (Screen/Window, Generic = portal on Linux) |
| Stream audio | Windows per-app. Linux Wayland: **whole system or nothing**; per-app is waiting on upstream portal work | **Per-app** sink-input capture (ahead of Discord on Linux). No whole-system option | `parec --monitor-stream`. A PipeWire graph helper process (same pattern as `untethered-cameras`) for "everything except the call" |
| Voice processing | AEC, Krisp NS, AGC, VAD sensitivity | AEC, NS, AGC and the sensitivity gate since A2 (libwebrtc's APM) | `libwebrtc::native::apm::AudioProcessingModule` (AEC/AGC/HPF/NS, `process_reverse_stream`), `audio_mixer`, `audio_resampler`. RNNoise in pure Rust (`nnnoiseless`) as a stronger NS option |
| Global PTT | Windows/X11/Wayland portal | Focused window | ashpd 0.11.1 `desktop::global_shortcuts` (`bind_shortcuts`, `receive_activated`/`receive_deactivated`). X11 key grab fallback |
| Reconnection | Automatic with visible RTC states and rejoin | DM short outages only. Voice rooms none | DM `LeaseWatch` / `encrypted_tracks` logic, SDK `Reconnecting`/`Reconnected` events |
| Windows | Native | Cross-builds without calls | LiveKit Rust SDK supports Windows. libwebrtc `DesktopCapturer` (WGC/DXGI). WASAPI process loopback for app audio |
| Web | Browser client (no stream quality control) | No calls | livekit-client JS with insertable-streams E2EE, `getDisplayMedia`. gomuks's own web UI embeds Element Call |

### 2.3 What the engine and media stack already give us

**gomuks (v0.2611.1, today's engine)** covers everything the call layer needs
from Matrix: `get_rtc_transports`, `request_openid_token`, `set_state` (including
`delay_ms`), `update_delayed_event` (`cancel` / `restart` / `send`),
`send_to_device` (Olm-encrypted), `listen_to_device`, `get_room_state`,
`send_event`, and `sync_complete` / `events_decrypted` frames. No parity item
needs a new gomuks feature. Ring, decline, membership and key handling are
already ours (`rtc_membership.rs`, `rtc_keys.rs`, `incoming.rs`).

**LiveKit Rust SDK 0.12.0 / libwebrtc 0.4.2** already provides simulcast (custom
layers), SVC, the codecs and encoder backends above, degradation preference,
dynacast/adaptive-stream flags, per-publication quality and enable controls, GCM
frame encryption with key indices, stats, `AudioProcessingModule`, `AudioMixer`,
resampler and `DesktopCapturer`. **We still own** mic/speaker/camera device I/O
(pacat and GStreamer subprocesses today), video rendering, VAD, hotkeys, and any
"who is watching" signal.

**LiveKit server** provides active-speaker detection (used for **Speaking**),
simulcast layer forwarding, dynacast and TURN (verified with coturn).

### Engine boundary: gomuks today, matrix-rust-sdk next

> **Update 2026-11-26:** the typed `CallSignal` seam landed (`a54f6f8`), call
> signaling runs on matrix-sdk (`dc68935`), matrix-sdk is the default engine and
> gomuks has been removed from the client (`02367e4`). The paragraph below is the
> plan as written.

On 2026-11-25 the project decided to replace gomuks with matrix-rust-sdk, and a
spike is under way. The call layer talks to gomuks through `Wire`
(`desktop_call.rs`) and the `rtc_membership::Signal` trait using `serde_json::Value`
commands. To avoid deepening that coupling, step R1 introduces a typed
`CallSignal` trait covering the nine operations above. The gomuks adapter wraps
`Wire`. A later matrix-rust-sdk adapter must provide delayed events (MSC4140),
encrypted to-device send and receive, OpenID tokens and RTC transport discovery
(MSC4143). Check these in the spike before porting. Media, UI and every other
step below are independent of the engine.

---

## 3. (a) Gaps ranked by user impact against effort

Impact: **H** = users notice in their first session / blocks a core Discord
workflow. **M** = noticeable regularly. **L** = polish. Effort: **S** ≤ a few days,
**M** ≈ 1–2 weeks, **L** = multi-week project.

| Rank | Gap | Impact | Effort | Blocked by |
|---|---|---|---|---|
| 1 | Screen-share/camera quality: 5 fps share, 960×540, 12 fps capture, 10 fps render | H | S→M | — |
| 2 | No echo cancellation / noise suppression / AGC / input sensitivity | H | M | A1 mixer |
| 3 | Voice rooms drop on any SFU reconnect. No rejoin after lease expiry | H | M | — |
| 4 | Discord call layout: Voice Connected panel, Mute/Deafen in user panel, persisted Voice & Video settings | H | M | restyle merge |
| 5 | Go Live flow: no screen source, quality/fps, inline audio, opt-in watching | H | M | V1, restyle |
| 6 | Push-to-talk only in the focused window, fixed key | H (gamers) | M | — |
| 7 | Voice channel rows (avatar, ring, mute/LIVE icons). Right-click user menu. DM volume | M-H | S-M | restyle |
| 8 | No simulcast or adaptive quality. Hidden video still downloaded | M | S-M | V1 |
| 9 | Incoming call: raw MXID, no ring card, no notification when unfocused | M | S | restyle |
| 10 | Stage: tile hover actions, mute badges, fullscreen, stage control bar | M | M | restyle |
| 11 | Hardware encode / codec choice (CPU at 1080p) | M | M | V1 |
| 12 | One pacat process per remote track. 40 ms buffers. Pulse-only I/O | M | M | — (enables 2, P1) |
| 13 | Group DM calls (DMs must have exactly two members) | M | M | R1 |
| 14 | Whole-system stream audio (excluding the call) | M | M-L | U4 |
| 15 | Connection quality indicator and stats | L-M | S | U1 |
| 16 | Stream popout window | M | L | U6 |
| 17 | Windows calls | H | L | A1, K1 seams |
| 18 | Web calls | M-H | L | engine decision |
| — | Soundboard, activities, Krisp-grade NS, viewer counts | L | — | later, by demand |

---

## 4. (b) Recommended delivery order

Each step ships on its own, keeps the existing acceptance tests green and adds
one new assertion. **Phase 1 touches no `view.rs`/`view/` files** and can start
now. Keep its `main.rs` edits to a few lines, because the uncommitted restyle also
modifies `main.rs`. **Phase 2** starts after the restyle is committed to `main`.

| Phase | Step | Ships | Rank |
|---|---|---|---|
| 1 | **V1** Video quality and full-screen sharing | Share at up to 1080p, 15/30 fps, monitor or window, cursor. Receivers render at ≥24 fps | 1, 5 (part) |
| 1 | **R1** Voice-room recovery and rejoin (+ `CallSignal` seam) | Voice channels ride out blips. Both call kinds rejoin after expiry | 3 |
| 1 | **A1** In-process mixer, one output stream | One playback stream per call. Per-user volume in DMs | 12, 7 (part) |
| 1 | **A2** Voice processing and input sensitivity | AEC/NS/AGC on by default. VAD gate | 2 |
| 1 | **V2** Simulcast, per-tile quality, pause hidden video | Camera simulcast. Thumbnails use the low layer | 8 |
| 1 | **K1** Global push-to-talk backend | Portal/X11 global key with a release delay | 6 |
| 2 | **U1** Voice Connected panel and user-panel Mute/Deafen | Discord sidebar layout | 4 |
| 2 | **U2** Voice & Video settings page (persisted) | Devices, volume, VAD/PTT, keybind, processing toggles, mic test, camera preview | 4, 6 |
| 2 | **U3** Participant rows and right-click user menu | Avatars, speaking ring, state icons, LIVE, User Volume | 7 |
| 2 | **U4** Go Live sheet and Watch Stream | Source/quality/fps/audio sheet, opt-in viewing, restore token | 5 |
| 2 | **U5** Incoming call card and desktop notification | Name/avatar, accept with video, ring when unfocused | 9 |
| 2 | **U6** Stage polish | Tile hover actions, badges, fullscreen, stage control bar | 10 |
| 3 | **C1** Hardware encoding and codec selection | VAAPI/NVENC, 1080p60 option | 11 |
| 3 | **C2** Whole-system stream audio | "Everything except this call" | 14 |
| 3 | **C3** Group DM calls | Ring several people | 13 |
| 3 | **C4** Connection info | Ping/packet loss in the panel | 15 |
| 3 | **C5** Popout stream window | Separate OS window per stream | 16 |
| 4 | **P1** Windows calls | | 17 |
| 4 | **P2** Web calls | | 18 |

V1 goes first: it is small, isolated and fixes the most visible problem. R1, A1/A2,
V2 and K1 are independent of one another and could run in parallel (A2 needs A1).

---

## 5. (c) Step details, with (e) verification

Verification builds on the harnesses that already exist:
`experiments/rtc-audio/run_element.sh` (modes `--voice-room`, `--room-ptt`,
`--window-share`, `--room-camera`, `--room-layout`, `--dm-video`, `--dm-window`,
`--native-dm`, plus the `scripts/test-media-interop.py` modes it forwards such as
`--desktop-dm`, `--incoming-dm`, `--element-video`, `--call-controls`,
`--call-devices`, `--call-reconnect`, `--call-turn`, `--multi-device-ring`), the
`scripts/qa_*.py` fixtures, LiveKit's Twirp `RoomService` (already queried in
`qa_desktop_dm.py`), and the isolated virtual KWin in `scripts/qa_kwin.py` driven by
KDE `selenium-webdriver-at-spi-inputsynth` from `.dev/portal-qa/prefix/bin/`.

**Cross-cutting harness additions** (do these alongside V1):

- **`UNTETHERED_QA_VIDEO_STATS=1`**, debug builds only, alongside the existing
  `UNTETHERED_QA_ICE_STATS`. Every few seconds, print per-track direction, source,
  `frameWidth`/`frameHeight`, `framesPerSecond`, the active simulcast RID, and bytes
  from `Room::get_stats()`. No addresses, keys or SDP. Most video assertions read it.
- **A render-rate probe.** In debug builds, count `video_redraw` frames actually
  presented per tile and report them the same way. Pixel checks prove content, not
  frame rate.
- **A QA control map for icon-only buttons** (needed by Phase 2). Today
  `qa_voice_room.py` finds controls by OCR of their text (`tesseract --psm 7`).
  Icon-only Discord controls have no text. Add a debug-only `UNTETHERED_QA_LAYOUT`
  dump of control id → bounds, or have the fixtures hover and OCR the tooltip.
  Decide before U1. Otherwise every call fixture breaks at once.

### V1 — Video quality and full-screen sharing (Phase 1, S→M)

> **Shipped** in [koalexbear/untethered#4](https://github.com/koalexbear/untethered/pull/4)
> (merged as `a5bd692`, 2026-11-25). Shares go out at source size: 1600×928 at
> 29–30 fps in `--window-share`, decoded at 30 fps by both receivers.
> `--switch-camera` and the app-audio step of `--dm-window` still fail, because
> their fixtures look for the Devices panel where it was before the call dock moved
> it. Fix those fixtures before or with U1.

**Files:** `rust-client/src/desktop_call/room_video.rs` (capture, publish, display).
`VideoInput::control` labels live in the same file. No view changes.

**Approach**

1. **Explicit encodings.** Screen: `VideoEncoding { max_bitrate: 2_500_000,
   max_framerate: 30.0 }` at up to 1920×1080. Camera: 1280×720, 1.7 Mbps, 30 fps.
   Keep `DegradationPreference` at the SDK defaults (MaintainResolution for
   screen, MaintainFramerate for camera). Take quality and fps as parameters, which
   U4 will set. Default to 1080p/30 for screens and 720p/30 for cameras.
2. **Capture at source size.** Read the portal stream's `size()` (ashpd) and fit it
   into the chosen cap with even dimensions. Pass that to the GStreamer caps and to
   `NativeVideoSource::new`. Replace the `W`/`H`/`FRAME` constants with per-capture
   fields. Raise `videorate max-rate` to the requested fps. Keep `add-borders=true`
   so a window resize letterboxes instead of breaking the frame size. Cameras
   request 1280×720 and let `videoscale` adapt.
3. **Monitor sources and cursor.** Use `SourceType::Window | SourceType::Monitor`
   (the KDE picker shows both) and accept either in the response check. Use
   `CursorMode::Embedded` when the portal offers it. Relabel "Share window" as
   "Share screen".
4. **Faster receive path.** Lower the display tick to the publisher's frame rate
   (≤33 ms). Scale to the largest tile actually shown (≤1920×1080 for focus and
   theater, ~320×180 for thumbnails) instead of a fixed 960×540. If profiling shows
   per-frame `image::Handle` allocation is the bottleneck, move it to a follow-up
   **V1b**: an `iced::widget::shader` program that keeps one Y/U/V texture set per
   feed and converts on the GPU. Only the wgpu renderer can do this; tiny-skia
   keeps the CPU path.

**Verify**

- `run_element.sh --window-share`: enlarge the `qa_window_pattern.py` window to
  1600×900, add a moving bar and a 12-px text block. Assert that the sender's
  outbound screen track reports ≥1280 px wide and ≥24 fps, that both receivers'
  inbound fps is ≥24, that the render-rate probe shows ≥24 presented frames/s on the
  focused tile, and that Tesseract reads the 12-px text from a receiver screenshot.
  Keep the existing assertions (cancel creates no track, stop releases the portal
  and PipeWire nodes).
- **Monitor share:** in the same KWin fixture, choose the screen entry in the KDE
  picker with inputsynth. Receivers must see both the pattern window and the decoy
  window. Today only the selected window is visible.
- `run_element.sh --element-video --dm-window`: Element's received width is
  measured at 924 px today. Add a Playwright `getStats()` check that Element decodes
  the native share at ≥24 fps.
- Camera: `--room-camera` and `--switch-camera` stay green with 1280×720 outbound.

### R1 — Voice-room recovery and rejoin (Phase 1, M)

> **Shipped** on main as `b268b35` (2026-11-26): `desktop_call/recovery.rs`,
> with fixtures `--call-reconnect` and `--room-reconnect`.

**Files:** `desktop_call/channel.rs`, `desktop_call.rs` (move `LeaseWatch`,
`encrypted_tracks`, recovery state into a new `desktop_call/recovery.rs`),
`rtc_membership.rs`. Add a typed `CallSignal` trait next to `Signal`.

**Approach**

- On `Reconnecting`: stop playback jobs, keep the `Room`, the key provider, the
  membership and the mic publication (the DM path already does this). Show
  "Reconnecting…". On `Reconnected`, rerun `Subscriptions::sync` and
  `GroupKeys::reconcile`. If track SIDs changed, rebind them. Re-send the current
  outbound key to the roster, because peers may have rotated while we were away.
- **Rejoin after expiry**, for both call kinds. If the delayed leave fired (lease
  lost for more than 16 s) but the user never pressed Leave, publish a *new*
  membership with a fresh `created_ts` and a fresh sender key, then reconnect to the
  SFU with a new JWT. Never revive the expired membership; the DM code's
  "never replace an expired membership" rule still holds. Give up after a bounded
  window (Discord keeps retrying while showing "RTC Connecting"). Proposal: retry
  with backoff for 5 minutes. A DM rejoins only if the peer's original session is
  still active.
- **`CallSignal`:** typed methods for identity, room state, set state (with an
  optional delay), delayed-event update, encrypted to-device send, the to-device
  stream, OpenID token, RTC transports and send event. Port `Wire` to implement it.
  Keep it in its own commit so the matrix-rust-sdk spike can implement the same
  trait.

**Verify**

- New `run_element.sh --room-reconnect`. It combines `qa_voice_room.py` (three
  clients) with the network-namespace/veth/`ss`/`nft` setup from `--call-reconnect`.
  (a) Take Bruno's link down for 6.5 s. Alice and Clara must keep hearing each other
  in every 100 ms window. After link-up, all six paths carry fresh tones on Bruno's
  *original* membership, and his microphone track ID may change. (b) Block only the
  SFU ports with `nft` so Matrix stays up. Same outcome. (c) A 25 s outage. Bruno
  must rejoin with a new membership event, everyone's key index advances, all six
  paths recover, and the room shows no duplicate Bruno.
- `--call-reconnect` and `--native-dm` (both DM directions) stay green. Add a 25 s
  variant to `--call-reconnect`: rejoin without a new ring event in the DM timeline.

### A1 — In-process audio mixer and one output stream (Phase 1, M)

> **Shipped** on main as `3727ed3` (2026-11-26): one mixed playback stream per
> client, and the DM Mixer's volume is checked at the desktop speaker.

**Files:** new `desktop_call/audio.rs`, plus `desktop_call.rs` (`play`,
`playback_pcm`, `controlled_capture`), `channel.rs` (`Subscriptions`),
`share_audio.rs` (unchanged API).

**Approach:** Each remote `NativeAudioStream` writes 10 ms frames into a per-source
ring buffer. One mixer task applies per-user gain, deafen and share-silent, then
sums with a clipping limiter (reuse `playback_pcm`'s clamp). It writes to a
**single** pacat playback child per call: on a speaker change, restart that one
child. The DM path passes the peer's user ID, so the mixer volume works in DMs. The
mixed frame is the far-end reference for A2. Hide the device I/O behind a small
`AudioIo` trait (open input, open output, list devices) with the pacat backend as
the Linux implementation. P1 then adds WASAPI without touching the mixer.

Keep subprocesses on Linux. In-process PipeWire clashes with libwebrtc's embedded
PipeWire symbols (see the comment in `cameras.rs`). In-process `libpulse` could
come later if latency measurements justify it.

**Verify:** `--voice-room` (all mixer assertions at 25 %, 150 %, 0 % and Reset),
`--call-devices`, `--call-controls` and `--dm-window` (share-audio unchecked and
deafen) must pass unchanged. New assertions: `pactl list sink-inputs` shows exactly
**one** Untethered playback stream per client in a three-person room, and in a DM
during an Element tab share. A DM run sets Element's volume to 25 % and checks the
960 Hz level drop at the desktop speaker, as the voice-room mixer test does.

### A2 — Voice processing and input sensitivity (Phase 1, M)

> **Done** on branch `claude/a2-voice-a94df8` (2026-11-27), not merged yet.
> `desktop_call/voice.rs`: one libwebrtc `AudioProcessingModule` per call
> session (AEC3, NS, AGC2, HPF), fed the mixer's played frames as the
> reference; every 10 ms mic frame is processed, metered and gated before
> `capture_frame`. A module's settings are fixed when built, so a switch
> builds a new one. The gate: **Auto** follows a noise floor that falls
> within frames and rises 5 dB/s, opening 12 dB above it (no ceiling: any
> steady sound is learned in seconds); **manual** is a threshold, −100 dB
> sending everything. 200 ms hangover, a one-frame ramp at each edge,
> silence below it with the publication unmuted; push-to-talk replaces it
> while armed. The gate's verdict is the local Speaking state (voice rooms)
> and, with the level, a `watch` for the meter. Settings live in
> `call_settings.rs`, saved per computer as `voice.json` in the profile
> (Discord's defaults: everything on, sensitivity automatic), and apply to
> the call at once. The controls (`view/voice_settings.rs`: Input
> sensitivity with the meter under the threshold slider, and the three
> Voice processing switches) sit in the dock's Devices panel until U2.
> Found on the way: the timer-paced mixer could leave up to 680 ms queued
> in the pipe to `pacat`, which kept AEC3 from converging for 20 s or more;
> that pipe is now one page (≈43 ms), so `pacat` paces the mixer. With all
> three modules on, AGC2 lifts steady noise back up for the first ≈10 s of
> a module, which is why the automatic gate has no ceiling.
> `--voice-processing` (release builds) measures, switching Alice's
> settings in her Devices panel: echo 26–29 dB down with Alice's talker at
> a median within 3 dB (ducked up to 10 dB under Bruno's syllables); noise
> 13–14 dB down with syllables within 1 dB; quiet noise below a −40 dB
> threshold decoded as silence at both peers, no ring, microphone unmuted
> at the SFU; the gate adds 10 ms to a loud onset; steady noise learned and
> silenced under the defaults while syllables over it pass. All call
> fixtures now start from unprocessed settings (`qa_events.RAW_VOICE`):
> steady tones are what the processing removes (NS holds one 13 dB down; a
> loud steady far-end tone makes AEC3 suppress the near end even without an
> echo path, and keep suppressing it for 20 s or more after it stops).

**Files:** `desktop_call/audio.rs` (APM and VAD), `desktop_call.rs`
(`controlled_capture`, `AudioMode` gains `processing: {aec, ns, agc}` and
`input_gate: Auto | Threshold(dBFS)`).

**Approach:** Create one `AudioProcessingModule::new(aec, agc, hpf=true, ns)` per
call. Feed each mixed output frame to `process_reverse_stream`. Run each 10 ms mic
frame through `process_stream` before `capture_frame`. Defaults match Discord: AEC
on, NS on, AGC on. Until U2 there is no UI, so add a debug env override to switch
processing off for A/B tests.

The VAD gate computes per-frame RMS dBFS after processing. **Auto** uses an
adaptive noise floor plus margin. **Manual** uses a fixed threshold. A 200 ms
hangover avoids clipping word ends. Below the gate, send silence (DTX is already on
by default) without muting the publication, so the remote **Speaking** and mute
icons stay truthful. Publish a local input level (`watch` channel) for U2's meter
and drive the local **Speaking** state from it instead of the SFU's round trip.

Optional **A2b**: an RNNoise-based "Strong" noise suppression via the pure-Rust
`nnnoiseless`, at 48 kHz on 480-sample frames, which match our frame size.

**Verify:** add a `qa_voice_processing.py` mode to the private PulseAudio fixture
behind `--voice-room`:

- **Echo:** load `module-loopback` from Alice's speaker monitor into her virtual
  mic, with 60 ms latency and −6 dB, to simulate speakers next to a mic. Bruno talks
  at 670 Hz. With processing off (env override), Bruno must hear his own 670 Hz back
  above a set level, proving the fixture works. With AEC on, the returned 670 Hz at
  Bruno's speaker must drop by ≥25 dB for 12 consecutive windows after a 2 s
  convergence, while Alice's 410 Hz still reaches Bruno.
- **Noise:** mix pink noise into Alice's mic with a 410 Hz tone. The broadband noise
  floor measured at Bruno must drop by ≥10 dB with NS on, and the tone must stay
  within 3 dB.
- **Gate:** feed noise only, below the threshold. Bruno and Clara must decode
  silence, and the SFU must report Alice as not speaking. Raise the level and audio
  must pass within 100 ms. Alice's mic publication stays unmuted throughout.
- Element interop: `--desktop-dm` and `--incoming-dm` stay green. Element still
  measures 480 Hz. AGC must not drift a steady test tone by more than ±3 dB, or the
  fixtures need a speech-like signal instead of a pure tone.

### V2 — Simulcast, per-tile quality, pause hidden video (Phase 1, S-M)

> **Done** on branch `claude/v2-simulcast-9158cf` (2026-11-27), not merged yet.
> Cameras publish 320×180 / 640×360 / 1280×720, all at 30 fps, with dynacast.
> While the window's view is built, each call tile notes the feed it draws and
> the layer it needs (`Previews::draws`): the focused tile the top layer, a grid
> of up to six the middle one, thumbnails, larger grids and the dock preview the
> low one. Feeds it doesn't draw are hidden: the stage collapsed with the preview
> off, tiles scrolled out of the strip or grid (`sensor` and `on_scroll`), the
> media viewer open, the window closed to the tray. The call loops pause hidden
> remote tracks with `set_enabled(false)` and pick layers by **dimension**
> (`update_video_dimensions`), not `set_video_quality`: every ask replaces the
> track's subscriber settings at the SFU, `set_video_quality` sends
> `disabled: false` (unpausing) and `set_enabled` sends the publication's full
> size (the top layer); only a dimension ask carries both. The SDK sends each ask
> from its own task, so they can arrive out of order; one complete ask 300 ms
> after the last change has the last word (`room_video::Viewer`).
> `--room-camera` checks it with the debug build's stats: grid tiles receive
> 640×360 and the sender stops its unwatched top layer (dynacast, after about
> 8 s); a focused tile receives 1280×720 and the layer resumes while the other
> peer keeps 640×360; the dock preview receives 320×180; hiding it stops inbound
> bytes within 2 s; the reopened stage shows a pattern changed meanwhile in a
> screenshot started 0.10 s after the click began. Release builds check the SFU's
> three layers and the reopened stage. `--room-layout` (fixed: it never opened
> Alice's stage once calls started docked) asserts three GCM layers per camera
> and passes its grid and focus checks; `--element-video` passes, so Element
> decodes the encrypted simulcast camera. Untethered rendering Element's own
> simulcast camera isn't covered: the fixture's browser camera is 320×180, too
> small for livekit-client to add layers. `--voice-room` and `--window-share`
> (debug and release) still pass; shares hold 1596×928 at 30 fps.

**Files:** `room_video.rs` (publish options), `channel.rs` and `desktop_call.rs`
(`publish_mic` sets `RoomOptions { dynacast: true, .. }`, subscription quality),
`Previews` (feeds report their display size class).

**Approach:** Camera publishes simulcast with layers 320×180 / 640×360 / 1280×720.
Screen shares stay single-layer at first (MaintainResolution). Optionally add a
720p15 low layer later for thumbnails. `Previews` records which feeds are focused,
in the grid or in thumbnails. The call loops call `set_video_quality(Low | Medium |
High)` to match. When the stage is closed and the dock preview hidden, call
`set_enabled(false)` on all remote video, and re-enable on open. Turn on dynacast so
publishers stop encoding layers nobody watches. Leave `adaptive_stream` off and
drive quality explicitly; its automatic behavior relies on browser element sizes.

**Verify:** `--room-layout` (three clients) plus the new stats hook. Twirp
`ListParticipants` must show each camera track with three layers and GCM
encryption. Alice's inbound width for an unfocused thumbnail must be ≤640, and
≥1280 once focused, while both patterns still change in one screenshot per
observation, as today. Collapsing the stage must stop inbound video bytes within
2 s and restore pixels within 1 s of reopening. `--element-video` both directions:
Element must decode Untethered's simulcast camera with E2EE, and Untethered must
still render Element's simulcast camera.

### K1 — Global push-to-talk backend (Phase 1, M)

> **Shipped** on main as `40e3b8e` (2026-11-26): push-to-talk holds while another
> app has focus (`--room-ptt --global`); the Linux app ID is now
> `io.github.koalexbear.Untethered`.

**Files:** new `rust-client/src/hotkeys.rs` (portal plus X11 backends), a
subscription line in `main.rs`, and `desktop_call.rs` (`AudioMode::hold` gains a
release delay). Keep the current in-window handling as a fallback.

**Approach:** On Wayland, use ashpd `GlobalShortcuts`: `create_session`, then
`bind_shortcuts` for `push-to-talk`, `toggle-mute` and `toggle-deafen`, with
preferred triggers. The compositor shows its own binding dialog. `Activated` and
`Deactivated` become `TalkKey(true/false)`. On X11, grab a configurable key with
x11rb. Add a release delay, adjustable in U2 as Discord's is. Keep today's rule that manual mute or deafen overrides the key, and that a
focus change never reopens capture.

**Verify:**

- X11: a `--room-ptt` variant, `--global`, moves focus to the `xmessage` window
  *before* the hold and keeps it there. Both peers must decode Alice's tone only
  while the key is held. Today's focus-change assertions invert for global mode.
- Wayland: a `qa_kwin.py` variant without `--no-global-shortcuts` that starts
  `kglobalaccel` and `xdg-desktop-portal-kde` in the private session. Accept the
  binding dialog with inputsynth, focus a PySide decoy window, then send key
  press and release with inputsynth. Measure the peers with the 100 ms windows and
  ≤800 ms edge timing from `--room-ptt`. Also check that the bound key no longer
  types into the decoy window, and document that in the README.

### U1 — Voice Connected panel and user-panel Mute/Deafen (Phase 2, M)

**Files:** `view.rs` (`call_bar`, `call_dock`, the restyle's `user_panel`),
`view/icons.rs` (new glyphs: `Mic`, `MicOff`, `Headphones`, `HeadphonesOff`,
`ScreenShare`, `PhoneOff`, `Signal`). Replace `view/call_controls.rs` with the
icon set. Also `main.rs` (mute/deafen allowed before joining).

**Approach:**

- **Voice Connected panel** above the user panel on `TRAY`: `GREEN` "Voice
  Connected" with a `Signal` glyph (C4 adds the ping), "♪ Room / Space" as a
  `LINK` that opens the stage, and a `PhoneOff` disconnect icon on the right. Below
  it, two equal buttons: **Camera** (`Video`) and **Screen** (`ScreenShare`), green
  while live. Theater gets the same compact panel.
- **User panel:** add **Mute** (`Mic`/`MicOff` in `RED` when muted) and **Deafen**
  (`Headphones`/`HeadphonesOff`) before the existing `Gear`. Each has a hover
  chevron menu with Input or Output device lists, drawn on the restyle's `FLOAT`
  surface. Pre-join mute and deafen set the initial `AudioMode`.
- Remove Devices, Mixer, the PTT checkbox and the drawer from the call block. They
  move to U2 and U3. Keep status lines such as "Reconnecting…" and camera errors as
  one muted line under the header.

**Verify:** `--call-controls` (mute and deafen at both ends, including from another
room), `--call-devices` (selection through the chevron menus) and `--room-ptt`,
retargeted to the new controls through the QA control map. Before/after
screenshots go to `experiments/rtc-audio/evidence/` through `RTC_QA_SCREENSHOTS`.
Pre-join mute: join `--voice-room` muted. Peers must hear silence and the SFU must
report the mic muted from the first sample.

### U2 — Voice & Video settings page, persisted (Phase 2, M)

**Files:** a new `view/voice_settings.rs` opened from the user panel `Gear` (beside
the existing Account view), a new `rust-client/src/call_settings.rs` saved as JSON
in the profile directory (`$XDG_DATA_HOME/untethered-iced`, from
`backend/native.rs`), plus `call_devices.rs` and `main.rs`.

**Content:** Input and output device; input and output volume; **Mic test** (a
local loopback through A1's mixer with a level meter); **Input mode**: Voice
Activity / Push to Talk; sensitivity (Auto toggle, or a slider over the live A2
meter); PTT keybind (K1 binding dialog) and release delay; echo cancellation,
noise suppression (Off / Standard / Strong if A2b ships) and AGC toggles; a camera
picker with a live preview (an existing portal camera reader without publishing).
Settings apply to the next call and to the current one.

**Verify:** `--call-devices` chooses devices on the settings page before joining,
then checks routes and tones as today. Quit and relaunch on the same disposable
profile, as `qa_linux_upgrade.py` does across an upgrade, to confirm the choices
persist. A sensitivity
sweep reuses A2's gate fixture with the slider driven through the control map. The
camera preview must show the virtual camera pattern without any SFU publication
(Twirp `ListParticipants` stays empty).

### U3 — Participant rows and right-click user menu (Phase 2, S-M)

**Files:** `view.rs` (voice rows under channels), `view/call_stage.rs` (tiles share
the menu), `desktop_call.rs` and `channel.rs` (publish a per-participant snapshot:
mic muted, camera on, sharing, speaking), `main.rs` (messages).

**Approach:** Rows get an avatar with a 2 px `GREEN` ring while speaking, the name,
then right-aligned `MicOff` / `HeadphonesOff` in `RED`, `Video`, and a red
**LIVE** pill. Clicking **LIVE** opens the stage focused on that stream (U4 makes it
"Watch"). Right-click opens a `FLOAT` menu: **User Volume** slider (0–200 %, the
existing `PeerVolume`), **Mute** (local, which sets volume to 0 and restores the
previous value), **Disable Video** (unsubscribe that user's camera), and **Copy
user ID**. Iced 0.14 has no context-menu widget: use `mouse_area(..).on_right_press`
and an overlay anchored at the cursor, reusing the restyle's hover-toolbar
positioning.

**Limits:** remote mute is known from the LiveKit publication's mute flag, only
while we are in the call. MatrixRTC and Element do not signal deafen, so the deafen
icon appears only for Untethered peers if we add a participant attribute (see
Risks). Outside the call, rows show names and avatars only.

**Verify:** `qa_room_volume.py` / `--voice-room` drives the context-menu slider
instead of the Mixer, with the same 25 % / 150 % / 0 % / Reset level assertions.
The speaking-label sequence in `--voice-room` changes from OCR'd "Speaking" text to
sampling ring pixels around each avatar (green present or absent), keeping the
overlapping-speaker and mute cases. When Clara mutes, all three sidebars show her
`MicOff` icon within 1 s.

### U4 — Go Live sheet and Watch Stream (Phase 2, M)

**Files:** a new `view/go_live.rs`, plus `room_video.rs` (quality/fps parameters
from V1, `PersistMode::ExplicitlyRevoked` restore token), `share_audio.rs`, and
`call_stage.rs` (own preview, viewers, watch state).

**Approach:**

- **Screen** opens a sheet: source (*Screen or window*, which hands off to the
  portal picker, plus a *Share last selection* button when a restore token exists),
  **Resolution** 720p / 1080p / Source (cap 1440p) and **Frame rate** 15 / 30 / 60
  (60 requires C1 or warns about CPU), and **Audio**: None / an application stream
  (today's `AppStream` list, refreshed when the sheet opens) / Entire system (C2).
  Then **Go Live**.
- Live state: own preview tile with **Stop streaming** and **Stream settings**,
  which reopens the sheet and applies changes without a new portal prompt when
  possible.
- **Watch Stream:** remote screen shares are *not* subscribed by default. Tiles show
  the sharer's avatar with a **Watch Stream** button and the LIVE pill. Clicking it
  calls `set_subscribed(true)`. **Stop watching** reverses it. Screen-share audio
  follows the same choice. DMs auto-watch, matching Discord.
- **Viewer count is deferred.** Neither MatrixRTC nor LiveKit tells a publisher who
  subscribes. See Risks for the options.

**Verify:** extend `--window-share`: (1) Before anyone clicks Watch, Bruno's and
Clara's stats show no inbound screen track, and no pixels from the pattern appear.
(2) Bruno clicks Watch Stream. Only Bruno receives frames at the chosen
resolution and fps; Clara still receives nothing. (3) Stream settings switches
1080p30 → 720p15 without a new portal session: the same PipeWire node stays alive
and Bruno's inbound size and fps change. (4) Re-sharing with the restore token opens
no picker in KWin. The app-audio path from `--element-video --dm-window` is
re-driven through the sheet with the same tone assertions (730→830 Hz present,
1030 Hz absent).

### U5 — Incoming call card and desktop notification (Phase 2, S)

**Files:** `view.rs` (`incoming_banner` becomes a centered card), `incoming.rs` (a
display name from room state, `accept_video`), `notifications.rs` (a call
notification with actions when the window is unfocused, respecting DND).

**Approach:** The card shows the avatar with a pulse ring, the display name,
"Incoming voice/video call", and buttons **Decline** (`RED`, `PhoneOff`),
**Accept** (`GREEN`, `Phone`) and, for video invites, **Accept with video**. While
ringing, the DM header shows a "Calling…" strip, as Discord does. When the window is
unfocused, send a desktop notification with Accept/Decline actions. It closes when
the call is answered elsewhere, matching today's in-app behavior.

**Verify:** `--incoming-dm` (decline opens no microphone stream; accept opens one)
and `--multi-device-ring` use the new buttons. OCR finds the caller's *display
name*, not the MXID. Notification actions: reuse the private
`org.freedesktop.Notifications` server from `qa_room_alerts.py` on the fixture's
session bus. Invoke
**Decline** from the notification and assert the encrypted decline reaches Element,
with zero microphone streams.

### U6 — Stage polish (Phase 2, M)

**Files:** `view/call_stage.rs`, `view.rs` (stage header and theater), and `main.rs`
(`window::set_mode` for fullscreen).

**Approach:** Tile name badges gain `MicOff`. Hovering a tile shows the restyle's
hover toolbar: Focus/Unfocus, Fullscreen, a volume shortcut and Stop watching. Add
a **stage control bar** at the bottom center (Camera, Screen, Mute, Deafen,
Disconnect, all icons) so theater no longer overlays the 211-px dock. Header
buttons: **Chat** (today's *Open chat*), **Hide members without video**,
**Fullscreen**. Fullscreen sets the Iced window to `Fullscreen` and shows only the
stage; Esc exits.

**Verify:** `--room-layout` (focus by clicking a tile, then back to the grid) and
`--dm-window` (theater, own share selection), retargeted through the control map.
Fullscreen: the KWin window geometry equals the virtual output (2100×1000) and the
focused tile covers ≥90 % of the screenshot while the pattern changes. Esc
restores the earlier geometry.

### Phase 3 — capability

- **C1 Hardware encoding and codec** (`room_video.rs`): try
  `VideoEncoderBackend::Vaapi` / `Nvenc` / `Hardware` with a software fallback.
  Offer H.264 or VP9 for screens. 1080p60 depends on this. *Verify:* the
  stats hook reports the encoder implementation. Capture CPU for 60 s of
  `--window-share` at 1080p30 with software and hardware encoders. Element interop
  `--element-video --dm-window` decodes each codec with E2EE. The test VMs may have
  no VAAPI device, so the fallback path must be what the harness asserts.
- **C2 Whole-system stream audio** (new helper binary like `untethered-cameras`):
  build a PipeWire virtual sink and link every application output except
  Untethered's own call playback (the venmic approach), then record its monitor.
  *Verify:* extend the `--dm-window` two-app fixture. Element must hear both apps'
  tones (730 Hz and 1030 Hz) on the screen-share audio track and **not** its own
  960 Hz echoed back from the native speaker.
- **C3 Group DM calls** (`desktop_call.rs` `check_dm`, `incoming.rs`): use the
  voice-room media path for DMs with 3–10 members. Ring with
  `m.mentions.room` (the power-level check already exists in
  `Offer::valid_state`). *Verify:* a three-account `--voice-room` variant in a
  group DM. Both recipients ring, one declines, and the other two talk.
- **C4 Connection info** (`desktop_call.rs` stats summary, `view.rs` panel): RTT and
  packet loss from the selected candidate pair, colored by thresholds, with a
  popover graph. *Verify:* add delay and loss with `tc netem` on the
  `--call-reconnect` namespace's veth. The indicator must move to the expected
  bucket.
- **C5 Popout stream window:** requires moving from `iced::application` to
  `iced::daemon` for multiple windows. Large refactor; schedule by demand.

### Phase 4 — platforms

**P1 Windows (L).** Make `livekit` a dependency on Windows too; the SDK ships
Windows builds of libwebrtc. Behind the A1 `AudioIo` trait, use WASAPI (via `cpal`
or the `wasapi` crate) for mic and speaker. Cameras: Media Foundation (`nokhwa` or a
small `windows`-crate reader). Screens: libwebrtc `DesktopCapturer` (WGC/DXGI) or
`windows-capture`, with a custom picker since there is no portal. App audio: WASAPI
process loopback (Windows 10 2004+), which is how Discord does per-app audio.
Hotkeys: a low-level keyboard hook behind K1's interface. Signaling and membership
code is already platform-neutral apart from `cfg(target_os = "linux")` gates. Split
`desktop_call` into `media/linux` and `media/windows` adapters. *Verify:* there is
no Windows harness yet. Start with `scripts/rust-client.ps1` on a Windows VM
against a Linux-hosted Synapse+LiveKit, the `--desktop-dm` tone checks with a
virtual audio cable, and Element on Linux as the far end. This is the roadmap's
existing "Windows runtime testing is still open" item.

**P2 Web (L).** Two options. (1) Embed **Element Call** in an iframe positioned over
the Iced canvas, as gomuks's web UI does. Interop comes for free, but it is not our
UI. (2) Use **livekit-client** through `wasm-bindgen` with its E2EE worker
(insertable streams, or `RTCRtpScriptTransform` on Firefox), `getDisplayMedia`
for screens and tab audio, DOM `<video>` overlays for tiles, and the same Rust
membership/key logic over the engine's RPCs. Recommendation: (2), because it keeps
one call state model and the Discord UI. Use (1) as a stopgap only if web calls are
needed before P2 can be staffed. *Verify:* Playwright Chromium (as in
`scripts/qa_web_login.py` and `qa_browser_alerts.py`) with fake media devices, in
both directions against Element and the Linux desktop, using the existing tone and
pattern checks.

---

## 6. (d) Risks and unknowns

| Risk / unknown | Affects | Mitigation or first check |
|---|---|---|
| **Raw frames through a pipe at 1080p** (≈93 MB/s at 30 fps, 186 MB/s at 60, plus copies into `I420Buffer`) | V1, C1 | Measure CPU first. Alternatives: GStreamer `shmsink`, or libwebrtc's `DesktopCapturer` (its own portal and PipeWire, so no symbol clash, but a second picker flow and unknown DMA-BUF support). In-process gstreamer-rs would load system libpipewire next to libwebrtc's copy, the same clash `cameras.rs` works around |
| **Iced image path at 30 fps per tile** (new texture per frame) | V1 | Profile with the render-rate probe. V1b moves to a wgpu `shader` widget. tiny-skia keeps the CPU fallback |
| **AEC quality with pacat buffering**: the reference and actual output are offset by an unknown 40–80 ms | A2 | Found in A2: the timer-paced mixer could leave up to 680 ms queued in the 64 KiB pipe to `pacat` (whatever it wrote while `pacat` started or after a stall), and AEC3 then needed 20 s or more to converge. The playback pipe is now one page (≈43 ms), so `pacat` paces the mixer; the echo fixture measures real suppression. In-process libpulse could follow if needed |
| **AGC/NS distort the tone fixtures** | A2, all audio tests | Confirmed in A2: NS holds a steady tone ≈13 dB down, and a loud steady far-end tone makes AEC3 suppress the near end even with no echo path, and keep it suppressed long after (speech-like far ends don't). The voice-room fixtures save processing off (`qa_voice_room.RAW`); `--voice-processing` measures processing with syllables |
| **GlobalShortcuts portal UX**: a compositor dialog, binding may swallow the key (one user report about Discord), GNOME support only since 48 | K1 | X11 grab fallback. In-window fallback. README note. Test KDE in the private KWin with kglobalaccel; it currently runs with `--no-global-shortcuts` |
| **Simulcast + E2EE + Element** | V2 | livekit-client supports E2EE simulcast. `--element-video` must pass before shipping. If it fails, keep simulcast between native peers only and gate by peer |
| **Hardware encoders in the prebuilt webrtc-sys** (VAAPI/NVENC may not be compiled in; H.264 with E2EE needs NAL-aware encryption) | C1 | Read the backend from stats. Fall back to software. Test H.264 E2EE against Element before offering it |
| **Viewer counts / who is watching**: no MatrixRTC or LiveKit primitive. Participant attributes need `canUpdateOwnMetadata`, which lk-jwt-service may not grant. Data packets would need their own encryption | U4 | Defer. Spike: inspect lk-jwt-service grants. Otherwise count only Untethered viewers through Olm to-device "watching" notices |
| **Remote deafen state** is not part of MatrixRTC | U3 | Show mute only (from LiveKit). Optionally add an `io.untethered` membership field that only our clients render |
| **Rejoin after expiry** could duplicate a user or race the delayed leave | R1 | Always a fresh `created_ts` and fresh key. Assert no duplicate roster entries in `--room-reconnect` (c) |
| **Engine migration mid-plan** | R1 onwards | The `CallSignal` trait isolates signaling. Confirm matrix-rust-sdk delayed events, encrypted raw to-device and OpenID in the spike before porting |
| **UI conflicts with the uncommitted restyle** (`view.rs`, `view/*`, `main.rs`) | Phase 2, K1 | Phase 1 avoids view files and keeps `main.rs` edits to a few lines. Rebase after the restyle commit |
| **OCR-driven fixtures can't find icon-only controls** | Phase 2 | QA control map or tooltip-OCR before U1 |
| **Discord's free tier is 720p30**, while our default would be 1080p30 | V1, U4 | Bandwidth is the real limit on self-hosted SFUs. Default to 1080p30 with a 2.5 Mbps cap and let users pick 720p. Watch server egress in multi-viewer tests |
| **Windows and web have no automated call harness** | P1, P2 | Budget harness work into each project. Neither should claim parity from Linux results |

---

## 7. Out of scope for this plan

Soundboard, Activities, Krisp-licensed noise suppression, stage channels, Discord
connectivity and Nitro tiers. `ROADMAP.md` lists these under "Later, by demand".
Viewer counts wait for the U4 spike.

## Sources

Discord behavior was checked against current public material on 2026-11-25:

- [Discord Support: Go Live and Screen Share](https://support.discord.com/hc/hi-in/articles/360040816151-Go-Live-and-Screen-Share). Nitro up to 4K/60; browser streams can't change quality.
- [Discord blog: How it all goes live](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology)
- [OnStream: How to stream on Discord in 2026](https://onestream.live/blog/how-to-stream-on-discord-most-easy-steps/). Free tier 720p/30 fps (third-party).
- [OMG! Ubuntu, Nov 2026: Discord Linux app Wayland updates](https://www.omgubuntu.co.uk/2026/11/discord-linux-app-wayland-updates). Push-to-talk through the GlobalShortcuts portal.
- [GamingOnLinux, Nov 2026: Discord's Linux upgrades](https://www.gamingonlinux.com/2026/11/discord-go-into-more-detail-on-their-linux-upgrades-with-more-to-come/). Per-app audio still waiting on xdg-desktop-portal.
- [GamingOnLinux, Jan 2025: Wayland screen sharing with audio](https://gamingonlinux.com/2025/01/discord-screen-sharing-with-audio-on-linux-wayland-is-officially-here). Whole-system audio only.

Media-stack claims come from the pinned crate sources: `livekit` 0.12.0
(`room/options.rs`: `TrackPublishOptions`, `compute_appropriate_encoding`,
`screenshare::PRESETS`, `RoomOptions` defaults), `libwebrtc` 0.4.2
(`native/apm.rs`, `desktop_capturer.rs`, `rtp_sender.rs` `VideoEncoderBackend`),
and `ashpd` 0.11.1 (`desktop/global_shortcuts.rs`, `desktop/screencast.rs`).
