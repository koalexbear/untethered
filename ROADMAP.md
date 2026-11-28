# Untethered: roadmap to Discord parity

**2026-11-27 · `main`**

**Next: sign-in everywhere (J5).**
Messages feel like Discord, and people can get in: invites show with
Accept and Decline, Matrix links open in the app, Explore finds the
homeserver's public spaces, and New message starts DMs and group DMs.
Ctrl+K switches rooms, every room and server opens where you left it, and
search covers a whole space with Discord's filters. Rooms have pinned messages
and Matrix threads. Sign-in everywhere (J5) finishes Phase 2, and the rest of Phase 1 (M8 onwards) fills in around
them. Calls continue in their own plan
([calls-parity-plan.md](rust-client/docs/calls-parity-plan.md)).

## Where it stands

- **Engine:** matrix-rust-sdk on desktop and in the browser (a Web Worker).
- **Layout:** Discord-style server rail with server icons, channel list with
  collapsible categories, header actions, hover toolbars, user panel and
  settings, and a Ctrl+K quick switcher over rooms, DMs and spaces. On the
  desktop: a system tray with close to tray, an unread badge and start at
  sign-in; launching again shows the running window.
- **Messaging:** formatted messages (code blocks, quotes, lists, spoilers),
  `@`, `#` and `:` autocomplete with real mentions, typing indicators,
  right-click menus, profile cards and editing your own profile, replies
  with jump-to-original, edits, deleting and forwarding messages, drafts
  that survive a restart, link previews, reactions with a floating picker,
  selectable and copyable text, mentions inbox, search across a space or all
  rooms with Discord's filters (the homeserver's for unencrypted rooms), read
  markers and unread divider, reports, moderation, permissions, space and
  room alerts.
- **Getting in:** invites to spaces, rooms and DMs with Accept and Decline;
  `matrix.to` links and `#room:server` addresses open in the app with a
  preview and Join, also from Join a Server; Explore searches the
  homeserver's public spaces and rooms, and a space lists the channels you
  haven't joined yet; New message finds people in the user directory and
  starts encrypted DMs and group DMs; blocking someone keeps their messages,
  invites and calls away.
- **History:** older messages keep loading as you scroll up, rooms open at
  their newest messages with a screenful, Page Up/Down, and a Jump to
  present bar. A room you left scrolled up opens at the same message when
  you come back, also after a restart; each server opens its last channel,
  and the app the room it closed on. Rooms you leave let go of the history
  you loaded. A room upgraded in Element reads as one channel with its
  older rooms' history.
- **People:** profile pictures in messages, DMs, the member list, calls and
  the user panel; online, idle and Do Not Disturb dots with custom statuses
  where the homeserver shares presence, and your own status from the user
  panel; a nickname per server (Edit Server Profile), and Owner, Admin and
  Moderator badges from each room's power levels.
- **Files and media:** drop, paste and pick files with progress, cancel and
  retry; every image previews inline, keeping its space while it loads;
  animated GIFs, WebPs, stickers and profile pictures, paused off screen;
  galleries for pictures sent together; videos and audio playing in the
  timeline; posters for videos you send; a media viewer with zoom, a
  thumbnail strip, preloading and Copy Image, playing video on Linux and in
  the browser.
- **Calls:** encrypted DM calls and voice rooms with Element interop (a click
  on a voice channel joins it), screen and window sharing with app audio,
  cameras, push-to-talk (also global), the call dock, per-participant
  volume, recovery after network loss.
- **Encryption:** older history's keys come from key backup a room at a
  time; a message that can't be decrypted says why when that's known.

## Next steps

Ordered by what a community moving from Discord misses most, then by effort.
Each step names the check that says it's done; add it to the QA scripts
(`scripts/qa_ui.py` and friends) as it lands.

### Matrix support

The **Matrix** column says where each step stands in the protocol: **Spec**
(any up-to-date homeserver), an accepted or unstable **MSC** (a Matrix spec
proposal; read both its unstable and stable names), or **Client**, with
nothing for other clients to read. Where Discord has something Matrix
doesn't:

1. **Prefer** the spec, then accepted MSCs, then MSCs Element already ships,
   so other clients understand what we send.
2. **Store it by who needs to see it:** this device only, in local
   settings; all your devices, in account data under our own name
   (`io.github.koalexbear.untethered.*`); everyone in a room, in our own
   state events, which power levels decide who may change. Account data
   isn't end-to-end encrypted: nothing from an encrypted room goes there
   unencrypted.
