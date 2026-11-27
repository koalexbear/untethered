"""Presence (ROADMAP N4), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_presence.py [--off]

The test account shares a room, workshop, and a DM with Mira, whose
presence the fixture sets through the presence API, as her client would
(again every 12 s: Synapse takes a device that stops offline after 30 s).
The release client in a private KWin session must:
- show Mira's dot in the DM list green while she's online, then amber with
  her custom status once she's idle (unavailable), each within a minute;
- show her under Online in workshop's member list and on her profile card
  (Idle, with the custom status), then under Offline once she goes offline,
  her DM dot a grey ring, each within a minute;
- publish its own status, read back through Mira's account: Online at
  first, then Idle, a custom status and Invisible from the user panel's
  status menu, Online again, Idle by itself after UNTETHERED_QA_AWAY (30)
  seconds without input and Online again on the next input.
With --off, Synapse has presence disabled: while Mira keeps setting hers,
no dot shows anywhere, the member list has no Online or Offline groups and
the status menu offers only Do Not Disturb.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import threading
import time
from urllib.parse import quote

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, read, tesseract  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 312, 840)
PANEL = (72, 846, 312, 900)
MAIN = (312, 40, 1180, 900)
MEMBERS = (940, 80, 1180, 900)
AWAY = 30
STATUS = "At the workshop"
OWN = "Testing presence"
ENTER = "\ue007"
ESCAPE = "\ue00c"
# Mira's initials circle (view.rs face(): her name's colour) and the dots.
PURPLE = (0x86, 0x5B, 0x89)
DOTS = {
    "online": (0x23, 0xA5, 0x5A),
    "idle": (0xF0, 0xB2, 0x32),
    "busy": (0xF2, 0x3F, 0x43),
    "offline": (0x80, 0x84, 0x8E),
}


def near(pixel, colour, slack=16):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def dot_in(im, box):
    """The status dot drawn in `box`, by its colour: None when there's none."""
    px = im.load()
    counts = dict.fromkeys(DOTS, 0)
    for y in range(box[1], box[3]):
        for x in range(box[0], box[2]):
            for name, colour in DOTS.items():
                if near(px[x, y], colour):
                    counts[name] += 1
    name, count = max(counts.items(), key=lambda kv: kv[1])
    return name if count >= 8 else None


def avatar(im, region, size):
    """Mira's picture in `region`: its box, from its left and top edges."""
    px = im.load()
    pts = [(x, y) for y in range(region[1], region[3]) for x in range(region[0], region[2])
           if near(px[x, y], PURPLE, 12)]
    if len(pts) < size * 4:
        return None
    left, top = min(p[0] for p in pts), min(p[1] for p in pts)
    return left, top, left + size, top + size


