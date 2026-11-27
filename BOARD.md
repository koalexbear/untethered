# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| J1 | Invites with Accept and Decline | main-session | `j1-invites` | working | 2026-11-27T12:12:07Z | `README.md`, `rust-client/src/avatars.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/invites.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/profile.rs`, `rust-client/src/engine/rooms.rs`, `rust-client/src/invites.rs`, `rust-client/src/main.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/typing.rs`, `rust-client/src/upgrades.rs`, `rust-client/src/view.rs`, `rust-client/src/view/invites.rs`, `scripts/qa_invites.py` |
| M6 | Message deletion and persistent drafts | codex-m6 | `codex/m6-delete-drafts` | review | 2026-11-27T10:42:45Z | `rust-client/src/account.rs`, `rust-client/src/context_menu.rs`, `rust-client/src/drafts.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/live_test.rs`, `rust-client/src/engine/messaging.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/main.rs`, `rust-client/src/permissions.rs`, `rust-client/src/reading.rs`, `rust-client/src/transfers.rs`, `rust-client/src/view.rs`, `rust-client/src/view/context_menu.rs`, `scripts/qa_matrix_sdk.py` |

## Done

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
