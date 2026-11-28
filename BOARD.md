# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| C1 | Server settings | c1-server-settings | `claude/c1-server-settings-709791` | review | 2026-11-28T09:54:23Z | `README.md`, `ROADMAP.md`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/space_settings.rs`, `rust-client/src/main.rs`, `rust-client/src/permissions.rs`, `rust-client/src/server_settings.rs`, `rust-client/src/view.rs`, `rust-client/src/view/server_settings.rs`, `scripts/qa_server_settings.py` |
| J5 | Sign-in everywhere | codex-j5 | `main` | working | 2026-11-27T22:26:13Z | `rust-client/src/engine/browser.rs`, `rust-client/src/engine/session.rs`, `rust-client/src/engine/worker.rs`, `rust-client/src/view/sdk_account.rs`, `rust-client/web/auth.js`, `rust-client/web/engine.js`, `rust-client/web/index.html`, `scripts/qa_matrix_sdk.py`, `scripts/qa_web_login.py` |
| M13 | Composer details | m13-composer | `claude/m13-composer-15b65f` | review | 2026-11-28T09:58:21Z | `README.md`, `ROADMAP.md`, `rust-client/Cargo.lock`, `rust-client/Cargo.toml`, `rust-client/src/autocomplete.rs`, `rust-client/src/commands.rs`, `rust-client/src/composer.rs`, `rust-client/src/dms.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/compose.rs`, `rust-client/src/engine/messaging.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/timeline.rs`, `rust-client/src/highlight.rs`, `rust-client/src/jumbo.rs`, `rust-client/src/main.rs`, `rust-client/src/places.rs`, `rust-client/src/rich.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/view.rs`, `rust-client/src/view/autocomplete.rs`, `rust-client/src/view/commands.rs`, `rust-client/src/view/rich.rs`, `rust-client/src/viewer.rs`, `scripts/qa_composer_details.py` |
| N8 | Server rail order and folders | n8-server-rail | `claude/n8-server-rail-3c3a7d` | review | 2026-11-28T09:54:01Z | `README.md`, `ROADMAP.md`, `rust-client/src/channel_order.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/rail.rs`, `rust-client/src/engine/rooms.rs`, `rust-client/src/main.rs`, `rust-client/src/rail_order.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/shortcuts.rs`, `rust-client/src/view.rs`, `rust-client/src/view/channel_order.rs`, `rust-client/src/view/drag.rs`, `rust-client/src/view/icons.rs`, `rust-client/src/view/server_rail.rs`, `scripts/qa_server_rail.py` |

## Done

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
- 2026-11-27 **N7** Search across a space (main-session): merged as eeda711
- 2026-11-27 **fix-unread-counts** Unread badges trust the client's counts (main-session): merged as 93e2e7d
- 2026-11-27 **N2** Remember where you were (main-session): merged as 3777467
- 2026-11-27 **N1** Quick switcher (main-session): merged as 5d1ff40
- 2026-11-27 **J3** DMs and blocking (main-session): merged as 5679235
- 2026-11-27 **J2** Join by link (main-session): merged as 1582107
- 2026-11-27 **M6** Message deletion and persistent drafts (codex-m6): merged as 60f368e (PR #6)
- 2026-11-27 **J1** Invites with Accept and Decline (main-session): merged as 09754bb
