# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| M5 | Member profile cards and own profile editing | codex-m5 | `codex/m5-profiles` | working | 2026-11-27T08:38:23Z | `rust-client/src/account.rs`, `rust-client/src/context_menu.rs`, `rust-client/src/engine/account_ui.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/profile.rs`, `rust-client/src/main.rs`, `rust-client/src/permissions.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/view.rs`, `rust-client/src/view/profile.rs`, `rust-client/src/view/room_people.rs`, `rust-client/src/web_files.rs`, `rust-client/web/files.js` |
| fix-media-previews | Image previews always load; the viewer keeps the scroll position | main-session | `fix-media-previews` | working | 2026-11-27T08:36:29Z | `README.md`, `rust-client/src/engine/media.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/inline.rs`, `rust-client/src/main.rs`, `rust-client/src/media.rs`, `rust-client/src/view.rs`, `rust-client/src/view/rich.rs`, `rust-client/src/view/selectable.rs`, `rust-client/src/view/upgrades.rs`, `rust-client/src/view/viewer.rs`, `scripts/qa_media_previews.py` |

## Done

- 2026-11-27 **room-upgrades** Upgraded rooms as one chat (room-upgrades): merged as fedb2d4
- 2026-11-27 **M3** Typing indicators (m3-typing): merged as a9422b5
- 2026-11-26 **single-instance** Single instance: a second launch shows the running window (main-session): merged as 2d29437
- 2026-11-26 **N6** System tray (n6-system-tray): merged as 4145624
- 2026-11-26 **M1** Rich text in received messages (m1-rich-text): merged as 62d12ae
- 2026-11-26 **M4** Right-click menus (m4-context-menus): merged as a1e37d5 (right-click Mention now uses M2's App::mention)
- 2026-11-26 **M2** Composer autocomplete (m2-autocomplete): merged as 1c707e1
- 2026-11-26 **roadmap-tray** Add the system tray to the roadmap (coordinator): merged as 86b1c98