def mira_dot(im, region, size=32):
    """("none" or a status) for the dot in the corner of Mira's picture in
    `region`, or None when her picture isn't there."""
    box = avatar(im, region, size)
    if not box:
        return None
    corner = (box[2] - size * 3 // 8, box[3] - size * 3 // 8, box[2] + 1, box[3] + 1)
    return dot_in(im, corner) or "none"


class Keeper:
    """Mira's presence, put again every 12 s as her client would."""

    def __init__(self, hs, user, hdr):
        self.url = f"{hs}/_matrix/client/v3/presence/{quote(user, safe='')}/status"
        self.hdr = hdr
        self.body = {"presence": "online"}
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)

    def put(self):
        with self.lock:
            body = dict(self.body)
        for _ in range(10):
            r = requests.put(self.url, headers=self.hdr, json=body, timeout=15)
            if r.status_code != 429:
                r.raise_for_status()
                return
            time.sleep(r.json().get("retry_after_ms", 1000) / 1000 + .2)
        raise RuntimeError("Mira's presence stayed rate limited")

    def set(self, presence, status_msg=None):
        with self.lock:
            self.body = {"presence": presence}
            if status_msg:
                self.body["status_msg"] = status_msg
        self.put()
        return time.monotonic()

    def run(self):
        while not self.stop.wait(12):
            self.put()


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--off", action="store_true", help="Synapse with presence disabled: no dots must show")
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-presence-", dir=os.environ.get("TMPDIR")))
    synapse = None
    keeper = None
    try:
        extra = "presence:\n  enabled: false\n" if args.off else ""
        synapse, hs = homeserver(tmp, extra)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        # Our initials circle mustn't be Mira's colour (view.rs face()).
        http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(me, safe='')}/displayname", headers=hdr,
             json={"displayname": "Tester"})
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))
        http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(mira, safe='')}/displayname", headers=mira_hdr,
             json={"displayname": "Mira"})

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"
        workshop = create(name="workshop", preset="private_chat", invite=[mira])
        dm = create(preset="trusted_private_chat", is_direct=True, invite=[mira])
        http("PUT", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/m.direct", headers=hdr,
             json={mira: [dm]})
        for room in (workshop, dm):
            http("POST", f"{path(room)}/join", headers=mira_hdr, json={})
        http("PUT", f"{path(workshop)}/send/m.room.message/{secrets.token_hex(8)}", headers=mira_hdr,
             json={"msgtype": "m.text", "body": "Morning, everyone"})

        keeper = Keeper(hs, mira, mira_hdr)
        keeper.set("online")
        keeper.thread.start()

        def ours():
            """Our presence as Mira's account reads it."""
            return http("GET", f"{hs}/_matrix/client/v3/presence/{quote(me, safe='')}/status",
                        headers=mira_hdr).json()

        def saved():
            r = requests.get(f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/"
                             "io.github.koalexbear.untethered.status", headers=hdr, timeout=15)
            return r.json() if r.ok else None

        print(f"Synapse at {hs} (presence {'off' if args.off else 'on'}): {me} with {mira}", flush=True)
        look(tmp, args.shots, args.off, me, keeper, ours, saved)
        if args.off:
            print("PASS: with presence off no dot shows, the member list isn't split and the status menu offers "
                  "only Do Not Disturb", flush=True)
        else:
            print("PASS: Mira's dot and custom status follow her presence within a minute, and our own status "
                  "goes out from the status menu and by itself after inactivity", flush=True)
    finally:
        if keeper:
            keeper.stop.set()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, off, me, keeper, ours, saved):
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    localpart = me[1:].split(":", 1)[0]
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off", UNTETHERED_QA_AWAY=str(AWAY))
        kde = KWin(stack, ui, ROOT, None, env=env)
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)

        def image():
            return kde.capture().crop(WINDOW).convert("RGB")

        def shot(name, im=None):
            im = im or image()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=WINDOW, sidebar=False):
            im = image().crop(rect)
            return [(clean(w), rect[0] + x, rect[1] + y) for w, x, y in read(im, sidebar=sidebar)]

        def word(part, rect=WINDOW, sidebar=False):
            return next(((x, y) for w, x, y in words(rect, sidebar) if part in w), None)

        def small(part, rect):
            """Small grey text, such as menu notes, read at four times its size."""
            return any(part in clean(w) for w, _, _ in tesseract(image().crop(rect), scale=4))

        def hover(pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}])

        def until(description, check, timeout, since=None):
            """Wait for `check`; on a timeout keep a screenshot. Returns the
            result and how long it took from `since`."""
            try:
                result = wait_for(description, check, timeout)
            except RuntimeError:
                shot("timeout-" + description.replace(" ", "-").replace("'", ""))
                raise
            return result, (time.monotonic() - since if since else None)

        def dm_dot():
            return mira_dot(image(), SIDEBAR)

        def own_dot():
            return dot_in(image(), (PANEL[0], PANEL[1], PANEL[0] + 50, PANEL[3]))

        def member(status):
            """Mira's member-list row as ("ONLINE"/"OFFLINE"/"MEMBERS" heading
            above her, her dot), once both read as expected."""
            im = image()
            found = mira_dot(im, MEMBERS)
            box = avatar(im, MEMBERS, 32)
            if not box:
                return None
            heads = [(clean(w), y) for w, x, y in read(im.crop(MEMBERS))
                     if clean(w) in ("online", "offline", "members")]
            above = [w for w, y in heads if MEMBERS[1] + y < box[1]]
            group = above[-1] if above else None
            return (group, found) if (group, found) == status else None

        # Signed in: the rooms list, then Direct Messages from the rail.
        until("workshop in the sidebar", lambda: word("workshop", SIDEBAR, True), 90)
        tap(kde, 36, 64)
        until("Mira's DM", lambda: word("mira", SIDEBAR, True) and dm_dot() is not None, 30)

        if off:
            check_off(kde, shot, image, words, word, small, until, dm_dot, own_dot, member, keeper, localpart, ours)
            return

        _, took = until("Mira's dot green", lambda: dm_dot() == "online", 60, time.monotonic())
        print(f"UI: Mira's DM shows her online, {took:.1f} s after the DM list opened", flush=True)
        since = keeper.set("unavailable", STATUS)
        _, took = until("Mira's dot amber", lambda: dm_dot() == "idle", 60, since)
        until("her custom status in the DM list", lambda: word("workshop", SIDEBAR, True), 10)
        shot("dm-idle")
        print(f"UI: Mira's dot turned amber {took:.1f} s after she went idle, with \"{STATUS}\" under her name",
              flush=True)

        # The member list and her profile card.
        tap(kde, 36, 120)
        tap(kde, *until("workshop", lambda: word("workshop", SIDEBAR, True), 20)[0])
        until("Mira's message", lambda: word("morning", MAIN), 30)
        header = Ui(lambda: words(), hover, image, width=WINDOW[2], height=WINDOW[3] - TITLE_BAR,
                    origin=(0, TITLE_BAR))
        tap(kde, *until("Member List", lambda: header.header("member list"), 20)[0])
        until("Mira under Online, amber", lambda: member(("online", "idle")), 60)
        until("her custom status in the member list", lambda: word("workshop", MEMBERS), 10)
        shot("members-idle")
        print("UI: workshop's member list has Mira under Online with an amber dot and her custom status", flush=True)
        name = until("Mira's name on her message", lambda: word("mira", (330, 80, 940, 860)), 10)[0]
        tap(kde, *name)
        until("her profile card, idle", lambda: word("idle", MAIN) and word("workshop", (330, 200, 940, 700))
              and mira_dot(image(), (330, 80, 940, 860), 64) == "idle", 20)
        shot("profile-card")
        tap(kde, *until("the card's Close", lambda: word("close", (330, 200, 940, 700)), 10)[0])
        print("UI: her profile card shows the amber dot and \"Idle · " + STATUS + "\"", flush=True)

        since = keeper.set("offline")
        _, took = until("Mira under Offline", lambda: member(("offline", "none")), 60, since)
        shot("members-offline")
        print(f"UI: Mira moved under Offline, without a dot, {took:.1f} s after she went offline", flush=True)
        tap(kde, 36, 64)
        until("her DM dot grey", lambda: dm_dot() == "offline", 20)
        print("UI: her DM shows a grey ring", flush=True)

        # Our own status, read back through Mira's account.
        hover((700, 450))  # input: not idle by itself yet
        until("ours online", lambda: ours().get("presence") == "online", 30)
        until("our dot green", lambda: own_dot() == "online", 10)
        print("Matrix: our own presence is online, the user panel's dot green", flush=True)

        def menu(entry):
            tap(kde, 150, 873)
            spot = until(f"{entry} in the status menu", lambda: word(entry, (72, 400, 400, 846)), 10)[0]
            shot("status-menu")
            tap(kde, *spot)

        menu("idle")
        until("ours unavailable", lambda: ours().get("presence") == "unavailable", 30)
        until("our dot amber", lambda: own_dot() == "idle" and word("idle", PANEL), 10)
        assert (saved() or {}).get("status") == "idle", saved()
        print("Matrix: Idle from the status menu publishes unavailable and saves the choice", flush=True)
        menu("custom")
        until("the custom status dialog", lambda: word("cooking", WINDOW), 10)
        kde.type(OWN)
        kde.keys(("Down", ENTER), ("Up", ENTER))
        got = until("our custom status", lambda: (p := ours()).get("status_msg") == OWN and p, 30)[0]
        assert got["presence"] == "unavailable", got
        until("it in the user panel", lambda: word("testing", PANEL), 10)
        assert (saved() or {}).get("message") == OWN, saved()
        print(f"Matrix: the custom status \"{OWN}\" goes out with it and shows in the user panel", flush=True)
        menu("invisible")
        got = until("ours offline", lambda: (p := ours()).get("presence") == "offline" and p, 30)[0]
        assert not got.get("status_msg"), f"Invisible hides the custom status: {got}"
        until("our dot grey", lambda: own_dot() == "offline" and word("invisible", PANEL), 10)
        print("Matrix: Invisible publishes offline, without the custom status", flush=True)
        menu("online")
        until("ours online again", lambda: (p := ours()).get("presence") == "online" and p.get("status_msg") == OWN,
              30)
        print("Matrix: Online again, with the custom status", flush=True)

        # Idle by itself, and back on input.
        hover((700, 450))
        since = time.monotonic()
        _, took = until("ours idle by itself", lambda: ours().get("presence") == "unavailable", AWAY + 40, since)
        until("our dot amber", lambda: own_dot() == "idle", 10)
        print(f"Matrix: idle by itself {took:.1f} s after the last input (UNTETHERED_QA_AWAY={AWAY})", flush=True)
        hover((720, 470))
        since = time.monotonic()
        _, took = until("ours online on input", lambda: ours().get("presence") == "online", 20, since)
        until("our dot green", lambda: own_dot() == "online", 10)
        shot("back-online")
        print(f"Matrix: online again {took:.1f} s after the pointer moved", flush=True)


