"""Search across a space (ROADMAP N7), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_space_search.py

The test account's space, Workshop, has two unencrypted channels and an
encrypted one. Mira posts a file in old-archive and 80 messages after it;
Enzo, on a gomuks device, posts an encrypted file in vault and 40 encrypted
messages after it, so neither file is among what the client syncs. The
release client in a private KWin session (as qa_kwin.py needs) must:
- with old-archive never opened, find Mira's file over the space with
  `from:mira has:file` (the homeserver's filtered history) and with a word
  (its full-text search), say it searched the homeserver, and open the hit
  in old-archive;
- in vault, find nothing for `from:enzo has:file` at first, say how far back
  this device searched, then find the file after Load older history.
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
from qa_matrix_sdk import SERVER_NAME, Gomuks, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
ENTER = ""


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-search-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def say(headers, room, content):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers, json=content)
        space = create(name="Workshop", preset="private_chat", creation_content={"type": "m.space"})
        general = create(name="general", preset="private_chat")
        archive = create(name="old-archive", preset="private_chat", invite=[mira])
        vault = create(name="vault", preset="private_chat", invite=[enzo], initial_state=[
            {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}])
        for child in (general, archive, vault):
            http("PUT", f"{path(space)}/state/m.space.child/{quote(child, safe='')}", headers=hdr,
                 json={"via": [SERVER_NAME]})
        http("POST", f"{path(archive)}/join", headers=mira_hdr, json={})
        http("POST", f"{path(vault)}/join", headers=enzo_hdr, json={})
        say(hdr, general, {"msgtype": "m.text", "body": "Welcome to the workshop"})

        upload = http("POST", hs + "/_matrix/media/v3/upload?filename=quarterly-plan.pdf", headers={
            **mira_hdr, "Content-Type": "application/pdf"}, data=b"%PDF-1.4 plan").json()["content_uri"]
        say(mira_hdr, archive, {"msgtype": "m.file", "body": "quarterly-plan.pdf", "url": upload,
                                "info": {"mimetype": "application/pdf", "size": 13}})
        for n in range(80):
            say(mira_hdr, archive, {"msgtype": "m.text", "body": f"archive chatter {n}"})

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        peer.joined(vault)
        descriptor = peer.upload(b"%PDF-1.4 secret", "secret-plan.pdf", encrypt=True)
        peer.rpc("send_message", {"room_id": vault, "base_content": descriptor, "text": ""})
        peer.find("the encrypted file", lambda e: (e.get("decrypted") or {}).get("body") == "secret-plan.pdf"
                  and str(e.get("event_id", "")).startswith("$"))
        for n in range(40):
            body = f"vault chatter {n}"
            peer.rpc("send_message", {"room_id": vault, "text": body})
            peer.find(body, lambda e, body=body: (e.get("decrypted") or {}).get("body") == body
                      and str(e.get("event_id", "")).startswith("$"))
        time.sleep(2)
        print(f"Synapse at {hs}: {me}'s Workshop with {mira} and {enzo}", flush=True)
        look(tmp, args.shots)
        print("PASS: search across a space finds an old file through the homeserver in a channel never "
              "opened here, and in an encrypted channel once its history is loaded", flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots):
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
            return {w.strip(".,:|?!#“”").lower() for w, _, _ in kde.words(rect)}

        def has(rect, *needles):
            found = words(rect)
            return all(n in found for n in needles)

        def containing(rect, part):
            return next(((x, y) for w, x, y in kde.words(rect) if part in w.lower()), None)

        def search(query, check, name):
            if has(MAIN, "return", "timeline"):
                tap(kde, 700, 152)  # the open search's field
            else:
                tap(kde, 1080, 51)  # the header's Search
            time.sleep(0.8)
            kde.select_all()
            kde.type(query)
            kde.keys(("Down", ENTER), ("Up", ENTER))
            try:
                return wait_for(name, check, 60)
            finally:
                shot(name.replace(" ", "-"))

        wait_for("media-proof", lambda: containing(SIDEBAR, "media-proof"), 90)
        tap(kde, 36, 186)  # the Workshop space, which opens general
        kde.actions("pointer", [{"type": "pointerMove", "x": 700, "y": 600}])  # no tooltip over the list
        try:
            wait_for("general open in Workshop", lambda: has(SIDEBAR, "old-archive", "vault")
                     and has(MAIN, "welcome", "workshop"), 30)
        finally:
            shot("space")

        search("from:mira has:file",
               lambda: containing(MAIN, "quarterly") and has(MAIN, "homeserver"), "mira's file")
        print("UI: from:mira has:file found the file in old-archive, never opened here, through the "
              "homeserver", flush=True)
        search("quarterly", lambda: containing(MAIN, "quarterly-plan") and has(MAIN, "match", "quarterly")
               and not containing(MAIN, "from:mira"), "words found")
        tap(kde, *containing(MAIN, "quarterly-plan"))
        try:
            wait_for("old-archive open at the file",
                     lambda: containing(MAIN, "quarterly-plan") and not has(MAIN, "homeserver"), 30)
        finally:
            shot("hit-opened")
        print("UI: a word search found it too, and the hit opened old-archive at the file", flush=True)

        # Search opens from the timeline, not from a message's context.
        tap(kde, *wait_for("Back to latest", lambda: containing(MAIN, "latest"), 10))
        wait_for("the timeline", lambda: not has(MAIN, "viewing", "older"), 20)
        search("from:enzo has:file", lambda: containing(MAIN, "device") and containing(MAIN, "older")
               and containing(MAIN, "matches") and not containing(MAIN, "secret"), "nothing yet")
        tap(kde, *containing(MAIN, "older"))
        try:
            wait_for("Enzo's encrypted file", lambda: containing(MAIN, "secret-plan"), 120)
        finally:
            shot("encrypted-found")
        print("UI: in vault, nothing until Load older history; then from:enzo has:file found the encrypted "
              "file", flush=True)


if __name__ == "__main__":
    main()
