"""Right-click menus (ROADMAP M4) in the native client on a private KWin output.

A second account writes in the room over the client-server API, so the menus
are checked on someone else's message as well as your own. The fixture checks
which entries each menu offers by sender and by power level, that Copy
Message Link and Copy Text reach the clipboard, that Mark as Read and Leave
reach the homeserver (Leave only after its confirmation), and that Shift+F10,
the arrow keys, Enter, Escape and a click outside the menu work.

    uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \\
      --with playwright --with pillow python scripts/test-media-interop.py --context-menus

Needs the debug client (it seeds the account), KWin, KDE inputsynth
(KWIN_INPUTSYNTH), Spectacle, kdotool, wl-paste and Tesseract. The menus are
driven in a release build when one exists, since winit panics under inputsynth
in debug builds; UNTETHERED_QA_UI_CLIENT picks another binary.
"""
import os
import secrets
import subprocess as sp
import time
from contextlib import ExitStack
from urllib.parse import quote

from qa_kwin import KWin
from qa_native_paste import wait_for
from qa_ui import CHAT_X, clean, read

TITLE_BAR = 28  # KWin's decoration above the 1180×960 window's content.
WIDTH, HEIGHT = 1180, 960
SHIFT, F10, DOWN, ENTER, ESCAPE = "", "", "", "", ""


