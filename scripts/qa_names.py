"""Names (ROADMAP M12), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_names.py

The test account, Quinn, runs a space, Guild, with two channels, general and
random, and also shares lobby, a room in no space, with Enzo on a gomuks
device. Enzo moderates Guild and its channels (power level 50) but not
lobby. The release client in a private KWin session (as qa_kwin.py needs)
must:
- show Enzo's MOD badge and Quinn's (the creator's: OWNER in a version 12
  room, ADMIN before) on their messages in general, none for Enzo in lobby,
  and Moderator on Enzo's profile card;
- set the nickname Captain for Guild from the server menu's Edit Server
  Profile: Enzo's device reads it from Quinn's member events in Guild,
  general and random, and the old name in lobby, as Element would; the
  timeline shows it in general and the old name in lobby;
- give news, a channel Enzo adds to Guild that Quinn joins afterwards, the
  nickname too;
- reset it from the right-click menu on Quinn's own name, which puts the
  global name back in every room of Guild.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build. From a worktree, point UNTETHERED_BACKEND at the main
checkout's `.dev/backends/…/gomuks`.
"""
import argparse
import contextlib
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, Gomuks, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, line_words, read  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
ENTER = ""
ESCAPE = ""
# The rail's first space, under Direct Messages, All Rooms and the rule
# (view.rs: 48 px buttons 8 apart, from 12 px under the title bar).
FIRST_SPACE = (36, 186)
ALL_ROOMS = (36, 120)
GLOBAL = "Quinn"
NICK = "Captain"
HELLO = "Hello from the guild hall"
ORDER = "Moderators keep order here"
LOBBY = "Hello from the lobby"
CHATTER = "Lobby chatter"


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-names-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))
        api = hs + "/_matrix/client/v3"
        http("PUT", f"{api}/profile/{quote(me, safe='')}/displayname", headers=hdr, json={"displayname": GLOBAL})
        http("PUT", f"{api}/profile/{quote(enzo, safe='')}/displayname", headers=enzo_hdr,
             json={"displayname": "Enzo"})

        def path(room):
            return f"{api}/rooms/{quote(room, safe='')}"

        def create(headers, **body):
            return http("POST", api + "/createRoom", headers=headers, json=body).json()["room_id"]

        def say(headers, room, body):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers,
                 json={"msgtype": "m.text", "body": body})

        # Room version 12 creators have no power level, they outrank them all.
        guild_power = {"users": {enzo: 50}}
        space = create(hdr, name="Guild", preset="private_chat", invite=[enzo],
                       creation_content={"type": "m.space"}, power_level_content_override=guild_power)
        general = create(hdr, name="general", preset="private_chat", invite=[enzo],
                         power_level_content_override=guild_power)
        random = create(hdr, name="random", preset="private_chat", invite=[enzo],
                        power_level_content_override=guild_power)
        lobby = create(hdr, name="lobby", preset="private_chat", invite=[enzo])
        for channel in (general, random):
            http("PUT", f"{path(space)}/state/m.space.child/{quote(channel, safe='')}", headers=hdr,
                 json={"via": [SERVER_NAME]})
        for room in (space, general, random, lobby):
            http("POST", f"{path(room)}/join", headers=enzo_hdr, json={})
        say(hdr, general, HELLO)
        say(enzo_hdr, general, ORDER)
        say(hdr, lobby, LOBBY)
        say(enzo_hdr, lobby, CHATTER)

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        for room in (space, general, random, lobby):
            peer.joined(room)

        def stored_name(room):
            """Quinn's name in `room`, from the homeserver's current state."""
            return http("GET", f"{path(room)}/state/m.room.member/{quote(me, safe='')}",
                        headers=hdr).json().get("displayname")

        names = Names(peer, me)
        rooms = dict(space=space, general=general, random=random, lobby=lobby)
        version = http("GET", f"{path(general)}/state/m.room.create", headers=hdr).json().get("room_version", "1")
        # The creator: Owner in a version 12 room, Admin at 100 before.
        badge = "owner" if version.isdigit() and int(version) >= 12 else "admin"
        print(f"Synapse at {hs}: {me} ({GLOBAL}) runs Guild with general and random, shares lobby with "
              f"{enzo} on gomuks, who moderates Guild", flush=True)

        def add_news():
            news = create(enzo_hdr, name="news", preset="private_chat", invite=[me])
            http("PUT", f"{path(space)}/state/m.space.child/{quote(news, safe='')}", headers=enzo_hdr,
                 json={"via": [SERVER_NAME]})
            peer.joined(news)
            # Joined from another device of Quinn's: the client sees a new
            # channel of Guild that still shows the global name.
            http("POST", f"{path(news)}/join", headers=hdr, json={})
            rooms["news"] = news
            return news

        look(tmp, args.shots, rooms, names, stored_name, add_news, badge)
        print("PASS: badges follow each room's power levels; a nickname set for Guild shows in its rooms only, "
              "reaches a channel joined later, and resets from the right-click menu", flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


class Names:
    """Quinn's member events as Enzo's gomuks device reads them."""

    def __init__(self, peer, me):
        self.peer, self.me = peer, me

    def seen(self, room):
        """Quinn's newest member event in `room` among what gomuks synced."""
        events = [e for e in self.peer.events if e.get("room_id") == room and e.get("type") == "m.room.member"
                  and e.get("state_key") == self.me and str(e.get("event_id", "")).startswith("$")]
        return max(events, key=lambda e: e.get("timestamp", 0), default=None)

    def state(self, room):
        """Quinn's name in `room` from gomuks's room state."""
        state = self.peer.rpc("get_room_state", {"room_id": room, "include_members": True,
                                                 "fetch_members": False, "refetch": False})
        event = next((e for e in state or [] if e.get("type") == "m.room.member"
                      and e.get("state_key") == self.me), None)
        return (event or {}).get("content", {}).get("displayname")

    def wait(self, room, name, timeout=60):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            event = self.seen(room)
            if event and (event.get("content") or {}).get("displayname") == name:
                return event
            self.peer.frame(1)
        raise RuntimeError(f"gomuks never saw {name!r} in {room}; newest: {self.seen(room)}")


def look(tmp, shots, rooms, names, stored_name, add_news, badge):
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off")
        kde = KWin(stack, ui, ROOT, None, env=env)
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)

        def shot(name):
            im = kde.capture().crop(WINDOW).convert("RGB")
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=MAIN):
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def word(part, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if part in w), None)

        def side(part):
            """A grey channel name in the sidebar, read at four times its size."""
            found = read(kde.capture().crop(WINDOW), sidebar=True, dialogs=False)
            return next(((x, y) for w, x, y in found
                         if part in clean(w) and SIDEBAR[0] < x < SIDEBAR[2] and SIDEBAR[1] < y < SIDEBAR[3]), None)

        def phrase(*parts, rect=MAIN, gap=160):
            """Where `parts` read in order on one line."""
            ws = words(rect)
            for w, x, y in ws:
                if w != parts[0]:
                    continue
                at, ok = x, True
                for part in parts[1:]:
                    nxt = [(vx, vy) for v, vx, vy in ws if v == part and abs(vy - y) < 8 and 0 < vx - at < gap]
                    if not nxt:
                        ok = False
                        break
                    at = min(nxt)[0]
                if ok:
                    return x, y
            return None

        def tagged(name, tag):
            """Where `name` reads with the badge `tag` right after it: the
            10 px badge is read at four times its size (whole-window OCR
            reads MOD as "mob")."""
            im = kde.capture()
            for w, x, y in words():
                if w != name:
                    continue
                line = [v for v, vx, _ in line_words(im, x, y) if v and vx > x - 30]
                # One misread letter in a longer tag ("gwner" for the gold OWNER).
                if len(line) >= 2 and line[0] == name and (line[1] == tag or len(tag) > 3 and len(line[1]) == len(tag)
                                                            and sum(a != b for a, b in zip(line[1], tag)) <= 1):
                    return x, y
            return None

        def settled(label, find, timeout=20):
            """Where `find` reads twice in a row, a moment apart: the timeline
            keeps its bottom in place, so a line arriving below moves
            everything above it."""
            def still():
                first = find()
                time.sleep(0.8)
                second = find()
                return first if first and second and abs(first[0] - second[0]) < 4 \
                    and abs(first[1] - second[1]) < 4 else None
            return wait_for(label, still, timeout)

        def press(button, pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}, {"type": "pause", "duration": 250},
                                    {"type": "pointerDown", "button": button}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": button}])

        def space_open(name):
            return any(w == name and SIDEBAR[0] < x < SIDEBAR[2] and y < 90 for w, x, y in words(WINDOW)) \
                or (side(name) or (0, 999))[1] < 90

        def away():
            """Move the pointer off the rail, whose tooltips cover the sidebar."""
            kde.actions("pointer", [{"type": "pointerMove", "x": 760, "y": 640}])
            time.sleep(0.4)

        def rail(at):
            tap(kde, *at)
            away()

        def open_guild():
            def opened():
                if space_open("guild"):
                    return True
                rail(FIRST_SPACE)
                time.sleep(1)
            wait_for("Guild selected in the rail", opened, 30)

        def open_room(name):
            try:
                at = wait_for(f"{name} in the sidebar", lambda: word(name, SIDEBAR) or side(name), 60)
            except RuntimeError:
                shot(f"missing-{name}")
                raise
            tap(kde, *at)

        def card():
            """Enzo's profile card: "Moderator · room power level 50"."""
            return phrase("moderator", "room", "power", rect=WINDOW)

        def dialog():
            return phrase("edit", "server", "profile", rect=WINDOW) and word("nickname", WINDOW)

        def everywhere(expected, label):
            for key in ("space", "general", "random", "news"):
                if key in rooms:
                    wait_for(f"{label} in {key} on the homeserver", lambda k=key: stored_name(rooms[k]) == expected, 60)
                    names.wait(rooms[key], expected)

        toolbar = Ui(lambda: words(WINDOW), lambda pos: None, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))

        def steps():
            # Badges, from each room's power levels.
            open_guild()
            open_room("general")
            wait_for("Enzo's message", lambda: word("order"), 60)
            try:
                enzo_at = settled("Enzo's MOD badge", lambda: tagged("enzo", "mod"), 30)
                settled(f"Quinn's {badge.upper()} badge", lambda: tagged("quinn", badge), 30)
            finally:
                shot("badges-general")
            print(f"UI: in general, Enzo's messages carry MOD and Quinn's {badge.upper()}, from its power levels",
                  flush=True)
            tap(kde, *enzo_at)
            try:
                wait_for("Moderator on Enzo's card", card, 15)
            finally:
                shot("enzo-card")
            kde.keys(("Down", ESCAPE), ("Up", ESCAPE))
            wait_for("the card closed", lambda: not card(), 10)
            print("UI: Enzo's profile card says Moderator · room power level 50", flush=True)

            # Edit Server Profile from the server menu.
            tap(kde, *wait_for("Edit Server Profile in the server menu",
                               lambda: toolbar.space_menu("Edit Server Profile", lambda p: tap(kde, *p)), 20))
            try:
                wait_for("Edit Server Profile", dialog, 10)
            finally:
                shot("edit-server-profile")
            kde.type(NICK)
            time.sleep(0.4)
            kde.keys(("Down", ENTER), ("Up", ENTER))
            wait_for("the dialog closed", lambda: not dialog(), 60)
            everywhere(NICK, "the nickname")
            assert stored_name(rooms["lobby"]) == GLOBAL, stored_name(rooms["lobby"])
            assert names.state(rooms["lobby"]) == GLOBAL, names.state(rooms["lobby"])
            assert not any((e.get("content") or {}).get("displayname") == NICK for e in names.peer.events
                           if e.get("room_id") == rooms["lobby"]), "the nickname reached lobby"
            print(f"Matrix: Enzo's device read {NICK} from Quinn's member events in Guild, general and random, "
                  f"and {GLOBAL} in lobby", flush=True)
            try:
                settled("the nickname on Quinn's message", lambda: tagged(NICK.lower(), badge), 30)
            finally:
                shot("nickname-general")
            assert not tagged("quinn", badge), "the old name still shows in general"
            rail(ALL_ROOMS)
            open_room("lobby")
            wait_for("Enzo's lobby message", lambda: word("chatter"), 60)
            try:
                settled("the global name in lobby", lambda: word("quinn"), 30)
            finally:
                shot("global-name-lobby")
            assert not word(NICK.lower()), "the nickname shows in lobby"
            assert not tagged("enzo", "mod"), "Enzo has no power in lobby"
            print(f"UI: Quinn's message reads {NICK} in general and {GLOBAL} in lobby, where Enzo has no badge",
                  flush=True)

            # A channel joined after the nickname was set takes it.
            news = add_news()
            wait_for("the nickname in news", lambda: stored_name(news) == NICK, 90)
            names.wait(news, NICK)
            print(f"Matrix: news, added to Guild by Enzo and joined afterwards, took {NICK} too", flush=True)

            # Reset from the right-click menu on Quinn's own name.
            open_guild()
            open_room("general")
            mine = settled("Quinn's name in general", lambda: tagged(NICK.lower(), badge), 30)
            for attempt in range(3):
                press(2, mine)
                try:
                    entry = wait_for("Edit Server Profile in the menu", lambda: phrase("edit", "server", rect=WINDOW), 6)
                    break
                except RuntimeError:
                    if attempt == 2:
                        shot("own-name-menu")
                        raise
            shot("own-name-menu")
            tap(kde, *entry)
            wait_for("Edit Server Profile", dialog, 10)
            tap(kde, *wait_for("Reset Nickname", lambda: phrase("reset", "nickname", rect=WINDOW), 10))
            wait_for("the dialog closed", lambda: not dialog(), 60)
            everywhere(GLOBAL, "the global name")
            try:
                settled("the global name back in general", lambda: tagged("quinn", badge), 30)
            finally:
                shot("reset-general")
            print(f"Matrix and UI: Reset Nickname from Quinn's own name put {GLOBAL} back in Guild, its channels and "
                  "news", flush=True)


        try:
            steps()
        except Exception:
            shot("failure")
            raise

if __name__ == "__main__":
    main()
