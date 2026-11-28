# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| J5 | Sign-in everywhere | codex-j5 | `main` | working | 2026-11-27T22:26:13Z | `rust-client/src/engine/browser.rs`, `rust-client/src/engine/session.rs`, `rust-client/src/engine/worker.rs`, `rust-client/src/view/sdk_account.rs`, `rust-client/web/auth.js`, `rust-client/web/engine.js`, `rust-client/web/index.html`, `scripts/qa_matrix_sdk.py`, `scripts/qa_web_login.py` |
| M14 | Selection across messages | m14-selection | `claude/m14-selection-b651f0` | review | 2026-11-28T14:45:19Z | `README.md`, `ROADMAP.md`, `rust-client/src/main.rs`, `rust-client/src/rich.rs`, `rust-client/src/selection.rs`, `rust-client/src/view.rs`, `rust-client/src/view/rich.rs`, `rust-client/src/view/selectable.rs`, `rust-client/src/view/selection.rs`, `scripts/qa_selection.py` |
| S6 | Performance budgets | s6-perf-budgets | `claude/s6-perf-budgets-7291bd` | working | 2026-11-28T14:21:24Z | `README.md`, `ROADMAP.md`, `rust-client/src/engine/app.rs`, `rust-client/src/main.rs`, `rust-client/src/perf.rs`, `scripts/perf_budgets.json`, `scripts/qa_performance.py` |
| U5 | Incoming call card and desktop notification | u5-incoming-call | `claude/u5-incoming-call-bece3a` | review | 2026-11-28T15:08:50Z | `README.md`, `ROADMAP.md`, `experiments/rtc-audio/README.md`, `experiments/rtc-audio/evidence/u5-incoming-video-card.png`, `rust-client/docs/calls-parity-plan.md`, `rust-client/src/incoming.rs`, `rust-client/src/main.rs`, `rust-client/src/notifications.rs`, `rust-client/src/notifications/desktop.rs`, `rust-client/src/tray.rs`, `rust-client/src/view.rs`, `rust-client/src/view/incoming.rs`, `rust-client/src/view/user_settings.rs`, `scripts/qa_desktop_dm.py`, `scripts/qa_dm_video.py`, `scripts/qa_element_video.py`, `scripts/qa_layout.py`, `scripts/qa_multi_device_ring.py`, `scripts/qa_native_dm.py`, `scripts/qa_ring_notice.py`, `scripts/qa_room_alerts.py`, `scripts/qa_rtc_audio.py`, `scripts/test-media-interop.py` |
| n7-index | Index encrypted rooms in the background for search | n7-index | `claude/n7-index-4c3db0` | review | 2026-11-28T15:14:18Z | `README.md`, `ROADMAP.md`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/search.rs`, `rust-client/src/engine/search_index.rs`, `rust-client/src/engine/timeline.rs`, `rust-client/src/main.rs`, `rust-client/src/search.rs`, `rust-client/src/search/*`, `rust-client/src/view/room_search.rs`, `scripts/qa_search_index.py`, `scripts/qa_space_search.py` |

## Done

- 2026-11-28 **U2** Voice & Video settings page (u2-voice-settings): merged as 1c45a60
- 2026-11-28 **M15** Profile customization (m15-profiles): merged as 8e2c625 (main 1c45a60)
- 2026-11-28 **M10** Voice messages (m10-voice-messages): merged as 07ab79e (main 1c45a60); recorder's voice_subscription renamed recorder_subscription in the U2 merge
- 2026-11-28 **C2** Roles and channel permissions (c2-roles): merged as 8efc57c (main 1c45a60)
- 2026-11-28 **fix-u1-fixtures** Retarget switch-camera and app-sound fixtures to U1's controls (u1-voice-panel): merged as 4532e28
- 2026-11-28 **N8** Server rail order and folders (n8-server-rail): merged as de46d8d
- 2026-11-28 **M13** Composer details (m13-composer): merged as cafc355 (main de46d8d)
- 2026-11-28 **C1** Server settings (c1-server-settings): merged as d0f34f4 (main de46d8d)
- 2026-11-28 **U1** Voice Connected panel and user-panel Mute/Deafen (u1-voice-panel): merged as 0d278da (main de46d8d)
- 2026-11-28 **fix-left-rooms** Rooms left elsewhere while closed stay listed (fix-room-ptt): merged as b6f46ea
- 2026-11-28 **fix-room-ptt** Fix --room-ptt: open Atrium without joining (fix-room-ptt): merged as b6f46ea
- 2026-11-28 **A2** Voice processing and input sensitivity (a2-voice): merged as f785f79 (main b6f46ea)
- 2026-11-27 **M12** Per-server nicknames and role badges (m12-names): merged as cfffeab
- 2026-11-27 **N5** Drag to reorder channels and categories (n5-order): merged as b5422a0
- 2026-11-27 **M11** Custom emoji and stickers (image packs) (main-session): merged as b6fa2b1
- 2026-11-27 **N3** Pinned messages and Matrix threads (codex-n3): merged as 3d219fe7b1b10c69e84aa7c77720d7fcbac1bec5
- 2026-11-27 **V2** Simulcast, per-tile quality, pause hidden video (v2-simulcast): merged as 7b99e29
- 2026-11-27 **N4** Presence and custom status (n4-presence): merged as 2e2da02
- 2026-11-27 **N9** Sounds and shortcuts (n9-sounds): merged as 1a44b88
- 2026-11-27 **J4** Explore public rooms and spaces (j4-explore): merged as 45a60c3
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
