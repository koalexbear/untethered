"""Joining by link (ROADMAP J2), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_join_links.py

A second account, Mira, makes a public room with an address and a message,
an invite-only room and a public space. In a room of its own, the test
account posts `matrix.to` links to the two rooms. The release client in a
private KWin session (as qa_kwin.py needs) must:
- open the public room's link as a preview (name, topic, members) with Join;
  Join joins it (Synapse lists the account) and opens it with Mira's message;
- for the invite-only room, about which the homeserver tells outsiders
  nothing, say it couldn't be previewed; trying to join it fails, and the
  card says so;
- take the space's address in Join a Server (the rail's +), preview it, and
  join and open the space.
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

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-join-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))

        def create(headers, **body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=headers, json=body).json()["room_id"]

        def say(headers, room, body):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=headers, json={"msgtype": "m.text", "body": body})
        lounge = create(mira_hdr, name="open-lounge", topic="Anyone can come in.", preset="public_chat",
                        room_alias_name="open-lounge")
        say(mira_hdr, lounge, "Hello from the lounge")
        closed = create(mira_hdr, name="closed-room", preset="private_chat", room_alias_name="closed-room")
        space = create(mira_hdr, name="Open Space", preset="public_chat", room_alias_name="open-space",
                       creation_content={"type": "m.space"})
        say(hdr, fixture["room"], f"Come to https://matrix.to/#/%23open-lounge:{SERVER_NAME}")
        say(hdr, fixture["room"], f"Not this one https://matrix.to/#/%23closed-room:{SERVER_NAME}")

        def membership(room_id):
            try:
                r = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(room_id, safe='')}/state/m.room.member/"
                         f"{quote(me, safe='')}", headers=mira_hdr)
            except requests.HTTPError:
                return None  # never a member
            return r.json().get("membership")
        print(f"Synapse at {hs}: Mira's rooms {lounge} and {space}", flush=True)
        look(tmp, args.shots, membership, lounge, closed, space)
        print("PASS: Matrix links and addresses preview rooms in the app, and Join joins and opens them",
              flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, membership, lounge, closed, space):
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
            return {w.strip(".,:|?!").lower(): (x, y) for w, x, y in kde.words(rect)}

        def has(rect, *needles):
            found = words(rect)
            return found if all(n in found for n in needles) else None

        def containing(rect, part):
            return next(((x, y) for w, x, y in kde.words(rect) if part in w.lower()), None)
        tap(kde, *wait_for("media-proof", lambda: containing(SIDEBAR, "media-proof"), 90))

        # The public room's link: a preview, then Join.
        try:
            link = wait_for("the lounge link", lambda: containing(MAIN, "lounge"), 30)
        finally:
            shot("links")
        tap(kde, *link)
        try:
            card = wait_for("the lounge's preview", lambda: has(MAIN, "anyone", "join", "close"), 20)
        finally:
            shot("lounge-preview")
        tap(kde, *card["join"])
        wait_for("Synapse lists the account in the lounge", lambda: membership(lounge) == "join", 20)
        wait_for("the lounge open with Mira's message", lambda: has(MAIN, "hello", "lounge") and not has(MAIN, "close"), 30)
        shot("lounge-joined")
        print("UI: the lounge's link showed its preview; Join joined it and opened it with Mira's message", flush=True)

        # The invite-only room's link: its join refused.
        tap(kde, *wait_for("media-proof", lambda: containing(SIDEBAR, "media-proof"), 10))
        tap(kde, *wait_for("the closed room's link", lambda: containing(MAIN, "closed"), 30))
        # The homeserver tells outsiders nothing about an invite-only room.
        try:
            card = wait_for("the closed room's card", lambda: has(MAIN, "private", "try", "close"), 20)
        finally:
            shot("closed-preview")
        tap(kde, *card["try"])
        try:
            card = wait_for("the card saying the join failed", lambda: has(MAIN, "invite-only", "close"), 20)
        finally:
            shot("closed-refused")
        assert membership(closed) != "join", "joined an invite-only room"
        tap(kde, *card["close"])
        print("UI: the invite-only room's link said it couldn't be previewed, and the card said joining it "
              "failed", flush=True)

        # Join a Server with the space's address.
        tap(kde, 36, 186)
        try:
            heading = wait_for("Join a Server in the create window", lambda: has(WINDOW, "already", "address"), 15)
        finally:
            shot("create-window")
        hx, hy = heading["already"]
        tap(kde, hx, hy + 38)
        kde.type(f"#open-space:{SERVER_NAME}")
        kde.keys(("Down", ""), ("Up", ""))  # Enter
        try:
            card = wait_for("the space's preview", lambda: has(MAIN, "space", "join", "close"), 20)
        finally:
            shot("space-preview")
        tap(kde, *card["join"])
        wait_for("Synapse lists the account in the space", lambda: membership(space) == "join", 20)
        try:
            wait_for("the space open", lambda: has(SIDEBAR, "open", "space") and not has(MAIN, "close"), 30)
        finally:
            shot("space-joined")
        assert not has(MAIN, "invite-only"), "the closed room's failure outlived its card"
        print("UI: Join a Server took the space's address, previewed it, and joined and opened the space", flush=True)


if __name__ == "__main__":
    main()
