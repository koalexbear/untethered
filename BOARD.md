# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| J4 | Explore public rooms and spaces | j4-explore | `claude/j4-explore-c19f11` | review | 2026-11-27T20:34:16Z | `README.md`, `ROADMAP.md`, `rust-client/src/avatars.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/explore.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/explore.rs`, `rust-client/src/invites.rs`, `rust-client/src/joining.rs`, `rust-client/src/main.rs`, `rust-client/src/mentions.rs`, `rust-client/src/view.rs`, `rust-client/src/view/explore.rs`, `rust-client/src/view/icons.rs`, `rust-client/src/view/joining.rs`, `scripts/qa_explore.py` |
| N3 | Pinned messages and Matrix threads | codex-n3 | `codex/n3-pins-threads` | working | 2026-11-27T15:03:37Z | `rust-client/src/engine/account_ui.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/live_test.rs`, `rust-client/src/engine/messaging.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/pins.rs`, `rust-client/src/engine/threads.rs`, `scripts/qa_matrix_sdk.py` |
| N4 | Presence and custom status | n4-presence | `claude/n4-presence-13e26f` | review | 2026-11-27T20:59:02Z | `README.md`, `ROADMAP.md`, `rust-client/src/account.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/presence.rs`, `rust-client/src/main.rs`, `rust-client/src/presence.rs`, `rust-client/src/tray.rs`, `rust-client/src/view.rs`, `rust-client/src/view/presence.rs`, `rust-client/src/view/profile.rs`, `rust-client/src/view/room_people.rs`, `scripts/qa_presence.py` |
| N9 | Sounds and shortcuts | n9-sounds | `claude/n9-sounds-8156ba` | review | 2026-11-27T20:55:02Z | `README.md`, `ROADMAP.md`, `rust-client/src/engine/alerts.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/qa.rs`, `rust-client/src/main.rs`, `rust-client/src/notifications.rs`, `rust-client/src/notifications/desktop.rs`, `rust-client/src/shortcuts.rs`, `rust-client/src/sounds.rs`, `rust-client/src/view.rs`, `rust-client/src/view/shortcuts.rs`, `rust-client/src/view/sounds.rs`, `scripts/qa_sounds.py` |
| V2 | Simulcast, per-tile quality, pause hidden video | v2-simulcast | `claude/v2-simulcast-9158cf` | review | 2026-11-27T20:50:38Z | `README.md`, `ROADMAP.md`, `experiments/rtc-audio/README.md`, `rust-client/docs/calls-parity-plan.md`, `rust-client/src/desktop_call.rs`, `rust-client/src/desktop_call/*.rs`, `rust-client/src/main.rs`, `rust-client/src/tray.rs`, `rust-client/src/view/call_controls.rs`, `rust-client/src/view/call_stage.rs`, `scripts/qa_room_camera.py`, `scripts/qa_room_layout.py`, `scripts/qa_voice_room.py`, `scripts/qa_window_share.py` |

## Done

- 2026-11-27 **M9** Polls (MSC3381): show, vote, end, create (main-session): merged as 76cfdc8
- 2026-11-27 **fix-call-video** Fix call video: share corruption at odd widths, low-fps own preview (fix-call-video): merged as 86aa2c6
- 2026-11-27 **inline-video-overlay** Inline video: controls over the video, auto-hiding (main-session): merged as d2c2530
- 2026-11-27 **M8** Forward messages (main-session): merged as 52398c0
- 2026-11-27 **P4** Video in the browser (Windows playback stays open) (main-session): released: browser half merged as 0c7f452; Windows video playback is still open
- 2026-11-27 **P5** Sending video (main-session): merged as 0c7f452
- 2026-11-27 **P2** Inline players (main-session): merged as 0c7f452
- 2026-11-27 **P1** Animated images (with P2, P4 browser, P5 on this branch) (main-session): merged as 0c7f452
- 2026-11-27 **P3** Viewer polish: preloading, thumbnail strip, gallery, sharper zoom (main-session): merged as 1b74b10
- 2026-11-27 **copy-image-zoom** Copy Image, and zoom in the viewer (part of P3) (main-session): merged as 372e13d
- 2026-11-27 **N7** Search across a space (main-session): merged as eeda711
- 2026-11-27 **fix-unread-counts** Unread badges trust the client's counts (main-session): merged as 93e2e7d
- 2026-11-27 **N2** Remember where you were (main-session): merged as 3777467
- 2026-11-27 **N1** Quick switcher (main-session): merged as 5d1ff40
- 2026-11-27 **J3** DMs and blocking (main-session): merged as 5679235
- 2026-11-27 **J2** Join by link (main-session): merged as 1582107
- 2026-11-27 **M6** Message deletion and persistent drafts (codex-m6): merged as 60f368e (PR #6)
- 2026-11-27 **J1** Invites with Accept and Decline (main-session): merged as 09754bb
- 2026-11-27 **M7** Link previews (main-session): merged as a3de2bf
- 2026-11-27 **roadmap-nitro** Roadmap: Nitro features, free where Matrix allows (main-session): merged as e732170
- 2026-11-27 **roadmap-gomuks** Roadmap: drop gomuks-era sections and mentions (main-session): merged as 508bd00
- 2026-11-27 **roadmap-matrix** Roadmap: Matrix support per step and a policy for client-only features (main-session): merged as e46cc82
- 2026-11-27 **roadmap-parity** Roadmap: status refresh and new Discord-parity steps (main-session): merged as b9834ed
- 2026-11-27 **memory-keys** Memory: fewer threads, capped arenas, close left rooms' timelines; older rooms' keys in bulk; clear UTD labels (main-session): merged as fd54b81
- 2026-11-27 **M5** Member profile cards and own profile editing (codex-m5): merged as f398e1f (PR #5)
- 2026-11-27 **ux-polish** Jump bar button, steady history loading, floating reaction picker, voice channel click joins (main-session): merged as 2ce0c06
- 2026-11-27 **fix-media-previews** Image previews always load; the viewer keeps the scroll position (main-session): merged as eeaaa4d
- 2026-11-27 **room-upgrades** Upgraded rooms as one chat (room-upgrades): merged as fedb2d4
- 2026-11-27 **M3** Typing indicators (m3-typing): merged as a9422b5
- 2026-11-26 **single-instance** Single instance: a second launch shows the running window (main-session): merged as 2d29437
