# CALL-01: native encrypted audio probe

This standalone crate tests the media path alongside the Linux desktop client.
The interoperability tests use **one Rust participant and one
unmodified Element Call widget**. The older two-Rust-peer test remains useful
for diagnosing transport problems independently of Matrix.

## Replay the Linux desktop DM call

Build the actual Iced app with `cargo build --locked -j2 --manifest-path
rust-client/Cargo.toml`, then run `sh experiments/rtc-audio/run_element.sh
--desktop-dm` with the environment variables shown below. In addition to
the common requirements, install Xvfb, xdpyinfo, xdotool, ImageMagick,
Tesseract, and PulseAudio (`pulseaudio`, `pacat`, and `pactl`). The test starts
a private X display and a private audio server with two null sinks. It clicks
**Voice call** in a live encrypted DM in Untethered, waits for Element's
incoming-call toast and ringtone, then accepts. It checks the 480 Hz desktop
mic at Element and the 960 Hz Element mic at the desktop speaker monitor,
checks both GCM publications at the SFU, waits past the first 60-second
membership expiry, confirms renewed membership and continuing audio, then
clicks the desktop **Hang up**.
It requires an empty remote call roster and no remaining desktop audio
streams. Neither test client needs physical audio hardware.

For the reverse direction, use `sh experiments/rtc-audio/run_element.sh
--incoming-dm` with the same setup. Element originates two distinct calls.
On the first, the desktop speaker monitor must detect the 440/620 Hz ringtone;
**Decline** must reach Element as an encrypted event referencing that exact
invite. A PulseAudio event subscription checks that no microphone stream
was created, even transiently. On the second, Iced **Accept** must open one
microphone; both distinct tones and both GCM publications are checked again.
Hanging up from Element must empty its roster and release both desktop streams.

### Verify answering on another device

Run `sh experiments/rtc-audio/run_element.sh --multi-device-ring` with the
same environment as `--incoming-dm`. The recipient has two independent
Linux devices on the same Matrix account. The second signs in and verifies
in its own Iced window. Each uses a separate X display, gomuks profile and
PulseAudio server. Element makes two encrypted DM calls.

