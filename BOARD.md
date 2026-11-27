# Work board

Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.

| ID | Title | Agent | Branch | Status | Updated | Paths |
|---|---|---|---|---|---|---|
| M3 | Typing indicators | m3-typing | `claude/m3-typing` | review | 2026-11-27T00:27:19Z | `README.md`, `ROADMAP.md`, `rust-client/src/account.rs`, `rust-client/src/backend/mod.rs`, `rust-client/src/engine/*`, `rust-client/src/main.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/typing.rs`, `rust-client/src/view.rs`, `rust-client/src/view/typing.rs`, `scripts/qa_replies.py`, `scripts/qa_typing*`, `scripts/qa_typing.py`, `scripts/replies-element.md`, `scripts/test-media-interop.py` |
| room-upgrades | Upgraded rooms as one chat | room-upgrades | `claude/room-upgrades` | review | 2026-11-27T00:23:01Z | `README.md`, `rust-client/src/context_menu.rs`, `rust-client/src/engine/app.rs`, `rust-client/src/engine/live_test.rs`, `rust-client/src/engine/messaging.rs`, `rust-client/src/engine/mod.rs`, `rust-client/src/engine/rooms.rs`, `rust-client/src/engine/search.rs`, `rust-client/src/engine/timeline.rs`, `rust-client/src/engine/upgrades.rs`, `rust-client/src/main.rs`, `rust-client/src/mentions.rs`, `rust-client/src/reading.rs`, `rust-client/src/replies.rs`, `rust-client/src/rooms.rs`, `rust-client/src/rooms/engine.rs`, `rust-client/src/search.rs`, `rust-client/src/upgrades.rs`, `rust-client/src/view.rs`, `rust-client/src/view/reaction_bar.rs`, `rust-client/src/view/upgrades.rs`, `scripts/qa_room_upgrades.py` |

## Done

- 2026-11-26 **single-instance** Single instance: a second launch shows the running window (main-session): merged as 2d29437
- 2026-11-26 **N6** System tray (n6-system-tray): merged as 4145624
- 2026-11-26 **M1** Rich text in received messages (m1-rich-text): merged as 62d12ae
- 2026-11-26 **M4** Right-click menus (m4-context-menus): merged as a1e37d5 (right-click Mention now uses M2's App::mention)
- 2026-11-26 **M2** Composer autocomplete (m2-autocomplete): merged as 1c707e1
- 2026-11-26 **roadmap-tray** Add the system tray to the roadmap (coordinator): merged as 86b1c98
