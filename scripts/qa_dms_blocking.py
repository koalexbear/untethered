"""DMs and blocking (ROADMAP J3), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_dms_blocking.py

Two more accounts, Mira and Milo, are in the homeserver's user directory;
Mira is in the test account's room and has said something there. The
release client in a private KWin session (as qa_kwin.py needs) must:
- from Direct Messages' +, find both by typing "mi" and start a group DM:
  an encrypted room that `m.direct` lists for both, as Element reads it;
- take Milo's full Matrix ID and start a DM with just him, listed the same;
- block Mira from her name's right-click menu, after a confirmation: the
  account's ignore list names her (Element hides her too), her message goes,
  what she says next and her invites never show, while the room carries on
  and loads its history again;
- list her under Settings' blocked people, where Unblock empties the list.
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

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
DIALOG = (330, 120, 850, 800)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-dms-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp, "user_directory:\n  search_all_users: true\n")
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))
        milo, _, _ = register(hs, "milo" + secrets.token_hex(3))
        room = fixture["room"]

        def path(room_id):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room_id, safe='')}"

        def say(headers, body):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers,
                 json={"msgtype": "m.text", "body": body})
        http("POST", f"{path(room)}/invite", headers=hdr, json={"user_id": mira})
        http("POST", f"{path(room)}/join", headers=mira_hdr, json={})
        say(mira_hdr, "Mira says hello")

        def account_data(kind):
            try:
                return http("GET", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/{kind}",
                            headers=hdr).json()
            except requests.HTTPError:
                return {}

        def encrypted(room_id):
            try:
                http("GET", f"{path(room_id)}/state/m.room.encryption/", headers=hdr)
                return True
            except requests.HTTPError:
                return False

        def invite_from_mira():
            http("POST", hs + "/_matrix/client/v3/createRoom", headers=mira_hdr,
                 json={"name": "trap-room", "invite": [me], "preset": "private_chat"})
        print(f"Synapse at {hs}: {me} with {mira} and {milo}", flush=True)
        look(tmp, args.shots, dict(me=me, mira=mira, milo=milo, account_data=account_data, encrypted=encrypted,
                                   say=lambda body: say(mira_hdr, body), mine=lambda body: say(hdr, body),
                                   invite_from_mira=invite_from_mira))
        print("PASS: New message starts encrypted DMs and group DMs every client lists as DMs; blocking hides "
              "someone here and on the account, and Settings unblocks them", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, f):
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    local = lambda user: user[1:].split(":", 1)[0]  # noqa: E731
    mira, milo = local(f["mira"]), local(f["milo"])
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

        def direct_rooms():
            return {user: set(rooms) for user, rooms in f["account_data"]("m.direct").items()}

        def new_message():
            # Direct Messages, then the + on its heading.
            tap(kde, 36, 64)
            heading = wait_for("Direct Messages' heading", lambda: has(SIDEBAR, "direct", "messages"), 20)
            tap(kde, 284, heading["messages"][1])
            try:
                return wait_for("New Message", lambda: has(WINDOW, "new", "message", "cancel"), 15)
            finally:
                shot("new-message")
        wait_for("media-proof", lambda: containing(SIDEBAR, "media-proof"), 90)

        # A group DM with the two people "mi" finds.
        new_message()
        kde.type("mi")
        try:
            wait_for("Mira and Milo found", lambda: containing(DIALOG, mira) and containing(DIALOG, milo), 20)
        finally:
            shot("people-found")
        tap(kde, *containing(DIALOG, mira))
        tap(kde, *containing(DIALOG, milo))
        try:
            create = wait_for("Create Group DM", lambda: has(WINDOW, "create", "group"), 10)
        finally:
            shot("people-picked")
        tap(kde, *create["group"])

        def group():
            rooms = direct_rooms()
            shared = rooms.get(f["mira"], set()) & rooms.get(f["milo"], set())
            return next(iter(shared), None)
        group_id = wait_for("m.direct listing the group DM for both", group, 30)
        assert f["encrypted"](group_id), "the group DM isn't encrypted"
        try:
            wait_for("the group DM open", lambda: not has(WINDOW, "cancel") and containing(MAIN, mira), 30)
        finally:
            shot("group-dm")
        print("UI: New message found Mira and Milo, and started an encrypted group DM m.direct lists for both",
              flush=True)

        # A DM with Milo alone, by his full Matrix ID.
        new_message()
        kde.type(f["milo"])

        def listed():
            # Below the search box, which says his ID too.
            hits = [(x, y) for w, x, y in kde.words(DIALOG) if milo in w.lower()]
            return max(hits, key=lambda p: p[1]) if len(hits) > 1 else None
        try:
            tap(kde, *wait_for("Milo's ID listed", listed, 20))
            wait_for("Milo picked", lambda: has(WINDOW, "add", "more"), 10)
        finally:
            shot("milo-picked")
        kde.keys(("Down", "\ue007"), ("Up", "\ue007"))  # Enter starts it

        def alone():
            rooms = direct_rooms()
            return next(iter(rooms.get(f["milo"], set()) - {group_id}), None)
        dm = wait_for("m.direct listing a DM with Milo alone", alone, 30)
        assert f["encrypted"](dm), "the DM isn't encrypted"
        try:
            wait_for("the DM open", lambda: not has(WINDOW, "cancel") and containing(MAIN, milo), 30)
        finally:
            shot("milo-dm")
        print("UI: Milo's full Matrix ID started an encrypted DM with him alone, listed in m.direct", flush=True)

        # Block Mira from her name in the room, after a confirmation.
        tap(kde, 36, 120)
        tap(kde, *wait_for("media-proof", lambda: containing(SIDEBAR, "media-proof"), 20))
        name = wait_for("Mira's message", lambda: has(MAIN, "says", "hello") and containing(MAIN, mira), 30)
        kde.actions("pointer", [{"type": "pointerMove", "x": name[0], "y": name[1]}, {"type": "pause", "duration": 250},
                                {"type": "pointerDown", "button": 2}, {"type": "pause", "duration": 80},
                                {"type": "pointerUp", "button": 2}])
        try:
            entry = wait_for("Block… in her menu", lambda: containing(WINDOW, "block"), 10)
        finally:
            shot("user-menu")
        tap(kde, *entry)
        try:
            wait_for("the confirmation", lambda: has(WINDOW, "cancel", "aren't") or has(WINDOW, "cancel", "told"), 10)
        finally:
            shot("block-confirm")
        # The dialog's red button is the lowest "Block".
        button = max(((x, y) for w, x, y in kde.words(WINDOW) if w.strip(".,:?!").lower() == "block"),
                     key=lambda p: p[1])
        tap(kde, *button)
        wait_for("the ignore list naming Mira", lambda: f["mira"] in f["account_data"]("m.ignored_user_list").get(
            "ignored_users", {}), 20)
        wait_for("her message gone", lambda: not has(MAIN, "says", "hello"), 30)
        f["say"]("Mira speaks again")
        f["invite_from_mira"]()
        f["mine"]("Still talking here")
        try:
            # Reloaded without her, the room fills its screen again.
            wait_for("the room carrying on", lambda: has(MAIN, "still", "talking", "welcome"), 30)
        finally:
            shot("blocked")
        time.sleep(3)
        assert not has(MAIN, "speaks", "again"), "a blocked person's new message showed"
        assert not containing(SIDEBAR, "trap"), "a blocked person's invite showed"
        print("UI: blocking Mira asked first, put her on the ignore list and hid her messages and invite, while "
              "the room carried on", flush=True)

        # Settings lists her; Unblock empties the list.
        tap(kde, 288, 871)
        try:
            row = wait_for("Settings' blocked people", lambda: has(WINDOW, "blocked", "unblock"), 20)
        finally:
            shot("settings-blocked")
        tap(kde, *row["unblock"])
        wait_for("the ignore list empty", lambda: not f["account_data"]("m.ignored_user_list").get(
            "ignored_users"), 20)
        try:
            wait_for("no one blocked", lambda: has(WINDOW, "haven't") and not has(WINDOW, "unblock"), 20)
        finally:
            shot("settings-unblocked")
        print("UI: Settings listed Mira under blocked people, and Unblock emptied the list", flush=True)


if __name__ == "__main__":
    main()
