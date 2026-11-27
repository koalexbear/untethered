"""Forward (ROADMAP M8), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_forward.py

The test account shares an encrypted room, vault, with Enzo on a gomuks
device, who posts an encrypted picture and a message there; it also has an
encrypted DM with Enzo and an unencrypted channel, general. The release
client in a private KWin session (as qa_kwin.py needs) must:
- forward the picture from its right-click menu to the DM and general at
  once, with a message of its own, warning that general isn't encrypted;
- forward the message from its hover toolbar to the DM.
Each forward must reach Enzo's device (decrypted in the DM, as ordinary
messages from the test account) and general, reuse the picture's file and
key with no new upload, and notify nobody (empty `m.mentions`).
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import io
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import Gomuks, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
MAGENTA = (225, 30, 190)
ENTER = ""
TEXT = "Meet at the workshop at noon"
NOTE = "for the record"


def near(pixel, colour, slack=50):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-forward-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))
        encryption = [{"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"
        vault = create(name="vault", preset="private_chat", invite=[enzo], initial_state=encryption)
        dm = create(preset="trusted_private_chat", is_direct=True, invite=[enzo], initial_state=encryption)
        general = create(name="general", preset="private_chat")
        http("PUT", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/m.direct", headers=hdr,
             json={enzo: [dm]})
        for room in (vault, dm):
            http("POST", f"{path(room)}/join", headers=enzo_hdr, json={})

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        peer.joined(vault)
        peer.joined(dm)
        picture = io.BytesIO()
        Image.new("RGB", (360, 240), MAGENTA).save(picture, "PNG")
        descriptor = peer.upload(picture.getvalue(), "plan.png", encrypt=True)
        peer.rpc("send_message", {"room_id": vault, "base_content": descriptor, "text": ""})
        original = peer.find("the encrypted picture", lambda e: (e.get("decrypted") or {}).get("body") == "plan.png"
                             and str(e.get("event_id", "")).startswith("$"))["decrypted"]
        assert "file" in original and "url" not in original, original
        peer.rpc("send_message", {"room_id": vault, "text": TEXT})
        peer.find(TEXT, lambda e: (e.get("decrypted") or {}).get("body") == TEXT
                  and str(e.get("event_id", "")).startswith("$"))
        time.sleep(2)

        uploads = tmp / "media_store/local_content"

        def stored():
            return sum(1 for f in uploads.rglob("*") if f.is_file())

        def forwarded_in_dm(msgtype, body):
            """Enzo's device decrypting what the test account forwarded to the DM."""
            return peer.find(f"{body} forwarded to the DM", lambda e: e.get("room_id") == dm
                             and e.get("sender") == me and e.get("type") == "m.room.encrypted"
                             and (e.get("decrypted") or {}).get("msgtype") == msgtype
                             and (e.get("decrypted") or {}).get("body") == body, timeout=90)["decrypted"]

        def in_general(msgtype, body):
            events = http("GET", f"{path(general)}/messages?dir=b&limit=20", headers=hdr).json()["chunk"]
            return next((e["content"] for e in events if e["sender"] == me and e["type"] == "m.room.message"
                         and e["content"].get("msgtype") == msgtype and e["content"].get("body") == body), None)

        print(f"Synapse at {hs}: {me} with {enzo} on gomuks", flush=True)
        look(tmp, args.shots, stored, original, forwarded_in_dm, in_general)
        print("PASS: Forward sends a picture and a message again to a DM and a channel, from the menu and the "
              "toolbar, reusing the encrypted file with no new upload", flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, stored, original, forwarded_in_dm, in_general):
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

        def hover(pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}])

        def picture():
            im = kde.capture().crop(WINDOW).convert("RGB")
            px = im.load()
            pts = [(x, y) for y in range(MAIN[1], MAIN[3], 6) for x in range(MAIN[0], MAIN[2], 6)
                   if near(px[x, y], MAGENTA)]
            if len(pts) < 20:
                return None
            return sum(p[0] for p in pts) // len(pts), sum(p[1] for p in pts) // len(pts)

        def press(button, pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}, {"type": "pause", "duration": 150},
                                    {"type": "pointerDown", "button": button}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": button}])

        def dialog():
            """The Forward To dialog's title, once it shows."""
            ws = words(WINDOW)
            return next(((x, y) for w, x, y in ws if w == "forward" and any(
                v == "to" and abs(vy - y) < 8 and 0 < vx - x < 120 for v, vx, vy in ws)), None)

        def pick(letters):
            kde.type(letters)
            time.sleep(0.4)
            kde.keys(("Down", ENTER), ("Up", ENTER))
            time.sleep(0.6)

        tap(kde, *wait_for("vault in the sidebar", lambda: word("vault", SIDEBAR), 90))
        wait_for("Enzo's message", lambda: word("noon"), 60)
        at = wait_for("Enzo's picture", picture, 60)
        before = stored()

        # From the right-click menu: the picture, to the DM and general, with a message.
        for attempt in range(3):
            press(2, at)
            try:
                entry = wait_for("Forward in the menu", lambda: word("forward", WINDOW), 6)
                break
            except RuntimeError:
                if attempt == 2:
                    raise
        tap(kde, *entry)
        wait_for("Forward To", dialog, 10)
        pick("enz")
        pick("gen")
        try:
            wait_for("the warning about general", lambda: word("encrypted", WINDOW) and word("server", WINDOW), 10)
        finally:
            shot("forward-dialog")
        # OCR can miss the grey placeholder; the field is under the preview.
        tap(kde, *wait_for("the optional message", lambda: word("optional", WINDOW) or (
            (p := word("plan.png", (330, 400, 860, 760))) and (p[0], p[1] + 43)), 10))
        kde.type(NOTE)
        kde.keys(("Down", ENTER), ("Up", ENTER))
        wait_for("the dialog closed", lambda: not dialog(), 60)
        print("UI: the picture's menu forwarded it to the DM and general, with a message, after warning that "
              "general isn't encrypted", flush=True)

        sent = forwarded_in_dm("m.image", "plan.png")
        assert sent["file"]["url"] == original["file"]["url"], (sent, original)
        assert sent["file"]["key"]["k"] == original["file"]["key"]["k"], "the forward carries the file's key"
        assert sent.get("m.mentions") == {}, sent
        assert "m.relates_to" not in sent, sent
        note = forwarded_in_dm("m.text", NOTE)
        open_copy = wait_for("the picture in general", lambda: in_general("m.image", "plan.png"), 30)
        assert open_copy["file"]["url"] == original["file"]["url"], open_copy
        wait_for("the message in general", lambda: in_general("m.text", NOTE), 30)
        assert stored() == before, f"{stored() - before} new uploads"
        print(f"Matrix: Enzo's device decrypted the forwarded picture and \"{note['body']}\" in the DM, general has "
              f"both; the same encrypted file and key, with no new upload", flush=True)

        # From the hover toolbar: the message, to the DM.
        row = wait_for("Enzo's message", lambda: word("noon"), 10)
        toolbar = Ui(lambda: words(WINDOW), hover, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
        button = wait_for("Forward in the toolbar", lambda: toolbar.message(row, "forward"), 20)
        tap(kde, *button)
        wait_for("Forward To", dialog, 10)
        pick("enz")
        kde.keys(("Down", ENTER), ("Up", ENTER))  # Enter with nothing typed sends
        wait_for("the dialog closed", lambda: not dialog(), 60)
        text = forwarded_in_dm("m.text", TEXT)
        assert text.get("m.mentions") == {}, text
        shot("forwarded")
        print("UI: the message's toolbar forwarded it to the DM, which Enzo's device decrypted", flush=True)


if __name__ == "__main__":
    main()