def test(tmp, hs, root, backend, seed, http):
    native, hdr = seed(tmp, hs, encrypted=False)
    me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
    # The other person only writes plain messages, so needs no client of its own.
    # Letters OCR can't confuse with digits (1/l/i, 0/o, 5/s).
    peer_name = "peer" + "".join(secrets.choice("acdefhkmnprtuvwxyz") for _ in range(5))
    auth = http("POST", hs + "/_matrix/client/v3/register", json={
        "auth": {"type": "m.login.dummy"}, "username": peer_name,
        "password": secrets.token_urlsafe(24)}).json()
    ph = {"Authorization": "Bearer " + auth["access_token"]}
    peer = auth["user_id"]

    def api(room):
        return hs + "/_matrix/client/v3/rooms/" + quote(room, safe="")

    def send(headers, room, body):
        return http("PUT", api(room) + "/send/m.room.message/" + secrets.token_hex(8), headers=headers,
                    json={"msgtype": "m.text", "body": body}).json()["event_id"]

    room = native["room"]
    side = http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr,
                json={"name": "side-room", "preset": "private_chat"}).json()["room_id"]
    for r in (room, side):
        http("POST", api(r) + "/invite", headers=hdr, json={"user_id": peer})
        http("POST", api(r) + "/join", headers=ph, json={})
    send(hdr, side, "Side room opened")
    send(hdr, room, "Sunflower from me")
    theirs = send(ph, room, "Orchid from peer")

    release = root / "rust-client/target/release/untethered-iced"
    ui_client = os.environ.get("UNTETHERED_QA_UI_CLIENT") or (str(release) if release.is_file() else None)
    if ui_client:
        os.environ["UNTETHERED_QA_CLIENT"] = ui_client
    with ExitStack() as stack:
        kde = KWin(stack, tmp, root, backend)
        win = wait_for("client window", lambda: kde.window("Untethered"), 45)
        kde.position(win, 0, 0, WIDTH, HEIGHT)

        def words():
            im = kde.capture().crop((0, 0, WIDTH, HEIGHT))
            return [(clean(w), x, y) for w, x, y in read(im, origin=(0, TITLE_BAR), sidebar=True)]

        def find(label, where=lambda x, y: x > CHAT_X, timeout=25):
            def hit():
                return next(((x, y) for w, x, y in words() if w == label and where(x, y)), None)
            try:
                return wait_for(label, hit, timeout)
            except RuntimeError:
                print("Visible words:", [w for w, _, _ in words()], flush=True)
                raise

        def sidebar(x, y):
            return 72 < x < CHAT_X

        def press(button, x, y):
            kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 120},
                                    {"type": "pointerDown", "button": button}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": button}])

        def hover(x, y):
            kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}])
            time.sleep(.4)

        def near(pos):
            # A menu opens at the pointer (to its left near the right edge) and
            # moves up to fit: its words are in a 230 px column around there.
            x, y = pos
            left = x - 232 if x > WIDTH - 240 else x - 4
            return {w: (wx, wy) for w, wx, wy in words() if left <= wx <= left + 236 and y - 340 <= wy <= y + 340}

        def menu(pos, want, absent=(), what="menu"):
            # inputsynth sometimes drops an event (see the live-testing notes),
            # so a right click that shows nothing is tried again; a menu that
            # never opens still fails.
            for attempt in range(3):
                press(2, *pos)
                try:
                    shown = wait_for(what, lambda: (ws := near(pos)) and set(want) <= set(ws) and ws, 6)
                    break
                except RuntimeError:
                    if attempt == 2:
                        print("Words near", pos, sorted(near(pos)), flush=True)
                        raise
                    print(f"No {what} after a right click; trying again.", flush=True)
            unexpected = set(absent) & set(shown)
            assert not unexpected, f"{what} offers {unexpected}"
            return shown

        def closed(pos, word):
            wait_for("menu closed", lambda: word not in near(pos), 10)

        def author(line):
            """Where the author's name is on the header above message `line`:
            the word just left of its "Today at" time."""
            def hit():
                ws = words()
                today = [(x, y) for w, x, y in ws if w == "today" and x > CHAT_X and y < line[1]]
                if not today:
                    return None
                tx, ty = max(today, key=lambda p: p[1])
                names = [(x, y) for w, x, y in ws if abs(y - ty) <= 6 and CHAT_X < x < tx]
                return max(names) if names else None
            return wait_for("author above " + str(line), hit, 15)

        def clipboard():
            return sp.run(["wl-paste", "--no-newline"], env=kde.env, capture_output=True, text=True,
                          timeout=5).stdout

        if not kde.window("media-proof"):
            press(0, *find("media-proof", sidebar))
        wait_for("media-proof open", lambda: kde.window("media-proof"), 20)
        orchid = find("orchid")
        sunflower = find("sunflower")
        # Unread from here on: the client has the side room open neither now nor later.
        tulip = send(ph, side, "Tulip waiting")

        # Someone else's message: everything the toolbar offers but Edit, plus Report.
        other = ("reaction", "reply", "text", "unread", "link", "report")
        shown = menu(orchid, other, ("edit",), "menu on the peer's message")
        press(0, *shown["link"])
        closed(orchid, "report")
        link = wait_for("message link on the clipboard", lambda: "matrix.to" in (c := clipboard()) and c, 10)
        assert link == f"https://matrix.to/#/{room}/{theirs}?via=test.invalid", link
        print("Copy Message Link:", link, flush=True)

        # Your own message: Edit, no Report; Copy Text copies the plain body.
        shown = menu(sunflower, ("reaction", "reply", "edit", "text", "link"), ("report",),
                     "menu on your own message")
        press(0, *shown["text"])
        wait_for("message text on the clipboard", lambda: clipboard() == "Sunflower from me", 10)

        # Escape and an outside click close the menu without doing anything else.
        menu(orchid, other)
        kde.keys(("Down", ESCAPE), ("Up", ESCAPE))
        closed(orchid, "report")
        menu(orchid, other)
        press(0, WIDTH // 2, TITLE_BAR + 200)
        closed(orchid, "report")
        assert "replying" not in {w for w, _, _ in words()}

        # People: Kick and Ban only over someone with less power, never yourself.
        shown = menu(author(orchid), ("profile", "mention", "kick", "ban", "copy", "id"), (),
                     "menu on the peer's name")
        press(0, *shown["mention"])
        # The composer was empty. OCR drops the @ and misreads some letters
        # of the random name, so its start is enough.
        wait_for("mention in the composer", lambda: any(
            w.startswith("pee") and len(w) >= 7 and x > CHAT_X and y > HEIGHT - 140 for w, x, y in words()), 10)
        # Your own picture, left of your name.
        name = author(sunflower)
        menu((CHAT_X + 36, name[1] + 12), ("profile", "mention", "copy"), ("kick", "ban"),
             "menu on your own picture")
        kde.keys(("Down", ESCAPE), ("Up", ESCAPE))
        print("Peer's menu offers Kick and Ban to the room's creator; your own menu doesn't.", flush=True)

        # The keyboard: Shift+F10 over a message, then Down and Enter on Reply.
        hover(*orchid)
        kde.keys(("Down", SHIFT), ("Down", F10), ("Up", F10), ("Up", SHIFT))
        wait_for("keyboard-opened menu", lambda: {"reaction", "reply"} <= set(near(orchid)), 10)
        kde.keys(("Down", DOWN), ("Up", DOWN), ("Down", ENTER), ("Up", ENTER))
        find("replying", lambda x, y: x > CHAT_X and y > HEIGHT - 200)
        print("Shift+F10, Down and Enter chose Reply.", flush=True)

        # A channel in the sidebar: Mark as Read reaches the homeserver.
        channel = find("side-room", sidebar)
        shown = menu(channel, ("mark", "read", "copy", "link", "leave"), (), "menu on a channel")
        press(0, *shown["mark"])

        def fully_read():
            r = http("GET", hs + f"/_matrix/client/v3/user/{quote(me)}/rooms/{quote(side, safe='')}"
                     "/account_data/m.fully_read", headers=hdr).json()
            return r.get("event_id") == tulip
        try:
            wait_for("side room marked read on the homeserver", fully_read, 15)
        except RuntimeError:
            raise RuntimeError("Mark as Read did not move m.fully_read to the newest event")
        print("Mark as Read moved m.fully_read in an unopened room to", tulip, flush=True)

        # Leave asks first: Cancel keeps the membership, Leave ends it.
        def membership():
            try:
                return http("GET", api(side) + f"/state/m.room.member/{quote(me)}", headers=hdr).json()["membership"]
            except Exception as e:  # The fixture's http raises on any error status.
                if getattr(getattr(e, "response", None), "status_code", None) == 403:
                    return "leave"
                raise

        def button(label):
            # The dialog's title also says Leave; its buttons are lowest.
            hits = [(x, y) for w, x, y in words() if w == label and x > CHAT_X]
            return max(hits, key=lambda p: p[1]) if hits else None
        for choice in ("cancel", "leave"):
            shown = menu(channel, ("leave",), (), "menu on a channel")
            press(0, *shown["leave"])
            find("rejoining", lambda x, y: True)
            press(0, *wait_for(choice + " button", lambda: button(choice), 10))
            wait_for("dialog closed", lambda: "rejoining" not in {w for w, _, _ in words()}, 10)
            if choice == "cancel":
                time.sleep(1.5)
                assert membership() == "join", "Cancel left the room"
        wait_for("left on the homeserver", lambda: membership() == "leave", 15)
        wait_for("side room gone from the sidebar",
                 lambda: not any(w == "side-room" and sidebar(x, y) for w, x, y in words()), 15)
        print("Leave Channel: Cancel kept the membership; Leave ended it and the room left the sidebar.",
              flush=True)
    print("PASS right-click menus", flush=True)


if __name__ == "__main__":
    raise SystemExit("Run through scripts/test-media-interop.py --context-menus")