def check_off(kde, shot, image, words, word, small, until, dm_dot, own_dot, member, keeper, localpart, ours):
    """Presence off: Mira's status changes show nowhere."""
    keeper.set("unavailable", STATUS)
    assert ours() == {"presence": "offline"}, f"Synapse with presence off: {ours()}"
    # Longer than the client's reading interval (40 s) and its own check.
    end = time.monotonic() + 50
    while time.monotonic() < end:
        assert dm_dot() == "none", "a dot shows on Mira's DM with presence off"
        assert own_dot() is None, "the user panel has a status dot with presence off"
        time.sleep(2)
    assert not word("workshop", SIDEBAR, True), "her custom status shows with presence off"
    assert word(localpart[:5], PANEL), "the user panel shows our Matrix ID instead of a status"
    shot("off-dm")
    print("UI: no dot or custom status on Mira's DM or in the user panel for 50 s", flush=True)
    tap(kde, 36, 120)
    tap(kde, *until("workshop", lambda: word("workshop", SIDEBAR, True), 20)[0])
    until("Mira's message", lambda: word("morning", MAIN), 30)
    hover = lambda pos: kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}])  # noqa: E731
    header = Ui(lambda: words(), hover, image, width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
    tap(kde, *until("Member List", lambda: header.header("member list"), 20)[0])
    until("Mira under Members, no dot", lambda: member(("members", "none")), 30)
    shot("off-members")
    print("UI: the member list keeps one Members group, without dots", flush=True)
    tap(kde, 150, 873)
    until("the status menu", lambda: word("disturb", (72, 400, 400, 846)) and small("share", (72, 400, 320, 846)), 10)
    assert not word("invisible", (72, 400, 400, 846)), "Invisible offered with presence off"
    shot("off-menu")
    kde.keys(("Down", ESCAPE), ("Up", ESCAPE))
    print("UI: the status menu offers only Do Not Disturb and says the homeserver doesn't share who's online",
          flush=True)


if __name__ == "__main__":
    main()