* Both desktops must show **Accept**/**Decline** and play the 440/620 Hz
  ringtone. Decline the first call on the second desktop. Element must
  receive exactly one encrypted decline referencing that invite; the first
  desktop must stop ringing and show **Call declined on another device**.
* Accept the second call on the first desktop. Its partner must show
  **Call answered on another device**, remove the controls, close the
  ringtone stream and produce at least 600 ms of silence before the
  invite expires. An audio-server event subscription checks that it never
  opened a microphone, even briefly. Element’s membership list and the
  LiveKit GCM publication identities must include only the caller and the
  answering device; the test also measures decrypted 480/960 Hz audio and
  checks that hangup releases the streams.

Recorded Linux run: `DETZYHHXKF` and `MDHRJJGYTA` were distinct verified
devices of one recipient. The second device declined the first invite;
the first answered the next. Silence, closed playback and the outcome on
the non-answering device were verified 1.5 s after the click, with 87.3 s
remaining on the invite. No second decline appeared when that call ended.

### Verify DM video

With the Linux portal camera test prerequisites below, run
`sh experiments/rtc-audio/run_element.sh --dm-video`. Two independent KWin
desktops, PipeWire camera portals, synthetic cameras and verified Matrix users
exercise an encrypted DM. Neither desktop opens a camera before answer.
The caller requests video, the callee accepts with their camera off, and
then starts their own camera separately.

The test checks four GCM tracks at LiveKit, observes changing and distinct
camera pixels on both screens, and measures the decoded 410/670 Hz audio.
It swaps the self-preview and remote tile, then opens a second encrypted
room during the call. Remote video changes in the sidebar dock;
hiding and restoring it preserves both camera readers and all four track IDs.
The test types a draft in the second room, returns to the full DM stage, then
sends the preserved draft. The peer's gomuks database confirms it decrypted
in the second room only. With that room still open, stopping the peer camera
clears the picture while audio continues; hanging up releases both capture
processes and all tracks. This test
covers native Linux peers with portal cameras, not a physical webcam or
Element video interoperability.

### Verify video with Element

See also [the call-stage test](#verify-the-call-stage) for screen sharing
in both directions during a video call.

Use the Linux DM video test environment plus the extracted Element Web
distribution (`ELEMENT_WEB_DIR`). Run each direction with fresh devices:

```sh
sh experiments/rtc-audio/run_element.sh --element-video
sh experiments/rtc-audio/run_element.sh --element-video --incoming-dm
```

Untethered uses a Camera-portal-authorized PipeWire source on a private KWin
desktop. Element Web 1.12.35 runs its bundled Element Call widget in Chromium
with a different Y4M camera pattern. Each test checks the rendered remote
image on **both** screens, not a local preview or a published-track flag.
In both directions, two source patterns appeared at 982 pixels wide in
Element and 498 pixels wide in Untethered. LiveKit reported GCM on both
cameras and both microphones. Element measured the native 410 Hz mic on its
inbound audio track and Linux measured the 670 Hz return tone at its speaker.

Turning off the Element camera must remove its stale picture in Untethered
and show **No incoming video** while the native camera stays on. Turning it
back on must deliver fresh video with the original microphones. Stopping the
native camera must release its capture process, and hangup must empty the
MatrixRTC roster and release the media tracks.

### Verify the call stage

```sh
sh experiments/rtc-audio/run_element.sh --dm-window
sh experiments/rtc-audio/run_element.sh --element-video --dm-window
```

Use the DM video environment above. For Element, put `Xvfb` on `PATH`:
Chromium runs on its own private display and captures a fixture tab through
Element's **Share screen** button. Neither client captures the host desktop.

Both cameras and both microphones start first. Cancelling the KDE chooser
must leave them alone; selecting **Prism** adds a fifth GCM track. Both the
sharer and the recipient must render the selected content. The sharer clicks
a different participant, changes Prism, reselects **Your screen** and enters
**Theater**. The recipient continues to see Prism, and fresh audio travels in
both directions. Stopping the share closes its portal grant and reader,
while the call keeps its original four camera and microphone publications.
During the Element run, the native side sends two independent apps through
one private output sink. Selecting **Prism** must deliver its 730→830 Hz
signal on an additional GCM screen-share audio track. Element separately
decodes the 410 Hz microphone and checks for the unselected app’s 1030 Hz
signal on all inbound audio tracks. The test stops app audio, restarts it,
exits the selected app, and deliberately selects its replacement. Neither
source disappearance nor window stop may fall back to recording the sink.
Check directly with either native audio server:

```sh
sh experiments/rtc-audio/run_element.sh --element-video --dm-window --incoming-dm
RTC_QA_AUDIO_SERVER=pipewire-pulse sh experiments/rtc-audio/run_element.sh --element-video --dm-window
```

Element also shares a changing browser tab with audio back to Untethered.
The tab plays 1030 Hz independently of the 670 Hz Element microphone. The
native speaker must reproduce both through its one playback stream, then only
the microphone when **Play shared audio** is unchecked. With the tab changed to 830 Hz, **Deafen** and
**Undeafen** must preserve that choice. Re-enabling share playback must recover
the new tab tone. Stopping the share must keep that playback stream for the
microphone and return the stage to the peer's camera without replacing the microphone tracks.
Chromium and Untethered use separate private PulseAudio servers; sound cannot
reach the listener through a local desktop loopback.

The [normal stage](evidence/own-share-stage.png) and [theater view](evidence/own-share-theater.png)
are screenshots of the running client. Set `RTC_QA_SCREENSHOTS` to an output
directory to save new screenshots of the shared test board.

### Verify live camera switching


Build both Linux binaries (`cargo build --locked --manifest-path rust-client/Cargo.toml`)
and run the following in the Element video test environment:

```sh
sh experiments/rtc-audio/run_element.sh --switch-camera
sh experiments/rtc-audio/run_element.sh --switch-camera --incoming-dm
```

The desktop enumerates camera nodes using a
small companion process with a portal-granted FD, independently of
libwebrtc’s private PipeWire symbols. The packaged helper lives alongside
gomuks under `/usr/lib/untethered`; the dev launcher builds it automatically.

The fixture adds a second virtual camera after the first has begun
publishing. During one encrypted call, it selects **Cobalt**, then **Amber**
in the actual Devices picker. At both selections the private PipeWire graph
must show only the selected camera linked, Element's remote tile and
Untethered's self-preview must both show the new changing pattern, and the
same four GCM camera/microphone publication IDs must remain active. Withdrawing
the selected Cobalt source must remove only that camera publication and leave
no capture link. Selecting Amber while off must not start capture until
**Start camera**; the same call then resumes video with both microphones
still carrying audio. These are virtual-camera measurements, not hardware QA.

### Verify three-person voice rooms

Build the Linux desktop and use the same desktop test requirements above:

```sh
LIVEKIT_SERVER=/path/to/livekit-server LK_JWT_SERVICE=/path/to/lk-jwt-service \
  PYTHON=/path/to/qa/venv/bin/python sh experiments/rtc-audio/run_element.sh --voice-room
```

This test starts three Untethered processes with different verified accounts,
separate displays and dedicated virtual microphones/speaker monitors. It
clicks **Join voice** in the encrypted *Atrium* call room. The first two must
exchange audio before the third joins. With all three present, every speaker
must contain the other two frequencies and exclude its own for twelve
consecutive 100 ms windows. The test checks one mixed playback stream per
client, three GCM microphone publications, the names under the channel, and
the room history (no non-state timeline messages, including encrypted rings).
With all three mics still active, the test opens Alice’s **Mixer** and drives
the rendered sliders. It measures each sender’s frequency at every receiver’s
speaker monitor for twelve fresh 100 ms windows per setting. At 25%, only
Bruno’s tone at Alice should drop to one quarter of baseline. Boosting Clara
to 150% on Alice must leave Bruno there at 25% and both other listeners
unchanged. Setting Bruno to zero must silence only him for Alice; **Reset**
must restore the baseline mix. The three GCM microphone track IDs must survive.

It then feeds one microphone at a time, with silence between speakers. On
each of the three rendered sidebars, the green speaking ring must surround the
correct person’s avatar in the voice roster and disappear during silence,
while the participants and microphone track IDs stay the same. An overlapping
pair must show two rings; when only one falls silent the other ring must
remain. Muting the remaining speaker while her test input is still loud must
clear the last ring, and unmuting must restore it. Each step checks the tones
received by all three clients.
After one leaves, the remaining two must exchange newly injected tones on
their original participant and track IDs. Final leave must clear call
memberships and release all capture/playback streams.

### Verify push-to-talk

Run `sh experiments/rtc-audio/run_element.sh --room-ptt` with the three-client
environment above and `xmessage` available. Alice enables **Push-to-talk**
before joining. The fixture drives physical Right Ctrl press/release events
on her isolated X display. Both peers’ speaker monitors must decode her
410 Hz tone while held, then a fresh 530 Hz tone during the same hold.
Key-up must silence just Alice and remove her capture stream while her
input tone keeps playing. Each measured key edge must settle at both
receivers within 800 ms (includes 100 ms speaker-analysis windows).

The test moves focus to another window mid-hold, returns focus with the key
still held, and separately delivers key-up to the other window. Neither case
may reopen capture. Manual mute and deafen override the key and require a new
press afterwards. Every consecutive 100 ms window across the exercise is
checked for Bruno’s tone at Clara and Clara’s at Bruno. Their original GCM
tracks must remain in place. This fixture covers an in-window key, not a
desktop-wide shortcut.

### Verify portal window sharing

Use the three-client setup above, plus `pipewire`, `wireplumber`, `pw-dump`,
`busctl`, GStreamer's PipeWire and base video plugins, PySide6 for the two
test windows (`/usr/bin/python`), and the [isolated KWin prerequisites](../../scripts/kwin-element.md).
Run `sh experiments/rtc-audio/run_element.sh --window-share` with
`KWIN_INPUTSYNTH` pointing to the installed KDE test input helper. No Element
Web archive is needed. Only the sharer runs on KWin; the receivers each have
a private X display. The virtual compositor, PipeWire server and session bus
are local to the fixture.

The test cancels a real KDE picker, then selects **Prism** from a fresh picker
that lists the virtual screen and every window, with both Prism and a decoy
window visible. Prism is 1600×900 with a bar sweeping its bottom band at 60 Hz,
because KWin only sends frames when the window changes. The receivers open
**Open call view**; both screenshots must contain the selected window's adjacent
pink/teal fields at video-tile scale.

Debug builds started with `UNTETHERED_QA_VIDEO_STATS=1` print one line per video
track on every 4 s heartbeat, from `Room::get_stats()`: direction, source,
frame size, frame rate and bytes. They contain no addresses, keys or SDP. The test requires
the sender's outbound screen share to be at least 1280 px wide at 24 fps or more,
and both receivers' inbound tracks to reach 24 fps. Each client gets its own four cores, as if
on separate machines. Measured on 2026-11-25 (KWin 6.8.3, libvpx VP8,
software GL on the receivers' Xvfb displays):

| Measurement | Result |
|---|---|
| Portal window size | Not reported by KDE; probed from PipeWire caps: 1600×928 (with title bar) |
| Sender capture / encode | 30.0 fps / 1600×928 at 29.0–30.0 fps, no quality limitation |
| Receivers decode | 1600×928 at 30.0 fps on both |
| Receive conversion (I420 → RGBA + `image::Handle`) | 30.0 fps at 0.5–1.2 ms per frame |
| Sender CPU (100% = one core) | Client 18–22% (encoder 9–12%), GStreamer 7–8%, KWin 14–15% |
| Receiver CPU, idle stage → 30 fps share | 2–5% → 102–117%, of which the UI thread is 93–103% |

Two runs, both passing. Before this change the same path captured 960×540 at
up to 12 fps and LiveKit encoded it at 5 fps. A 60 Hz source now reaches
GStreamer at 60 fps and is cut to 30 there.

The per-frame `image::Handle` itself is cheap. The receiving UI thread is the
bottleneck: each new frame is a new image to upload and redraw, and it
saturates one core under software GL. Persistent YUV textures converted on
the GPU (plan step V1b) are the next step for the receive side.
After the sharer navigates to a text channel, the test changes the source
to blue/orange and injects new mic tones. It checks the new pixels at both
receivers, all six decoded audio paths, the original microphone IDs and a GCM
`SCREEN_SHARE` publication. **Stop sharing** must remove the portal session,
restore the PipeWire video graph to baseline, unpublish the video and clear
both receiver tiles while the voice room stays connected.

### Verify voice-room camera video

Use `sh experiments/rtc-audio/run_element.sh --room-camera` with the same
three-client and [private KWin setup](../../scripts/kwin-element.md). The fixture
provides a virtual camera using GStreamer's `rawvideoparse` and `pipewiresink`
on its own PipeWire server; it does not open a physical webcam. The sender can
reach the camera only through the real Camera portal's file descriptor.

The test starts with no camera, then cancels a pending permission request and
checks that no capture process, PipeWire link or SFU video track exists. On a
fresh grant it verifies an active link to the virtual camera and a GCM
`CAMERA` publication. The sender's video stats must show the camera sent at
1280×720; on 2026-11-25 it was sent at 1280×720 and 12.0 fps, the virtual
camera's own rate. Screenshots of both peers must show the green/magenta
test fields, then fresh orange/blue fields on the same video track. **Stop
camera** must remove the capture process, camera links, SFU video track and
both remote tiles while leaving the virtual camera available. Changing mic
tones verifies all six audio paths both during video and after stopping it.

### Verify multi-camera grid and focus

Run `sh experiments/rtc-audio/run_element.sh --room-layout` with the same
KWin camera prerequisites. Bruno and Clara each have a private compositor,
session bus, Camera portal and virtual PipeWire camera. Alice views both feeds
on a third native desktop. The fixture reads Alice's Iced captions, clicks
the actual tiles and measures two adjacent color fields per camera in **one
screenshot per observation**. Both patterns change in the grid and again in
each focus view. The selected feed must be visibly larger and the thumbnail
must show the other camera's fresh pixels. Both GCM camera track IDs and all
three original microphone IDs must survive switching.

All three speaker monitors are checked in consecutive 100 ms windows for the
whole layout-switch sequence. Clara then leaves with her camera still focused.
Her capture link and reader must disappear; Alice must return to the grid,
see a new pattern from Bruno on his original video track, and hear his new
mic tone while Clara's speaker stays silent.

### Verify mute and deafen

Use `sh experiments/rtc-audio/run_element.sh --call-controls` with the same
setup. The test clicks **Mute**, switches to another room, and uses the active
DM's call bar there. It tests deafen/undeafen both with and without a prior
manual mute. Each transition requires 12 fresh 100 ms samples at **both**
ends: a through-tone or silence (Element FFT peak below −65 dB; desktop peak
below 32 in signed 16-bit PCM). It also checks the actual capture-stream
count and the SFU mute flags. Throughout the test the browser receiver, track,
SSRC and signaling sockets stay the same, and both participants keep their
original GCM publications on the SFU. Last, it opens the dock's **Mixer** and
sets Element to 25%: Element's 960 Hz at the desktop speaker must drop to a
quarter of a steady reference for 12 fresh 100 ms windows, still in one
playback stream, and **Reset** must restore it.

### Verify microphone and speaker selection

Run `sh experiments/rtc-audio/run_element.sh --call-devices` with the same
setup. Two virtual microphones continuously supply **480 Hz** and **730 Hz**,
and both virtual speaker outputs are monitored for Element's **960 Hz**.
The test clicks the Iced **Devices** selectors. After each switch it requires
12 fresh 100 ms windows at Element and at each speaker: the chosen mic tone,
the return tone at the chosen output, and silence at the other. It checks
PulseAudio stream routes, unchanged server defaults, the original two GCM
publication IDs and participant IDs, and browser media/signaling continuity.

It also selects a spare mic while muted and removes it before unmuting.
There must be no fallback capture: Element stays silent, playback continues,
and explicitly choosing a working mic restores audio on the same track.

### Verify recovery after a network interruption

For the desktop network recovery test, run
`sh experiments/rtc-audio/run_element.sh --call-reconnect` with the desktop
prerequisites, plus `nsenter`, `ss` with socket-destroy support and `nft`.
The runner gives Untethered and its gomuks child their own network namespace
and veth; Element, Synapse and LiveKit stay online in the outer test namespace.
It takes the link down for at least 6.5 s and discards the desktop's established
SFU signaling socket. The test observes **Reconnecting**, no arriving RTP,
silence and closed desktop audio devices. It changes the test mic from 480 Hz
to 730 Hz while disconnected. After link-up it requires 12 fresh samples of
the new tone at Element and 12 of the 960 Hz return tone at the desktop, both
GCM tracks, unchanged MatrixRTC membership and exactly one ring event. These
are short-outage checks; they do not exercise TURN or resume an expired call.

### Verify TURN-only calls

#### Two native Linux desktops

Build `rust-client` in debug mode so that selected ICE statistics are available.
With `PYTHON`, `LIVEKIT_SERVER` and `LK_JWT_SERVICE` as below, run:

```sh
sh experiments/rtc-audio/run_element.sh --native-dm
sh experiments/rtc-audio/run_element.sh --unsupported-dm
```

The first fixture uses two disposable accounts, one Iced window per private
X display, and a different network namespace for each client and its gomuks
engine. Only Matrix/SFU signaling and coturn UDP are reachable. Independent
virtual speaker monitors distinguish the two injected microphone tones from
one another and from silence in successive 100 ms windows.

* Cancel before answer, decline on the other device, and let a third ring
  expire unattended. The recipient must decrypt each ring referencing the
  caller’s actual membership event. All three attempts together must open
  zero microphone streams and leave no active membership.
* Call and accept in both directions. Require the two expected GCM microphone
  publications, decoded speech tones at both speakers, and selected relay/UDP
  pairs on the actual inbound and outbound audio transports of **both** clients.
* Switch rooms, mute and deafen during the first call. Cut just the relay
  flow: both outputs must fall silent. Restore it: both must receive *new*
  tones without changing track IDs. For the reverse call, interrupt the
  caller’s entire network link. Check **Reconnecting**, automatic return
  of fresh audio, and no second invitation. Each hangup must release the
  devices, dismiss the call controls and clear both Matrix memberships.

`--unsupported-dm` runs against Synapse without an advertised MatrixRTC
transport or a running SFU. Clicking **Voice call** must report the
unsupported server and return the caller to the DM, without an invitation,
slot, membership, capture or playback. It does not require LiveKit or Element.

Recorded run, 2026-11-25: the two native devices were `TOQYBQFIQH` and
`OWWFXUYZKU`. The canceled, declined and unanswered attempts had **zero
microphone starts**; the caller displayed **No answer** 45.3 s after
the encrypted invitation’s sender timestamp (advertised lifetime 45 s).
On the first answered call each firewall blocked direct SFU media packets
(22 and 217); both speakers recovered fresh 830/530 Hz on the original GCM
tracks after TURN was restored. On the reverse call, after the caller's
6.5-second link outage, both speakers reproduced those fresh tones within
7.6 s of link restoration, without redial. Matrix history on both devices
contained five distinct encrypted invitations and one matching decline.

#### Linux desktop and Element

With the debug Linux desktop build and desktop test environment above, install
`coturn`, `nftables`, iproute2 and `nsenter`. Run the GUI tests one at a time:

```sh
sh experiments/rtc-audio/run_element.sh --call-turn
sh experiments/rtc-audio/run_element.sh --call-turn --incoming-dm
```

The real desktop/gomuks and Chromium/Element clients run on separate private
subnets. Before dialing, each client gets default-deny IPv4/IPv6 firewall
rules: only local IPC, the test services' TCP signaling ports and authenticated
UDP TURN at `192.0.2.3:3478` are reachable. LiveKit advertises a disposable
coturn 4.19.2 instance; each firewall counts drops to the SFU's direct media
ports. The runner does not force either client's ICE policy.

At both clients, active inbound/outbound RTP stats are traced through their
transport's `selectedCandidatePairId` to the **local relay candidate**. The
Rust debug build's opt-in `UNTETHERED_QA_ICE_STATS=1` diagnostic reports only
direction, candidate type, relay protocol and packet count. The assertion
requires `relay/udp` in all four cases, with live traffic at both firewalls.
Element must decode 480 Hz and the Linux speaker must render 960 Hz. Blocking
only TURN on the desktop must silence **both** while the call stays joined.
After unblocking, the runner verifies a newly injected 730 Hz at Element and
960 Hz at the desktop for 12 fresh 100 ms windows, with GCM on both microphone
publications. This covers TURN/UDP, not TURN/TCP or TURN/TLS.

## Replay the Element ↔ Rust encrypted audio test

From the checkout, with Chromium, OpenSSL, iproute2, the pinned gomuks backend,
and the Python dependencies used by the membership check below:

```sh
cargo build --locked -j2 --manifest-path experiments/rtc-audio/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  LIVEKIT_SERVER=/path/to/livekit-server \
  LK_JWT_SERVICE=/path/to/lk-jwt-service_linux_amd64 \
  PYTHON=/path/to/qa/venv/bin/python \
  sh experiments/rtc-audio/run_element.sh
```

Verified with LiveKit 1.13.11 and `element-hq/lk-jwt-service` 0.10.0. The
runner starts a private network namespace, Synapse, the SFU, its authorization
service, and two disposable accounts. Element's **Voice call** button opens
the bundled widget with a 960 Hz WAV as its test microphone. On Linux the
Rust peer joins the same MatrixRTC slot, requests an OpenID token from gomuks
and exchanges it at the advertised `/sfu/get` endpoint. The authorization
service verifies the token against Synapse over TLS and issues the SFU JWT.
Neither peer is provisioned with the other's media key.

Both peers publish GCM-encrypted audio using per-participant keys and HKDF.
The Rust peer accepts Element's key only from an encrypted to-device event
matching the room, sender and participating device/member. The test requires:

- Rust decodes Element's **960 Hz** tone across 20 consecutive PCM frames.
- With Rust's key withheld, the widget receives at least **60 RTP packets**,
  with no sustained **480 Hz** tone in its inbound audio.
- After gomuks sends the key to that Element device, the widget measures
  **480 Hz** for ten consecutive spectra of its received WebRTC track.
- During that same call, Rust installs a fresh sender key at index **1**,
  selects it on the existing frame cryptor, and submits **730 Hz** on the same
  microphone publication. With key 1 withheld, the test requires at least
  **80 additional RTP packets** and **18 additional spectra** with no 730 Hz
  detection. After gomuks delivers key 1 over Olm, Element must detect the new
  tone for ten consecutive spectra. The browser checks the same receiver,
  track, SSRC and open SFU signaling socket throughout; Rust checks the same
  GCM publication and no reconnect or republish events.

This is a synthetic microphone on a local ICE network. It does not exercise
real capture/playback hardware, TURN, incoming DM ringing, or a rotation
initiated by Element.

## Replay the two-Rust-peer test

Install the build prerequisites for the LiveKit Rust SDK (including Clang, CMake,
and a C++ toolchain for the underlying WebRTC library). Put a `livekit-server`
binary on disk. The verified test used server 1.13.11.

```sh
cd experiments/rtc-audio
cargo build --locked -j2
unshare -Urn sh -ec '
  ip link set lo up
  ip link add rtc-test type dummy
  ip addr add 192.0.2.2/24 dev rtc-test
  ip link set rtc-test up
  RTC_PROBE_ICE_IP=192.0.2.2 python3 run_local.py "$1"
' probe /absolute/path/to/livekit-server
```

The isolated namespace needs an ICE interface other than bare loopback. The
runner starts the test SFU, creates short-lived test tokens for a unique room,
then stops the server. It does not use the homeserver or its credentials.

## Replay the MatrixRTC membership check

With the Python and extracted Element Web prerequisites from the
[Element test setup](../../scripts/kwin-element.md), run from the checkout:

```sh
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow --with cryptography \
    python scripts/test-media-interop.py --rtc-membership
```

This test uses two disposable accounts in an encrypted room and no compositor
or SFU. The Rust client uses its native backend and authenticated gomuks RPCs.
A Chromium instance running Element reads **its own MatrixRTC participant
list**. It checks the native user, device, member ID, room-wide voice slot and
advertised transport. Synapse's eight-second delayed leave is renewed; the
observer verifies the member survives the original deadline and disappears
on explicit leave. A second join ends by disconnecting the native backend:
Element sees the roster empty and Synapse confirms the empty state event.

The probe implements the MSC3401 session membership that this Element release
accepts in numbered room versions. It has a one-minute upper bound, even if the
client dies before arming the delayed leave. The desktop renews the lease on
longer calls. The advertised transport in this membership-only check is a
sentinel, so a roster entry does not demonstrate a media connection.

## What comes next

Element and Untethered can each initiate DM calls while both apps are open.
The isolated relay test covers both caller directions. Physical hot-unplug
behavior and Windows runtime testing are not covered by the virtual-device tests.
