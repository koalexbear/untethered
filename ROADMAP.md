# Untethered: roadmap to Discord parity

**2026-11-24 · `rust-discord-ui` at `bfa75b3`**

**Next: drag-and-drop attachments and voice calls.** Start the technical proof for both now. Ship files first while proving encrypted call interoperability with Element. This is a delivery order, not a calendar estimate.

## First implementation checkpoint

- **Files available:** Native and web staging tray, multi-file drop/picker, explicit Send,
  per-room drafts, optional caption, sequential encrypted uploads and attachment save.
  Verified a real Dolphin drag between native Wayland windows in an isolated compositor.
  The native live test uploads and saves a byte-identical binary; a separate Chromium
  device also decrypted that file. Element Web 1.12.35 displayed the encrypted attachment
  sent by the browser client. Windows cross-builds; Explorer runtime testing is still open.
- **Composer previews:** local image thumbnails and compact file cards on native and web.
- **Received images:** verified inline encrypted thumbnails on native and Chromium;
  an independent browser device rendered the expected image after both login and reload.
  `scripts/test-media-interop.py` reruns the check with disposable accounts.
- **Native screenshot paste:** verified real Ctrl+V on Wayland and X11 with
  `scripts/test-media-interop.py --native-paste {wayland,x11}`. Each had zero
  uploads/events while staged and one upload/encrypted image event after Send;
  downloaded ciphertext decrypted to the exact test pixels and the pasted text caption.
- **Progress, Cancel, Retry:** desktop consumes gomuks's progress stream; the
  browser worker uses actual XHR upload events. `scripts/test-media-interop.py
  --upload-cancel` throttles encrypted 16 MiB files: both transports closed
  mid-body with zero new room events; each retry delivered exactly one.
- **Portal-only Wayland drop:** `scripts/test-media-interop.py --portal-drop` uses
  a GTK sender with no URI list and a real private `xdg-document-portal`. Switching
  rooms during retrieval and upload left the other room untouched. The original
  encrypted room received exactly one PNG and one PDF; both decrypted byte-for-byte.
- **KWin/Dolphin ↔ Element:** [repeatable runtime test](scripts/kwin-element.md)
  passed with KWin 6.8.3, Dolphin 26.08.3 and Element Web 1.12.35 on a second
  Matrix account. One PNG and one PDF stayed local until Send. Switching rooms
  mid-upload produced two encrypted events in the original room, none in the
  other. Element saved both and sent those bytes back; the native KDE Save
  dialogs produced files identical to the originals.
