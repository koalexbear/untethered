"""Explore (ROADMAP J4), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_explore.py

A second account, Mira, publishes in the homeserver's room directory a public
space, Weavers Guild, with two public channels, loom-talk (with a message of
hers) and pattern-swap, which aren't published themselves; and a public
room, Open Kitchen, outside the space. The release client in a private KWin
session (as qa_kwin.py needs) must:
- open Explore from the compass under the server rail's +, listing the
  space and the kitchen;
- find the space by searching "weavers", which leaves the kitchen out, and
  list only spaces under Spaces;
- preview it (a space, its topic, its members) with Join; Back returns to
  the results;
- join it (Synapse lists the account) and open it, with both channels in its
  channel list though the account hasn't joined them, and a hint to pick one;
- preview loom-talk from that list and join it: it opens with Mira's
  message, and the space stays open around it.
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
from urllib.parse import quote

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import clean, read  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
# The compass, under the rail's + (view.rs): the rail's buttons are 48 px
# apart by 8, from 12 px under the 28 px title bar, after Direct Messages,
# All Rooms, a 2 px rule and the +.
COMPASS = (36, 242)
# Synapse lets only server admins publish to the room directory by default.
PUBLISH = """room_list_publication_rules:
  - action: allow
"""


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-explore-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp, PUBLISH)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=mira_hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def say(room, body):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=mira_hdr,
                 json={"msgtype": "m.text", "body": body})
        space = create(name="Weavers Guild", topic="Spinning, dyeing and weaving, together.", preset="public_chat",
                       visibility="public", room_alias_name="weavers", creation_content={"type": "m.space"})
        parent = [{"type": "m.space.parent", "state_key": space, "content": {"via": [SERVER_NAME], "canonical": True}}]
        loom = create(name="loom-talk", topic="Looms of every size.", preset="public_chat", initial_state=parent)
        swap = create(name="pattern-swap", topic="Share your drafts.", preset="public_chat", initial_state=parent)
        for child in (loom, swap):
            http("PUT", f"{path(space)}/state/m.space.child/{quote(child, safe='')}", headers=mira_hdr,
                 json={"via": [SERVER_NAME]})
        say(loom, "Warp threads go on first")
        kitchen = create(name="Open Kitchen", topic="Recipes and cooking chat.", preset="public_chat",
                         visibility="public", room_alias_name="kitchen")
        listed = http("POST", hs + "/_matrix/client/v3/publicRooms", headers=hdr, json={}).json()["chunk"]
        assert {space, kitchen} <= {r["room_id"] for r in listed}, listed

        def membership(room_id):
            try:
                r = http("GET", f"{path(room_id)}/state/m.room.member/{quote(me, safe='')}", headers=mira_hdr)
            except requests.HTTPError:
                return None  # never a member
            return r.json().get("membership")
        print(f"Synapse at {hs}: Mira's space {space} with {loom} and {swap}, and {kitchen}", flush=True)
        look(tmp, args.shots, membership, space, loom, swap)
        print("PASS: Explore finds a public space by search, previews it, joins it and shows its channels",
              flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, membership, space, loom, swap):
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
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect):
            """Cleaned words in `rect`; grey sidebar names too (a 4x pass)."""
            if rect == SIDEBAR:
                found = read(kde.capture().crop(WINDOW), sidebar=True, dialogs=False)
                x0, y0, x1, y1 = rect
                return [(clean(w), x, y) for w, x, y in found if x0 <= x < x1 and y0 <= y < y1]
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def has(rect, *needles):
            found = {w: (x, y) for w, x, y in words(rect)}
            return found if all(n in found for n in needles) else None

        def containing(rect, part):
            return next(((x, y) for w, x, y in words(rect) if part in w), None)

        def lacks(rect, part):
            return containing(rect, part) is None
        wait_for("media-proof in the sidebar", lambda: containing(SIDEBAR, "media-proof"), 90)

        # The compass opens Explore, listing the homeserver's public rooms.
        tap(kde, *COMPASS)
        try:
            wait_for("Explore", lambda: has(SIDEBAR, "discover") and has(MAIN, "community"), 15)
            wait_for("the space and the kitchen listed", lambda: containing(MAIN, "guild")
                     and containing(MAIN, "kitchen"), 30)
        finally:
            shot("explore")
        print("UI: the compass opened Explore, listing Weavers Guild and Open Kitchen", flush=True)

        # Searching (the search box has focus) finds the space alone. The
        # box then reads "weavers" too: the card is found by "guild".
        kde.type("weavers")
        try:
            wait_for("the search narrowed to the space", lambda: containing(MAIN, "guild")
                     and lacks(MAIN, "kitchen"), 20)
        finally:
            shot("search")
        # Spaces only, then everything again.
        tap(kde, *wait_for("Spaces in the sidebar", lambda: has(SIDEBAR, "spaces")["spaces"], 10))
        wait_for("Spaces lists the space", lambda: has(MAIN, "find", "space") and containing(MAIN, "guild"), 20)
        tap(kde, *wait_for("Everything in the sidebar", lambda: has(SIDEBAR, "everything")["everything"], 10))
        wait_for("everything again", lambda: has(MAIN, "community") and containing(MAIN, "guild"), 20)
        print("UI: searching \"weavers\" left only the space, also under Spaces", flush=True)

        # A card previews the space; Back returns to the results.
        card = wait_for("the space's card", lambda: containing(MAIN, "guild"), 10)
        tap(kde, *card)
        try:
            preview = wait_for("the space's preview", lambda: has(MAIN, "space", "spinning", "join", "back"), 20)
        finally:
            shot("preview")
        tap(kde, *preview["back"])
        card = wait_for("the results again", lambda: has(MAIN, "community") and containing(MAIN, "guild"), 15)
        tap(kde, *card)
        preview = wait_for("the space's preview again", lambda: has(MAIN, "space", "join", "back"), 20)
        print("UI: the space's card showed its preview with Join; Back went back to the results", flush=True)

        # Join: the space opens with both channels, which the account hasn't joined.
        tap(kde, *preview["join"])
        wait_for("Synapse lists the account in the space", lambda: membership(space) == "join", 20)
        try:
            wait_for("the space open with its channels", lambda: containing(SIDEBAR, "weavers")
                     and containing(SIDEBAR, "loom") and containing(SIDEBAR, "pattern")
                     and lacks(SIDEBAR, "discover") and has(MAIN, "pick", "channel"), 30)
        finally:
            shot("space-joined")
        assert membership(loom) is None and membership(swap) is None, "joined the channels with the space"
        print("UI: Join joined the space and opened it, listing loom-talk and pattern-swap", flush=True)

        # A channel from that list: its preview, then Join opens it in the space.
        tap(kde, *wait_for("loom-talk in the sidebar", lambda: containing(SIDEBAR, "loom"), 10))
        try:
            preview = wait_for("loom-talk's preview", lambda: has(MAIN, "looms", "join", "close"), 20)
        finally:
            shot("channel-preview")
        tap(kde, *preview["join"])
        wait_for("Synapse lists the account in loom-talk", lambda: membership(loom) == "join", 20)
        try:
            wait_for("loom-talk open with Mira's message", lambda: containing(MAIN, "warp")
                     and lacks(MAIN, "close"), 30)
            wait_for("still in the space, pattern-swap still offered", lambda: containing(SIDEBAR, "weavers")
                     and containing(SIDEBAR, "pattern"), 15)
        finally:
            shot("channel-joined")
        assert membership(swap) is None, "joined pattern-swap too"
        print("UI: loom-talk's preview joined it, and it opened in the space with Mira's message", flush=True)


if __name__ == "__main__":
    main()
