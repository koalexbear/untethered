"""Invites (ROADMAP J1), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_invites.py

A second account, Mira, invites the test account to a room, a DM and a space.
The release client in a private KWin session (as qa_kwin.py needs) must:
- list the room under Invites in All Rooms and the DM at the top of Direct
  Messages, each with who sent it, and the space in the server rail;
- open each invite's card; Accept on the room joins it (Synapse lists the
  account as joined) and opens it; Decline on the DM leaves it (Synapse
  records the leave) and removes it; Accept on the space joins it and opens
  the space.
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
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-invites-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))
        http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(mira, safe='')}/displayname", headers=mira_hdr,
             json={"displayname": "Mira"})

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=mira_hdr, json=body).json()["room_id"]
        room = create(name="design-review", topic="Sketches before Friday.", preset="private_chat", invite=[me])
        dm = create(preset="trusted_private_chat", is_direct=True, invite=[me])
        space = create(name="Mira Workshop", preset="private_chat", invite=[me],
                       creation_content={"type": "m.space"})

        def membership(room_id):
            r = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(room_id, safe='')}/state/m.room.member/"
                     f"{quote(me, safe='')}", headers=mira_hdr)
            return r.json().get("membership")
        print(f"Synapse at {hs}: Mira invited {me} to {room}, {dm} and {space}", flush=True)
        look(tmp, args.shots, membership, room, dm, space)
        print("PASS: invites show with who sent them; accepting joins and opens, declining leaves", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, membership, room, dm, space):
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
            return {w.strip(".,:|").lower(): (x, y) for w, x, y in kde.words(rect)}

        def has(rect, *needles):
            found = words(rect)
            return found if all(n in found for n in needles) else None

        # All Rooms: the room's invite, with who sent it.
        tap(kde, 36, 120)
        try:
            listed = wait_for("the room's invite in All Rooms", lambda: has(SIDEBAR, "invites", "design-review", "mira"), 90)
        finally:
            shot("all-rooms-invites")
        tap(kde, *listed["design-review"])
        card = wait_for("the room's invite card", lambda: has(MAIN, "invited", "design-review", "accept", "decline"), 15)
        shot("room-card")
        tap(kde, *card["accept"])
        wait_for("Synapse lists the account as joined", lambda: membership(room) == "join", 20)
        wait_for("the joined room open, its invite gone",
                 lambda: "invites" not in words(SIDEBAR) and has(MAIN, "design-review") and not has(MAIN, "accept"), 30)
        shot("room-joined")
        print("UI: All Rooms listed design-review under Invites from Mira; Accept joined and opened it", flush=True)

        # Direct Messages: the DM's invite, declined.
        tap(kde, 36, 64)
        listed = wait_for("the DM's invite in Direct Messages", lambda: has(SIDEBAR, "invites", "mira"), 30)
        tap(kde, *listed["mira"])
        card = wait_for("the DM's invite card", lambda: has(MAIN, "wants", "message", "decline"), 15)
        shot("dm-card")
        tap(kde, *card["decline"])
        wait_for("Synapse records the leave", lambda: membership(dm) == "leave", 20)
        wait_for("the DM's invite gone", lambda: "invites" not in words(SIDEBAR), 30)
        print("UI: Direct Messages listed Mira's invite; Decline left the DM and removed it", flush=True)

        # The space: a rail entry below the divider, then its card.
        time.sleep(1)
        tap(kde, 36, 186)
        card = wait_for("the space's invite card", lambda: has(MAIN, "space", "workshop", "accept"), 15)
        shot("space-card")
        tap(kde, *card["accept"])
        wait_for("Synapse lists the account in the space", lambda: membership(space) == "join", 20)
        try:
            wait_for("the space open", lambda: has(SIDEBAR, "workshop") and not has(MAIN, "accept"), 30)
        finally:
            shot("space-joined")
        print("UI: the rail showed the space's invite; Accept joined it and opened the space", flush=True)


if __name__ == "__main__":
    main()