- **Unencrypted rooms and upload boundaries:** [Linux and Chromium ↔ Element](scripts/kwin-element.md#verify-plain-rooms-and-upload-limits)
  each round-tripped a 437-byte PNG and a 65,536-byte PDF on a homeserver
  advertising a 65,536-byte limit. Both named and skipped a 65,537-byte
  file, leaving the valid pair ready. Each run produced exactly four uploads
  and four ordinary `m.room.message` events across both directions; both
  saved originals matched byte-for-byte. Chromium also kept a caption local
  when the only attachment was rejected.
- **Files still to do:** Windows Explorer runtime QA remains deferred. The
  Linux/Chromium checks do not establish Windows file interoperability.
- **Calls started:** a user-triggered MatrixRTC transport check in Account, and a
  [repeatable native audio probe](experiments/rtc-audio/README.md). Two Rust peers
  decoded each other's encrypted audio through a test SFU. Separately, Element
  Web 1.12.35 recognized the Rust/gomuks device in its voice-call participant
  list, kept it through a heartbeat, and removed it on both explicit leave
  and backend disconnect (`scripts/test-media-interop.py --rtc-membership`).
  The **Linux Rust ↔ Element Call audio test** now passes with the embedded
  Element Call widget, gomuks Olm sender-key exchange, and OpenID authorization
  through lk-jwt-service 0.10.0. Rust measured Element's 960 Hz on decoded PCM;
  Element received encrypted RTP without decoding Rust's 480 Hz until the key
  arrived, then measured it on the inbound track. The Rust probe also rotated
  its sender key mid-call: Element received 92 further RTP packets with no new
  tone before key 1 arrived, then measured 726.6 Hz for the 730 Hz test tone
  on the same track and SSRC, without reconnecting.
- **Outgoing Linux desktop DM call:** Clicking **Voice call** in Iced rings
  Element Web 1.12.35. Element displayed the incoming DM and started its
  ringtone; desktop capture stayed closed until acceptance. After **Join**,
  Element measured the desktop microphone's 480 Hz and the desktop speaker
  monitor measured Element's 960 Hz. The SFU independently reported GCM on
  both microphone tracks. Audio survived the first 60-second membership lease.
  Clicking **Hang up** in Iced emptied the Element roster and released both
  audio streams. Replay with `run_element.sh --desktop-dm` in
  [the RTC test guide](experiments/rtc-audio/README.md). Input/output were
  private PulseAudio devices.
- **Incoming Linux desktop DM call:** `run_element.sh --incoming-dm` starts
  two calls from Element. On the first, Iced rang audibly and **Decline**
  delivered one encrypted decline referencing the invitation; no capture
  stream opened. On the second, **Accept** opened one microphone, transmitted
  480 Hz to Element and played Element’s 960 Hz. LiveKit reported GCM on
  both audio tracks. Hanging up in Element cleared both devices and released
  desktop audio.
- **Answered on another device:** [Two-device Linux DM test](experiments/rtc-audio/README.md#verify-answering-on-another-device)
  rang two verified devices of the same account from Element. Declining on
  one silenced the other. On the next call, answering on the first cleared
  the second device’s prompt and ringtone within 1.5 s, with 87.3 s left on
  the invitation and zero microphone opens on that device. Only Element and
  the answering device published GCM tracks; both decoded the other’s audio.
- **Mute and deafen:** `run_element.sh --call-controls` exercised the Linux
  call bar during a live encrypted Element DM, including after switching to
  another room. Muting stopped capture and silenced the desktop's 480 Hz at
  Element while the 960 Hz return tone continued. Deafening silenced both;
  undeafening preserved an explicit mute and restored an open mic when it
  had been on. Both original GCM track IDs and the connection survived all
  six transitions.
- **Linux device selection:** In `run_element.sh --call-devices`, Element heard
  480 → 730 → 480 Hz on the same desktop audio track when the microphone
  changed. Moving its 960 Hz return audio between two speaker sinks silenced
  the unselected output. Both encrypted track IDs and the connection stayed
  intact; the system defaults were unchanged. Removing a selected mic while
  muted left capture closed on unmute; selecting a working input restored it.
  Physical hot-unplug remains unverified.
- **Linux network recovery:** `run_element.sh --call-reconnect` disconnected
  the desktop's private link to Matrix and LiveKit for 6.5 seconds while
  Element stayed online. RTP stopped, both ends went silent and the call bar
  showed **Reconnecting**. After restoration, Element decoded a new 730 Hz
  tone and the desktop played 960 Hz. Both microphones used GCM; the same two
  Matrix devices stayed in the call and the timeline contained one ring.
  Hangup released both streams.
- **TURN-only Linux ↔ Element calls:** [The relay test](experiments/rtc-audio/README.md#verify-turn-only-calls)
  blocks direct ICE before dialing, in separate namespaces for the desktop
  and Element Web 1.12.35. With either client initiating, both selected
  `relay/udp` for incoming and outgoing audio and both microphone tracks
  used GCM. Blocking just coturn silenced both ends; restoring it delivered
  a new 730 Hz desktop tone to Element and the 960 Hz return tone to Linux.
  TURN/TCP and TURN/TLS were not exercised.
- **Linux Untethered ↔ Untethered DM calls:** [Two-desktop test](experiments/rtc-audio/README.md#two-native-linux-desktops)
  verified both caller directions across separate TURN-only networks. Cancel,
  decline and no-answer opened zero microphones. The caller displayed **No
  answer** at 45.3 s for a 45-second invite. Both answered calls carried
  distinct decoded audio on GCM tracks; the first kept working while changing
  rooms, muting and deafening. After a full link interruption on the reverse
  call, fresh audio returned on both speakers in 7.6 s without another ring.
  Hangup released all streams and memberships. On a homeserver without RTC
  discovery the caller saw an explicit error and sent no invite or membership.
- **Two-way Linux DM video:** [The video-call test](experiments/rtc-audio/README.md#verify-dm-video)
  rang a real encrypted DM with video intent. After acceptance and separate
  camera grants, both desktops rendered the other camera's changing pattern
  at 498×280, alongside the local preview. Swapping focus kept the GCM track
  IDs unchanged. A second encrypted room showed changing remote video at
  160×90; hiding/restoring the preview and returning to the full stage
  preserved both readers, all four publications and the other room’s draft.
  The recipient decrypted the message only in that other room.
  Stopping one camera preserved the other video and both
  audio paths; hanging up released both readers and all publications.
- **Linux ↔ Element DM video:** [Both caller directions](experiments/rtc-audio/README.md#verify-video-with-element)
  exchanged two changing remote camera patterns (982 px wide in Element,
  498 px in Untethered) and 410/670 Hz audio with four GCM publications.
  Turning off Element’s camera cleared the remote image in the native UI;
  turning it back on restored video without redial. The call roster and all
  publications cleared on hangup. These runs used synthetic cameras.
- **Live Linux camera switching:** [Camera picker test](experiments/rtc-audio/README.md#verify-live-camera-switching)
  switched two portal-authorized camera sources during an encrypted DM with
  Element. Both the self-preview and remote picture changed; both camera and
  microphone publication IDs stayed unchanged. Removing the selected virtual
  source stopped only its video. No fallback camera opened, and selecting and
  starting the remaining source restored video without a redial.
- **Linux call stage and selectable self-share:** [Screen-share acceptance](experiments/rtc-audio/README.md#verify-the-call-stage)
  rendered the sender’s own selected window at 871 px wide, switched to the
  peer and back, and enlarged the share in theater. Element rendered both
  patterns at 924 px wide and shared its own changing tab back. Both cameras
  and both microphones kept their encrypted track IDs throughout.
- **Incoming shared audio on Linux:** Element shared a tab with an independent
  GCM audio publication. In a DM, the native speaker reproduced the tab and
  microphone together; unchecking **Play shared audio** removed only the tab.
  **Deafen** silenced both and preserved the listener’s choice. Stopping
  the tab closed its playback stream without interrupting voice. Outbound
  Linux window/system audio is a separate milestone.
- **Three-person Linux voice rooms:** **Join voice** and **Leave voice** use
  the encrypted room's MatrixRTC slot without DM notifications. In the
  [three-account desktop test](experiments/rtc-audio/README.md#verify-three-person-voice-rooms)
  all six sender→listener paths carried the correct tones (410, 670, 1030 Hz);
  the SFU reported three GCM microphones, and room history had zero rings.
  After the third participant left, the other two exchanged fresh tones on
  their original tracks. On all three rendered rosters the **Speaking** label
  followed each mic in turn and cleared after silence. Two simultaneous
  speakers showed two labels; silencing one left the other highlighted.
  **Mixer** adjusts each person's playback level locally. Setting Bruno to
  25% on Alice's client reduced his decoded tone from 6,994 to 1,748; Clara's
  tone on Alice stayed at 6,987, and both other listeners' mixes stayed at
  baseline. Local silence, independent 150% boost and reset also passed.
  [Focused push-to-talk](experiments/rtc-audio/README.md#verify-push-to-talk)
  passed on Linux/X11: both peers decoded Alice only on a Right Ctrl hold;
  release or focus loss closed her capture stream. Bruno and Clara still
  heard each other in every 100 ms measurement window across transitions.
  A system-wide key for talking from another app remains open.
- **Linux portal window sharing:** [The three-client window test](experiments/rtc-audio/README.md#verify-portal-window-sharing)
  uses a private KDE Wayland picker and PipeWire. Cancelling creates no video
  publication. Selecting a window publishes one GCM screen track; both other
  desktops rendered its original and changed color pattern while the sharer
  browsed another channel. All six audio paths stayed live on the original
  microphone tracks. **Stop sharing** released the portal session and capture
  nodes, unpublished the video and cleared both remote views. Full-monitor
  capture and system-audio sharing are still unverified.
- **Linux voice-room camera:** [The three-client camera test](experiments/rtc-audio/README.md#verify-voice-room-camera-video)
  uses a real Camera portal with an isolated virtual camera. Both peers
  rendered changing green/magenta → orange/blue frames from a single GCM
  camera publication. Turning the camera off removed the reader, PipeWire
  capture links, SFU video publication and remote tiles; all six audio paths
  remained live on the original tracks. Physical webcams remain unverified.
- **Linux multi-camera layouts:** In the [grid/focus test](experiments/rtc-audio/README.md#verify-multi-camera-grid-and-focus),
  Alice rendered Bruno and Clara side by side, enlarged each in turn and
  switched back to the grid. Both feeds changed on-screen in each layout,
  including the unfocused thumbnail, with the same two encrypted video
  tracks and all six audio paths live. When Clara left with her camera
  focused, her capture was released; Alice returned to the grid and saw
  Bruno's next pattern on his original track.

- **Encrypted replies and jump-to-original:** [Linux Iced ↔ Element Web acceptance](scripts/replies-element.md)
  sent replies to two messages with identical text and sender. With 80
  intervening events, all 86 messages on Synapse were distinct Megolm events.
  After reload, Element highlighted the correct original outside its restored
  live timeline. After restarting the native client, both reply links fetched
  their distinct original event IDs through `/context` and rendered the
  matching neighboring messages.
- **Editing encrypted replies:** The [extended interop test](scripts/replies-element.md)
  edited one reply twice in Untethered and the other once in Element. All
  three edits were Megolm `m.replace` events targeting the two reply IDs.
  After restart/reload, both timelines displayed *Polar revision* and
  *Meridian correction* in place. Each client’s two links still opened the
  correct original despite identical original text.
- **Reactions on edited replies:** The [two-client reaction test](scripts/replies-element.md#reactions-on-an-edited-reply)
  added 👍 once from each client. Both showed 2 on the base reply ID.
  Untethered removed its own annotation after a restart; after reconnecting
  again, both showed 1 and only Element still owned the reaction. Synapse
  confirmed no reactions targeted the replacement, quoted message or sibling reply.

- **Room-scoped local message search:** The [Linux desktop search test](scripts/room-search.md)
  found an edited encrypted reply after restart with 150 newer messages.
  The old wording no longer matched, and a matching message cached in another
  encrypted room did not appear. Only opening the result fetched its context,
  using the base reply ID. **Return to timeline** restored the same message
  at the same pixel position after three arrivals and a five-line edit above it.

- **Linux room notifications:** The [two-account notification test](scripts/room-alerts.md)
  muted ordinary chat in an encrypted room through the Iced controls. A
  direct mention from Element appeared in Dunst; an ordinary message and a
  mention addressed to someone else reached the receiver but produced zero
  desktop notification calls. A second room still notified. The override
  survived restart without replaying notifications, and **Account default**
  removed it from the homeserver.
- **Browser tab notifications:** The [Chromium acceptance test](scripts/room-alerts.md#verify-browser-tab-notifications)
  enabled the site’s real notification permission and backgrounded the
  receiving tab. Element sent encrypted messages in the same muted room.
  The direct mention produced exactly one visible Chromium popup; an
  ordinary message and a mention for another user produced none. Activating
  the popup refocused Untethered. The grant and room mute survived reload
  without replay. Closed-tab push and other browser/platform combinations
  remain unverified.

- **Room posting enforcement:** The [two-account Synapse test](scripts/room-permissions.md)
  made an encrypted room read-only through Untethered. Synapse rejected the
  joined member’s encrypted send, plain-text send and attempt to change the
  posting rule with HTTP 403 `M_FORBIDDEN`; the administrator could still
  post. After Untethered restarted and restored posting, Synapse accepted
  the member’s message and both clients displayed it. A stricter sticker
  permission and intervening changes to two other rules survived restoration.
- **Ban and unban:** [Two-account tests](scripts/room-permissions.md#verify-ban-unban-and-reinvitation)
  used the native **People** controls and a real Element member. With public
  joins allowed, the federated member’s banned rejoin returned HTTP 403
  `M_FORBIDDEN` (“You are banned from this room”). On the same Synapse server
  the equivalent refusal was HTTP 403 `M_BAD_STATE`. After restart, **Unban**
  left the member outside; in invite-only mode **Invite** allowed rejoining,
  and both clients displayed the new encrypted message.
- **Kick in a public room:** The [two-account kick test](scripts/room-permissions.md#verify-kick-in-a-public-room)
  used **People → Kick**. Synapse recorded a moderator-authored `leave` with
  the supplied reason and rejected both encrypted and plain member sends
  with HTTP 403 `M_FORBIDDEN` while the member was out. The member rejoined
  with HTTP 200 and both clients displayed the next encrypted message.
  There were no invite or ban events for that member.
- **Message reporting:** The [Linux desktop and Chromium tests](scripts/room-permissions.md#verify-message-reports)
  each submitted reports as an ordinary member of an encrypted room. An
  independent Synapse administrator checked the exact room ID, event ID,
  reporter, sender and reason for each report. Canceling on a look-alike
  message produced none. Editing the target while the form was open did
  not redirect the report; reporting the updated text targeted its edit
  event. No message was removed or report posted in chat. In-client
  moderation history and the wider community workflows remain open.

- **Cross-room mentions inbox:** [Two-account tests on Linux and Chromium](scripts/mentions-inbox.md)
  opened two encrypted rooms with identical mention text. After restarting
  the native client or closing and reopening the browser tab, the inbox
  showed each direct mention once and one added by an edit. Ordinary,
  room-wide, other-recipient and edit-removed mentions were excluded.
  With 70 readable messages per room, all three selections fetched the exact
  room/event and displayed its neighboring message. Leaving a room removed
  only its two entries from the open inbox.

- **Linux upgrade and key continuity:** The [Arch package upgrade test](scripts/linux-upgrade.md#recorded-run-2026-11-25)
  upgraded `0.1.0-2` to `0.1.0-6` in place. Device ID `VPKQEQZBTB` and
  both public keys (Ed25519 and Curve25519) were identical before and after.
  Element sent an encrypted message while the old client was stopped; it
  was absent from the profile until the upgraded client decrypted it.
  The upgraded client also decrypted a fresh message after Element rotated
  its Megolm session. Neither sign-in nor recovery was repeated.

- **Account-wide Do Not Disturb:** [Linux + Chromium acceptance](scripts/room-alerts.md#verify-account-wide-do-not-disturb)
  enabled DND on Linux, restarted both receivers, then disabled it from
  Chromium. Neither showed a popup for decrypted mentions or ordinary
  chat during DND. Each showed the baseline and both restored notifications;
  the mentions inbox retained the silenced mentions. The full Matrix rule
  set matched its baseline after resuming. A separate Linux call test stopped
  an already audible ringtone and answered a new silent call, with 480/960 Hz
  received on the respective peers and both microphone tracks encrypted.

- **Cross-device read state and receipts:** The [Linux/Chromium/Element test](scripts/read-state.md)
  checked exact `m.fully_read` and private receipt event IDs in two encrypted
  rooms. A hidden tab, an unfocused window, room search, and a scrolled-up
  timeline left the marker unchanged. Marking the room unread on web survived
  restarting both clients. Marking it read preserved the other room’s
  highlight. Opting into public receipts made the next native read visible
  in Element; turning sharing off kept later reads private. Both receivers
  displayed Element’s public receipt beneath the exact message.

- **Inherited space alerts:** [Native Linux and Chromium tests](scripts/room-alerts.md#verify-space-alerts)
  verified mute and all-message defaults for nested spaces, explicit channel
  overrides, direct mentions, DND and an unaffected outside room. Moving a
  room out of its last parent and adding it to a muted space updated the
  actual Synapse rules. The browser run checked both a hidden tab and a second
  Linux device on the same account: exactly six visible popups on each.
  Clearing the space preference preserved the independent channel override
  and unrelated custom rules.

## What parity means

A small community should be able to move its everyday conversations here: find a server and its channels, talk in DMs, drop a screenshot, join friends in voice, share a screen, catch up on mentions, and moderate its space. Test those workflows side by side with Discord. A button that looks right but doesn't work with another Matrix client isn't parity.

Keep the frontend in **Rust/Iced** and **gomuks** as the Matrix engine. Share room, attachment and call state between desktop and web; isolate file access, capture, playback and notifications behind platform adapters. Keep native controls for desktop calls. Launching a hosted Element Call page is useful for testing but isn't the finished native calling experience.

**Baseline before this milestone (`bfa75b3`):** server rail and space/DM navigation, grouped plain-text timeline, in-memory room drafts, password login, recovery-key verification, and encrypted messages. Linux and Chromium had exchanged encrypted messages and restored sessions. Windows cross-built but had no runtime acceptance test. Media cards, transfers, calls, and voice channels were not yet implemented.

## A. Drop a file and send it

**Experience:** Drag one or more files from Dolphin or Explorer. Highlight the target conversation by name. Show removable attachment chips, image previews, file sizes and a caption field above the composer. Send only when the user presses Send. The `+` picker and pasted screenshots use the same tray. Drafts and in-flight files stay with their original room when the user switches channels.

1. **Prove native Wayland drop first.** Pinned Iced 0.14 explicitly marks `FileDropped` and `FileHovered` unimplemented on Wayland. Try an upstream fix or a small Rust integration with the existing window and Wayland data device. Verify multiple items, spaces in filenames, `text/uri-list`, and portal-backed transfers. An XWayland window or a file picker does not pass the Dolphin → native Wayland test. Use Iced window events on Windows; use DOM `DataTransfer.files` and `File` objects on web.
2. **One transfer model:** staged → uploading → sending event → confirmed, with cancel/failure/retry per file. Bind each transfer to the room ID when staged, respect the homeserver limit, and show progress without blocking navigation. If upload succeeds but event sending fails, retry with the existing media descriptor. Send one standard Matrix media event per file initially so Element can read it.
3. **Use the transport gomuks actually supports.** Desktop: stream file bytes to authenticated loopback `POST /_gomuks/upload`, set encryption for encrypted rooms, and pass the returned descriptor as `base_content` to `send_message`. The file-path `upload_media` RPC is FFI-only. Browser: add upstream's `wasm-upload` binary transfer to our JSON-only worker bridge. Bound file size and concurrency because the current worker buffers a whole file. Use indeterminate progress when byte progress isn't available.
4. **Receive as well as send.** Keep media descriptors in the timeline model. Render image thumbnails and download cards with filename, type and size. Handle encrypted thumbnails and downloads through gomuks. Wire the upstream media service worker or an equivalent browser download path; our current browser bootstrap doesn't register it.

**Done when:** Drop an image and a PDF into an encrypted room on KDE Wayland, switch channels mid-upload, and receive both in the intended Element room. Open the image, save a byte-identical PDF, then repeat in reverse. Exercise Windows and Chromium, an unencrypted room, an oversized file, cancel, and retry without duplicate sends.

## B. Ring a friend and talk

**Experience:** Start an audio call from a DM; the other party can accept or decline. Show ringing, connecting, and connected states. Keep a compact call bar visible while browsing channels, with mute, deafen, device selection and disconnect. The first release covers one-to-one voice while both apps are open.

1. **Prove two-way encrypted audio before polishing the call UI.** Use two accounts and a test homeserver with MatrixRTC discovery, an SFU and relay connectivity. Evaluate a maintained Rust-facing WebRTC/LiveKit media stack for desktop. Use gomuks for transport discovery, authorization and Matrix signaling where supported. Test native capture/output, negotiated audio, E2EE and key rotation against Element. A token or a successful room join is not evidence that the other person can hear you.
2. **Establish the engine boundary.** gomuks exposes RTC transport discovery, LiveKit token requests, delayed-event operations and encrypted to-device messaging. Its own web frontend uses an Element Call widget. Inventory membership, ringing, call keys, track encryption and cleanup required by the native adapter. Prefer reuse over maintaining a parallel Matrix client. For web, evaluate upstream Element Call behind the same call-state model. Surface missing server infrastructure and unsupported legacy calling explicitly.
3. **Finish the lifecycle:** incoming/outgoing, accept, decline, cancel, no answer, hang up, reconnect and answered-on-another-device. Handle permission denial and device removal. Release the microphone on hangup and expire stale call membership after a crash. No silent downgrade from encrypted media.

**Done when:** Untethered and Element can initiate calls to one another on separate networks, including a relay-required connection. Two Untethered users can do the same. Both hear audio, can change channels without dropping the call, mute/deafen correctly, recover from a disconnect and hang up cleanly. Check a server without RTC support. Record real Linux, Windows and browser results separately; keep the feature experimental if encrypted interop fails.

## Remaining delivery order

| Milestone | Deliverable | Acceptance example |
|---|---|---|
| **3. Voice channels, video and screen sharing** | Join a named voice room without ringing everyone. Show participants under the channel, speaking indicators, per-user volume, push-to-talk. Then add camera, grid/focus modes, and screen/window sharing. Use the desktop portal + PipeWire on Wayland, Windows capture, browser share picker. Track system-audio capture and global hotkeys by platform. | Three people join voice, one shares a window, and another browses text channels without losing audio. Leaving stops capture. |
| **4. Everyday messaging** | Replies and jump-to-original, edits, redactions, reactions, mentions/autocomplete, typing, read markers/unread divider, Markdown/code, real avatars, spoilers, link previews, and drafts that survive restart. | Two clients see the same edited reply and reactions after reconnect, even if the original was outside loaded history. |
| **5. Servers, navigation and discovery** | Ordered/collapsible channel categories, remembered room and scroll position, DM/group chat creation, real member lists, unread/mention badges, Ctrl+K, pins, threads, room links, message search and mentions inbox. Expose how much encrypted history is indexed locally. | Find a week-old reply, jump to context, then return to the prior scroll position. |
| **6. Notifications and presence** | Desktop/browser notifications, per-server and per-channel overrides, DND, presence/custom status, sounds, unread sync. Separate foreground ringing from closed-app ringing; tray/background operation is opt-in. | Answer on one device: the other stops ringing. Muted rooms stay quiet; a direct mention follows the selected override. |
| **7. Community management** | Create/join/leave spaces and rooms, invites, channel ordering, profiles/nicknames, permissions, bans/kicks, reports and moderation history where available. Map Discord-like controls honestly onto Matrix power levels and room permissions. | A moderator sets up a community and a second account actually encounters the configured access limits. |
| **8. Account and release readiness** | SSO/OIDC, logout, session/device management, backup health, recovery, and emoji/SAS verification when supported by the engine. Accessible names, screen-reader and keyboard operation, selectable text, scaling, signed Windows distribution, browser compatibility and update/migration tests. | A new user installs, signs in, recovers history, shares a file, calls someone and upgrades without the terminal or losing device keys. |
| **Later, by demand** | Custom emoji/stickers, voice notes, polls, forum channels, stages, scheduled events, bots/commands and richer moderation integrations. | Prioritize by what real communities miss. Discord connectivity, billing, Nitro and its marketplace are separate products. |

**Across every milestone:** Compare keyboard focus, right-click menus, hover states, scroll anchoring, spacing and empty/error states with Discord. Start accessibility and real Windows testing with the first feature. Keep the emoji-heavy channel-switch regression fixture; measure responsiveness and memory during transfers and calls. Don't rebuild the full timeline for every upload tick. No file contents, tokens or call keys in diagnostics.

## First implementation tickets

1. `FILES-01` — Dolphin → Wayland and Explorer → Windows drop, staged without sending.
2. `CALL-01` — Encrypted native audio with Element; record server requirements, media-stack choice and signaling gaps.
3. `FILES-02` — Encrypted desktop send/receive/save with a byte-for-byte integrity check.
4. `FILES-03` — Multi-file drafts, paste, progress, cancel/retry, browser binary upload and authenticated download.
5. `CALL-02` — DM ringing, persistent call controls and device switching on the working media path.

**Implementation references:** branch `rust-discord-ui`, `rust-client/src/{main,rooms,view}.rs` and `rust-client/src/backend/{native,web}.rs`; [Iced window events](https://docs.rs/iced_core/0.14.0/iced_core/window/enum.Event.html); pinned gomuks [RPC contracts](https://github.com/gomuks/gomuks/blob/v0.2611.1/pkg/hicli/jsoncmd/commands.go), [media handler](https://github.com/gomuks/gomuks/blob/v0.2611.1/pkg/gomuks/mediaupload.go), [WASM upload](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/api/wasmclient.ts), [Element Call integration](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/ui/widget/ElementCall.tsx), and [widget driver](https://github.com/gomuks/gomuks/blob/v0.2611.1/web/src/ui/widget/widgetDriver.ts).
