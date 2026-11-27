# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| A2 | Voice processing and input sensitivity | a2-voice | `claude/a2-voice-a94df8` | working | 2026-11-27T22:45:17Z | `README.md`, `ROADMAP.md`, `experiments/rtc-audio/README.md`, `experiments/rtc-audio/run_element.sh`, `rust-client/docs/calls-parity-plan.md`, `rust-client/src/call_settings.rs`, `rust-client/src/desktop_call.rs`, `rust-client/src/desktop_call/audio.rs`, `rust-client/src/desktop_call/channel.rs`, `rust-client/src/desktop_call/voice.rs`, `rust-client/src/main.rs`, `rust-client/src/view.rs`, `rust-client/src/view/voice_settings.rs`, `scripts/qa_events.py`, `scripts/qa_rtc_audio.py`, `scripts/qa_voice_processing.py`, `scripts/qa_voice_room.py`, `scripts/test-media-interop.py` |
| J5 | Sign-in everywhere | codex-j5 | `main` | working | 2026-11-27T22:26:13Z | `rust-client/src/engine/browser.rs`, `rust-client/src/engine/session.rs`, `rust-client/src/engine/worker.rs`, `rust-client/src/view/sdk_account.rs`, `rust-client/web/auth.js`, `rust-client/web/engine.js`, `rust-client/web/index.html`, `scripts/qa_matrix_sdk.py`, `scripts/qa_web_login.py` |
| M11 | Custom emoji and stickers (image packs) | main-session | `m11-emoji` | review | 2026-11-27T21:57:55Z | `README.md`, `ROADMAP.md`, `rust-client/src/autocomplete.rs`, `rust-client/src/emotes.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/compose.rs`, `rust-client/src/engine/media.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/packs.rs`, `rust-client/src/engine/timeline.rs`, `rust-client/src/main.rs`, `rust-client/src/media.rs`, `rust-client/src/pills.rs`, `rust-client/src/preview.rs`, `rust-client/src/reactions.rs`, `rust-client/src/rich.rs`, `rust-client/src/view.rs`, `rust-client/src/view/autocomplete.rs`, `rust-client/src/view/emoji_picker.rs`, `rust-client/src/view/emotes.rs`, `rust-client/src/view/icons.rs`, `rust-client/src/view/reaction_bar.rs`, `rust-client/src/view/rich.rs`, `rust-client/src/view/selectable.rs`, `scripts/qa_emotes.py` |
| M12 | Per-server nicknames and role badges | m12-names | `claude/m12-names-fc018c` | review | 2026-11-27T21:56:30Z | `README.md`, `ROADMAP.md`, `rust-client/src/context_menu.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/names.rs`, `rust-client/src/engine/rooms.rs`, `rust-client/src/main.rs`, `rust-client/src/names.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/view.rs`, `rust-client/src/view/context_menu.rs`, `rust-client/src/view/gallery.rs`, `rust-client/src/view/names.rs`, `rust-client/src/view/profile.rs`, `rust-client/src/view/room_people.rs`, `scripts/qa_names.py` |
| N5 | Drag to reorder channels and categories | n5-order | `claude/n5-order-211d5e` | review | 2026-11-27T21:59:39Z | `README.md`, `ROADMAP.md`, `rust-client/src/channel_order.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/arrange.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/rooms.rs`, `rust-client/src/explore.rs`, `rust-client/src/main.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/view.rs`, `rust-client/src/view/channel_order.rs`, `scripts/qa_channel_order.py` |

## Done

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
- 2026-11-27 **M7** Link previews (main-session): merged as a3de2bf
- 2026-11-27 **roadmap-nitro** Roadmap: Nitro features, free where Matrix allows (main-session): merged as e732170
- 2026-11-27 **roadmap-gomuks** Roadmap: drop gomuks-era sections and mentions (main-session): merged as 508bd00
- 2026-11-27 **roadmap-matrix** Roadmap: Matrix support per step and a policy for client-only features (main-session): merged as e46cc82
- 2026-11-27 **roadmap-parity** Roadmap: status refresh and new Discord-parity steps (main-session): merged as b9834ed
- 2026-11-27 **memory-keys** Memory: fewer threads, capped arenas, close left rooms' timelines; older rooms' keys in bulk; clear UTD labels (main-session): merged as fd54b81
- 2026-11-27 **M5** Member profile cards and own profile editing (codex-m5): merged as f398e1f (PR #5)