3. **Never break other clients:** what we add reads sensibly in Element
   (fallback text, ordinary events), and the UI says when something is
   Untethered-only.
4. **Never fake enforcement:** the server enforces power levels and
   nothing else. Slowmode, timeouts, invite links that expire or run out,
   stage speakers, forum channels and scheduled events need a bot, a server
   feature or an MSC; until then they're left out, or offered with what
   Matrix can enforce (lowering a power level, removing an address).
5. **Ask the server** what it offers (versions, capabilities, presence,
   link previews, call servers) and hide what it doesn't, rather than show
   a button that fails.
6. **Write it up:** a format we invent gets an MSC under our own unstable
   name, so other clients can adopt it.

### Phase 1 — messages that feel like Discord

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **M1. Rich text** · done | Spec (`formatted_body`) | Render received `formatted_body` (and Markdown from clients that only send `body`): bold, italic, strikethrough, inline code, code blocks with a copy button, quotes, lists, links, headings, and spoilers (`data-mx-spoiler`, `\|\|text\|\|`) that reveal on click. Sanitise as Element does. | A message from Element with each construct looks the same in Untethered, and a spoiler stays hidden until clicked. |
| **M2. Composer autocomplete** · done | Spec (`m.mentions`) | `@` members, `#` rooms and `:` emoji shortcodes with a popup above the composer (arrow keys, Tab/Enter), sending proper `m.mentions` and pills; an emoji button that reuses the reaction picker. | Mentioning someone by typing `@mi` + Tab highlights them in Element and in their mentions inbox. |
| **M3. Typing indicators** · done | Spec | Send typing notices while composing (with a privacy toggle) and show "Mira is typing…" / "Several people are typing…" under the composer. | Typing in Element shows the indicator within a second and it clears when they stop or send. |
| **M4. Right-click menus** · done | Client | Context menus for messages (Reply, Edit, Delete, Copy text, Copy link, Mark unread, Pin, Report), users (Profile, Message, Mention, Kick/Ban by permission) and channels (Mark read, Notifications, Copy link, Leave). | Every action in the hover toolbar is also reachable by right-click and keyboard (Shift+F10 / menu key). |
| **M5. Profiles** · done | Spec (profiles; extra fields since v1.16) | Click an avatar or name for a profile card (picture, name, user ID, power level, Message button); edit your own display name and picture in User Settings. | Changing your picture here updates it in Element and in this client's member list. |
| **M6. Delete and drafts** · done | Spec (redactions); drafts per device | Delete (redact) your own messages and, with permission, others'; drafts and staged-file captions survive a restart; a "N new messages since 14:02" bar at the top with Mark as read. | A redaction from either client removes the message in both; a draft typed before quitting is still there after restarting. |
| **M7. Link previews** · done | Spec (the server's `preview_url`); opt-in where encrypted | URL embeds through the homeserver's `preview_url`, off by default in encrypted rooms, with a per-room switch; image links show inline. | A link to a page with Open Graph tags shows its title, description and image; turning previews off stops the request. |
| **M8. Forward** · done | Spec (content sent again) | Forward a message, attachments included, to another room or DM from its toolbar and right-click menu. Encrypted attachments are sent again with the same file and key, not uploaded again. | A message and an image forwarded from an encrypted room show in Element's DM as ordinary messages from you, and no new upload happens. |
| **M9. Polls** · done | Accepted MSC3381 (sent and read under its unstable `org.matrix.msc3381.poll.*` names, as Element does) | Show, vote in and end polls (MSC3381, as Element does), with live results; create one from the composer's **+**. | A poll started in Element shows here; a vote here moves Element's count; ending it shows the same final tally in both. |
| **M10. Voice messages** | Unstable MSC3245, as Element sends it | Record and send a voice message (MSC3245) from the composer, with a waveform; playing received ones is P2. | A voice message recorded here plays in Element with its waveform and length. |
| **M11. Custom emoji and stickers** · done | Spec v1.19 (MSC2545 image packs) | Image packs (MSC2545) for a space and for your account, in the emoji picker, autocomplete and reactions, with a sticker picker. Packs work anywhere, as Discord sells with Nitro: your own packs, and packs from rooms you choose to use everywhere (`m.image_pack.rooms`). Animated ones play (P1). Cinny, FluffyChat, Nheko and SchildiChat show them; check Element before relying on it, and send the `:shortcode:` as fallback text. | An emoji pack added to a space appears in its rooms' picker, and in every room once you choose to use it everywhere; a custom emoji sent here renders in Cinny, and a client without image packs shows its `:shortcode:`. |
| **M12. Names** · done | Spec (per-room names); badges and colours client-side | Per-server nicknames (Matrix's per-room display names) and an Admin or Moderator badge, or a name colour, from the sender's power level, standing in for Discord's role colours. | A nickname set in one space shows only there, also in Element; a moderator's badge shows wherever their name does. |
| **M13. Composer details** · done (no spell check) | Spec (`m.emote`); the rest client-side | Up arrow in an empty composer edits your last message; Escape marks the channel read; a message of only emoji shows larger; code blocks get syntax highlighting; `/me`, `/shrug` and `/spoiler`, listed when `/` starts the composer. Spell check isn't possible yet: Iced 0.14's `text_editor` has no spell checking and no way to underline or replace a misspelt word, and in the browser build the editor is drawn on a canvas, out of the browser's own checker's reach. | Each works as on Discord, and `/me` arrives in Element as an emote. |
| **M14. Selection across messages** | Client | Drag a selection from one message into the next, and copy their text in order. Today a selection stays within one paragraph. | Dragging across three messages and pressing Ctrl+C pastes their three texts. |
| **M15. Profile customization** | Spec (per-room names and pictures; profile fields since v1.16); the extra fields only Untethered shows | A profile picture per server (Matrix's per-room member avatar, alongside M12's nicknames), and a banner, bio and accent colour on your profile card as extended profile fields under our own names. Animated pictures play (P1). | A per-server picture set here shows only in that space, also in Element; a banner and bio set here show on your card in another Untethered. |

### Phase 2 — getting people in

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **J1. Invites** · done | Spec | Invites to spaces, rooms and DMs, shown with who sent them, in the server rail and DM list with **Accept** and **Decline**. Today the room list shows joined rooms only, so invites never appear. | An invite from Element to a space, a room and a DM each shows here within a sync; **Accept** joins and opens it, **Decline** removes it in both clients. |
| **J2. Join by link** · done | Spec (`matrix.to`, `matrix:` links); previews as the server allows | `matrix.to` links and `#room:server` addresses open in the app: rooms you're in open, others show a preview (name, topic, members) with **Join**. **Join a server** in the server rail's **+** takes a link or address. | A link to a room you're not in shows its preview and joins on a click; pasting `#room:server` into **Join a server** does the same; nothing opens a browser. |
| **J3. DMs and blocking** · done | Spec (`m.direct`, user directory, `m.ignored_user_list`) | **New message** with user-directory search for a DM or a group DM, and blocking people (`m.ignored_user_list`): their messages, invites and calls stay away. | A DM and a group DM started here are encrypted rooms Element lists as DMs; blocking someone hides them here and in Element. |
| **J4. Explore** · done | Spec (public room directory; space hierarchy) | The homeserver's public rooms and spaces, searchable, with a preview before joining; a joined space lists the channels you haven't joined yet. | A public space found here joins and shows its channels. |
| **J5. Sign-in everywhere** | Spec v1.15 (OAuth 2.0; matrix.org runs MAS) | OIDC through the Matrix Authentication Service, which matrix.org and other homeservers are moving to, single sign-on in the browser too (desktop-only now), and creating an account where the homeserver allows it. | Signing in to a homeserver that uses MAS works on the desktop and in Chromium, and a new account can be created and verified. |

### Phase 3 — navigation and the desktop

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **N1. Quick switcher** · done | Client | Ctrl+K: fuzzy search over rooms, DMs and spaces, unread first. | Ctrl+K, three letters and Enter opens a room without the mouse. |
| **N2. Remember where you were** · done | Client | Last room per space and scroll position per room, restored on switch and restart. | Leave a room mid-history, visit two others, come back to the same message. |
| **N3. Pins and threads** · done | Spec (pinned events; threads since v1.4) | Pinned messages panel (`m.room.pinned_events`); Matrix threads in a side panel with a thread list per channel. | A thread started in Element shows its replies in the panel; pinning here shows in Element. |
| **N4. Presence** · done | Spec, often off on large servers | Online/idle/offline dots and custom status where the homeserver enables presence; hide them cleanly where it doesn't. | Going idle in Element turns the dot amber here within a minute. |
| **N5. Channel order** · done | Spec (`m.space.child` `order`) | Drag to reorder channels and categories, writing `m.space.child` `order`. Categories are the spaces a space lists; dropping a channel in another moves its edge there. | The order made here appears in Element's space view. |
| **N6. System tray** · done | Client | A tray icon (StatusNotifierItem on KDE and GNOME, for example with `ksni`; the notification area on Windows) with an unread and mention badge. Closing the window keeps Untethered running in the tray, as Discord does (a setting, on by default), so messages, notifications, ringing and calls carry on. The tray menu offers Open, Mute, Deafen, Do Not Disturb and Quit; optionally start at login (XDG autostart, the Windows Run key) minimised to the tray. Not applicable on the web. | Close the window during a call: audio continues, the tray shows the call and a mention badge, Mute in the tray menu mutes, an incoming DM call still rings, and Quit leaves the call and exits cleanly. |
| **N7. Search across a space** · done | Spec (`/search`, unencrypted rooms); a local index for encrypted | Search a space or all rooms with Discord's filters (`from:`, `in:`, `has:file`/`image`/`link`, `before:`, `after:`): the homeserver's search for unencrypted rooms, so it finds history this device never loaded, and a local full-text index for encrypted ones. Results say which rooms were searched and how far back. | A week-old file found with `from:` and `has:file` in an unencrypted channel never opened on this device, and in an encrypted one once indexed. |
| **N8. Server rail order and folders** · done | Order: unstable MSC3230, as Element; folders: own account data | Drag servers to reorder them, saved as Element does (`org.matrix.msc3230.space_order` in each space's account data) so both clients agree, and group them into folders, which only Untethered shows (its own account data, `io.github.koalexbear.untethered.space_folders`). | The order and a folder made here are the same after a restart and on a second device. |
| **N9. Sounds and shortcuts** · done | Client | Sounds for messages, mentions, voice join and leave, mute and deafen, each with a switch; Discord's shortcuts: next unread channel (Alt+Shift+↓), mark a server read (Shift+Escape), and Ctrl+/ for the list. | Each sound plays and can be turned off; Alt+Shift+↓ opens the next unread channel. |

### Phase 4 — media follow-ups

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **P1. Animated images** · done | Client | Play GIF and animated WebP in the timeline and viewer, and animated custom emoji, stickers and profile pictures (pause when off screen). | A GIF sent from Element and an animated profile picture both animate here and stop using CPU when scrolled away. |
| **P2. Inline players** · done | Client (MSC3245 waveforms) | Play videos inline in the timeline as Discord does, plus an audio player for voice notes and `m.audio`. | A short clip plays without opening the viewer; an audio file shows a waveform-less player with seek. |
| **P3. Viewer polish** · done | Client | Zoom and pan, a thumbnail strip, copy image, preloading the neighbouring images, and a gallery layout for several attachments in one message. Matrix sends each attachment as its own message, so pictures sent together by one sender form the gallery. | Ten screenshots step without a loading pause; scroll-zoom keeps the point under the cursor. |
| **P4. Video everywhere** · browser done | Client | Video playback on Windows (Media Foundation or bundled GStreamer) and in the browser (an HTML `<video>` element over the canvas). The browser plays videos in its own player over the viewer; Windows playback is still open. | The same MP4 plays on Linux, Windows and Chromium. |
| **P5. Sending video** · done | Spec (`info`, thumbnails) | Thumbnail, dimensions and duration for videos you send, so other clients show a poster. | A clip sent from here shows its poster frame in Element. |

### Phase 5 — calls

Follow the [calls plan](rust-client/docs/calls-parity-plan.md). Its Phase 1
is done: V2 has cameras send three simulcast layers, each tile receive the
layer its size needs and video nobody can see pause, and A2 runs echo
cancellation, noise suppression and gain control on the microphone, with
Discord's input sensitivity (automatic, or a threshold over a live meter),
saved on this computer. U1 is done: calls show in Discord's Voice Connected
panel above the user panel (status, the call's room linking to its stage,
Disconnect, Camera and Screen), and Mute and Deafen sit in the user panel with
menus of microphones and speakers; muting or deafening before you join starts
the call that way, and the call fixtures click icon-only controls through a
control map (`UNTETHERED_QA_LAYOUT`). U2 is done: User Settings has a sidebar
of pages, and its **Voice & Video** page (from the user panel's gear or either
device menu) has the input and output devices and volumes, a mic test (a local
loopback with a level meter), Voice Activity or Push to Talk with a recorded
key and release delay, input sensitivity, the processing switches, and the
camera with a preview that publishes nothing. Every choice applies to the
call at once and to every later call, and is saved on this computer across
restarts. The panel's Voice Settings popup keeps only shared audio and each
participant's volume. Next come U3–U6: participant menus, Go Live and Watch
Stream, the incoming call card, and stage polish. Stream quality is a choice, up to 60 fps at the
source's resolution, with no tier to pay for.

### Phase 6 — community management

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **C1. Server settings** · done | Spec (name, picture, topic; image packs, MSC2545); banner: own state event | A settings page for a space: name, icon, banner, topic, its channels' names and topics (order is N5's drag), and its emoji pack: upload emoji and stickers, rename, choose their use, delete, or create the pack. Gated on power levels; read-only otherwise. | Changing a space's name, icon and topic here shows in other clients; an emoji added, renamed or deleted here follows in the pickers and the pack's state event; a member without permission sees the page read-only. |
| **C2. Roles and channel permissions** | Spec (power levels); role names and colours: own state event | Role presets on power levels (Admin, Moderator, Member, and named custom levels) and per-channel overrides, such as an announcement channel only moderators post in. | A second account meets every configured limit, as in the existing permission tests. |
| **C3. Moderation history and join links** | Client (from room history); expiring links need a bot | A moderation log (kicks, bans, permission, name and topic changes, with who and when), the bans list, and the space's join links and pending invites, which a moderator can revoke. | A kick and a ban done in Element appear in the log here with their reasons. |

### Phase 7 — settings, account and release

| Step | Matrix | Deliverable | Done when |
|---|---|---|---|
| **S1. Settings pages** | Client; account data where it syncs | Appearance (theme, font scale, compact mode), Notifications, Keybinds, Privacy (read receipts, typing, link previews), Voice & Video (U2). | Every setting persists across restarts and devices where Matrix has an account-data equivalent. |
| **S2. Account** | Spec (devices, cross-signing, key backup) | Logout everywhere, session list with verification state, key backup health, recovery reset. | A new device can be verified and an old one signed out from here. |
| **S3. Release** | Client | Accessibility names and keyboard paths, scaling, signed Windows builds with a runtime test, Flatpak, upgrade tests. | A new user installs, signs in, recovers history, shares a file, calls someone and upgrades without the terminal. |
| **S4. macOS** | Client | A macOS build with its tray (menu bar), notifications, global shortcuts and calls. | A signed macOS build signs in, recovers history, shares a file and calls someone. |
| **S5. Updates and push** | Spec (push gateways, e.g. Sygnal WebPush) | Windows builds that update themselves, and web push so a closed browser tab still notifies where the homeserver has a push gateway. | An update installs without the terminal; a mention reaches a closed tab. |
| **S6. Performance budgets** | Client | A fixture account with a few hundred rooms, and memory and startup budgets the QA scripts check on every change, alongside the scrolling and media fixtures. | A change that pushes memory or startup past its budget fails the check. |

### Known gaps

- Dragging the timeline's scrollbar thumb can snap back to the newest
  message; the wheel and Page Up/Down keep their position. Not yet
  investigated (seen with `anchor_bottom` in Iced 0.14).
- With the tiny-skia renderer, switching images in the viewer can leave a
  one-pixel line from the previous image (partial redraw); wgpu redraws fully.
- Video plays on Linux, where it depends on the system's GStreamer codecs,
  and in the browser; not yet on Windows (P4).
- Profile pictures of members who never posted come from their membership
  event, so they appear once the member list has loaded.
- A text selection stays within one paragraph (M14).
- Image packs (M11): a space's packs are made and edited in its Server
  Settings (C1), but a channel's own pack and your own pack (only under
  Cinny's unstable `im.ponies.user_emotes`; the spec dropped personal
  packs) can't be edited here yet. Custom emoji show their first frame, not
  their animation, and a room's own pack under the unstable name is read
  when its history is first opened.
- A space's banner (C1) is read when the space is first opened in a
  session and whenever its Server Settings load, so another member's new
  banner shows after that, not live.
- Polls under MSC3381's stable `m.poll.*` names don't show: matrix-sdk's
  timeline reads only the unstable `org.matrix.msc3381.poll.*` events
  Element sends, which is also what Untethered sends. Editing a poll before
  anyone votes, as Element allows, isn't offered yet (M9).
- Fetching an older room's keys from backup isn't covered by a live test
  yet: `scripts/qa_room_upgrades.py` has no encrypted rooms with keys only
  in backup.
- Search in encrypted rooms covers what this device has loaded; Load older
  history fetches more on request, but nothing indexes them in the
  background yet, and the homeserver's search has no `before:`/`after:`,
  so those filter what it returns (N7).
- Memory on a large account hasn't been measured since the 2026-11-27
  thread, arena and history changes (S6).

## Checkpoint log

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
- **Progress, Cancel, Retry:** the desktop reports the engine's upload progress; the
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
  Web 1.12.35 recognized the Untethered device in its voice-call participant
  list, kept it through a heartbeat, and removed it on both explicit leave
  and backend disconnect (`scripts/test-media-interop.py --rtc-membership`).
  The **Linux Rust ↔ Element Call audio test** now passes with the embedded
  Element Call widget, an Olm-encrypted sender-key exchange, and OpenID authorization
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
  the tab closed its playback stream without interrupting voice.
- **Outgoing Linux application audio:** With a window already shared,
  choosing a specific playback stream delivered 730→830 Hz to Element
  separately from the 410 Hz mic. A second app playing 1030 Hz through
  the same output was absent at the receiver. Tested with PulseAudio and
  PipeWire-Pulse. Exiting the selected app removed only its audio track;
  ending the window share also closed the recorder. Whole-system audio
  capture is outside this per-stream test.
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
- **Linux portal screen sharing:** [The three-client share test](experiments/rtc-audio/README.md#verify-portal-window-sharing)
  uses a private KDE Wayland picker, which now lists screens as well as windows,
  and PipeWire. Cancelling creates no video publication. Selecting a 1600×900
  animated window publishes one GCM screen track at its own size (1600×928)
  and 30.0 fps, where it used to be letterboxed to 960×540 at 5 fps. Both receivers decoded 30.0 fps
  and rendered its original and changed color pattern while the sharer
  browsed another channel. All six audio paths stayed live on the original
  microphone tracks. **Stop sharing** released the portal session and capture
  nodes, unpublished the video and cleared both remote views. Receiving 30 fps
  takes a full core on the UI thread under software GL (2–4% idle); GPU YUV
  rendering is the follow-up. A full-monitor share and system-audio sharing
  are still unverified.
- **Linux voice-room camera:** [The three-client camera test](experiments/rtc-audio/README.md#verify-voice-room-camera-video)
  uses a real Camera portal with an isolated virtual camera. Both peers
  rendered changing green/magenta → orange/blue frames from a single GCM
  camera publication, sent at 1280×720 (up to 30 fps; the virtual camera
  supplies 12). Turning the camera off removed the reader, PipeWire
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

Keep the frontend in **Rust/Iced**. The Matrix engine is **matrix-rust-sdk** ([migration notes](rust-client/docs/matrix-sdk-migration.md)). Share room, attachment and call state between desktop and web; isolate file access, capture, playback and notifications behind platform adapters. Keep native controls for desktop calls. Launching a hosted Element Call page is useful for testing but isn't the finished native calling experience.

**Across every step:** compare keyboard focus, right-click menus, hover states, scroll anchoring, spacing and empty and error states with Discord. Test accessibility and real Windows builds with each feature, measure responsiveness and memory during transfers and calls, and keep file contents, tokens and call keys out of diagnostics.

**Nitro, free where Matrix allows:** most of what Discord sells with Nitro is a limit, not a technical need. Custom emoji and stickers anywhere (M11), per-server profiles and profile customization (M12, M15), long messages (Matrix allows events up to 64 KiB), HD streaming (the calls plan), animated pictures (P1) and app themes (S1) come with Untethered. Upload size is the homeserver's setting, and a room address does what a boosted server's vanity URL does. Super reactions, avatar decorations and Shop items are left out.

**Later, by demand:** forum channels, stages, scheduled events, bot commands and richer moderation integrations, most of which need a server feature or a bot (see [Matrix support](#matrix-support)). Discord connectivity, billing, a paid tier and a marketplace are separate products.
