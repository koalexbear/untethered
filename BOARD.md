# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| N3 | Pinned messages and Matrix threads | codex-n3 | `codex/n3-pins-threads` | working | 2026-11-27T15:03:37Z | `rust-client/src/engine/account_ui.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/live_test.rs`, `rust-client/src/engine/messaging.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/pins.rs`, `rust-client/src/engine/threads.rs`, `scripts/qa_matrix_sdk.py` |
| P1 | Animated images (with P2, P4 browser, P5 on this branch) | main-session | `phase4-media` | working | 2026-11-27T15:58:45Z | `rust-client/src/player.rs rust-client/src/media.rs rust-client/src/preview.rs rust-client/src/inline.rs rust-client/src/avatars.rs rust-client/src/view.rs rust-client/src/view/viewer.rs rust-client/src/viewer.rs rust-client/src/engine/media.rs rust-client/src/main.rs rust-client/src/web_files.rs ROADMAP.md README.md scripts/qa_phase4.py` |
| P2 | Inline players | main-session | `phase4-media` | working | 2026-11-27T15:58:47Z | `rust-client/src/player.rs rust-client/src/media.rs rust-client/src/preview.rs rust-client/src/inline.rs rust-client/src/avatars.rs rust-client/src/view.rs rust-client/src/view/viewer.rs rust-client/src/viewer.rs rust-client/src/engine/media.rs rust-client/src/main.rs rust-client/src/web_files.rs ROADMAP.md README.md scripts/qa_phase4.py` |
| P4 | Video in the browser (Windows playback stays open) | main-session | `phase4-media` | working | 2026-11-27T15:58:51Z | `rust-client/src/player.rs rust-client/src/media.rs rust-client/src/preview.rs rust-client/src/inline.rs rust-client/src/avatars.rs rust-client/src/view.rs rust-client/src/view/viewer.rs rust-client/src/viewer.rs rust-client/src/engine/media.rs rust-client/src/main.rs rust-client/src/web_files.rs ROADMAP.md README.md scripts/qa_phase4.py` |
| P5 | Sending video | main-session | `phase4-media` | working | 2026-11-27T15:58:49Z | `rust-client/src/player.rs rust-client/src/media.rs rust-client/src/preview.rs rust-client/src/inline.rs rust-client/src/avatars.rs rust-client/src/view.rs rust-client/src/view/viewer.rs rust-client/src/viewer.rs rust-client/src/engine/media.rs rust-client/src/main.rs rust-client/src/web_files.rs ROADMAP.md README.md scripts/qa_phase4.py` |

## Done

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
- 2026-11-26 **N6** System tray (n6-system-tray): merged as 4145624
- 2026-11-26 **M1** Rich text in received messages (m1-rich-text): merged as 62d12ae
- 2026-11-26 **M4** Right-click menus (m4-context-menus): merged as a1e37d5 (right-click Mention now uses M2's App::mention)
- 2026-11-26 **M2** Composer autocomplete (m2-autocomplete): merged as 1c707e1
- 2026-11-26 **roadmap-tray** Add the system tray to the roadmap (coordinator): merged as 86b1c98
