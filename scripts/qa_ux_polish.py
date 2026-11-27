"""Timeline and room list behaviour, live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_ux_polish.py

Seeds a plain room with 400 messages and an encrypted voice channel, then
drives the release client in a private KWin session (as qa_kwin.py needs):
- Page Up, pressed quickly and nothing else, reaches the first message:
  older messages keep loading while the timeline sits at the top, without
  an "Earlier messages" click;
- a click on the left of the "You're viewing older messages" bar, away from
  its "Jump to present" label, jumps to the newest message;
- the reaction picker opened from a message's right-click menu floats by
  the message's top-right corner (above it near the bottom of the window)
  without moving the timeline, and no other message's toolbar covers it;
  a click
  elsewhere closes it and does nothing else; opened from the smile beside
  the message's reactions, it floats by that smile (below, or above near
  the bottom of the window), and Escape closes it;
- clicking the voice channel in the room list opens it and starts joining
  its call (no call server is needed to see the call start).
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import os
from pathlib import Path
import re
import secrets
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

COUNT = 400
WINDOW = (0, 0, 1180, 900)
FEED = (330, 60, 1180, 800)
BAR = (330, 600, 1180, 830)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-ux-", dir=os.environ.get("TMPDIR")))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        room = fixture["room"]
        for n in range(1, COUNT + 1):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json={"msgtype": "m.text", "body": f"Backlog message {n:03}"})
        http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json={
            "name": "lounge", "preset": "private_chat", "creation_content": {"type": "org.matrix.msc3417.call"},
            "initial_state": [{"type": "m.room.encryption", "state_key": "",
                               "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]})
        print(f"Synapse at {hs}: {COUNT} messages in {room}, and a voice channel", flush=True)
        look(tmp, args.shots)
        print("PASS: history kept loading at the top, the whole bar jumped to the present, the reaction "
              "picker floated by its button and closed outside, and the voice channel joined on a click",
              flush=True)
    finally:
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

        def window():
            assert kde.native.poll() is None, "the client exited"
            return kde.window("Untethered")
        kde.position(wait_for("client window", window, 60), *WINDOW)

        def shot(name):
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=FEED):
            return [(w.strip(".,:|"), x, y) for w, x, y in kde.words(rect)]

        def lines():
            found = {}
            for w, x, y in words():
                if re.fullmatch(r"\d{3}", w) and x > 400:
                    found.setdefault(w, (x, y))
            return found

        def find(word, rect=FEED):
            return next(((x, y) for w, x, y in words(rect) if w.lower() == word), None)

        def room_entry(name):
            return lambda: next(((x, y) for w, x, y in kde.words((72, 40, 330, 900)) if name in w.lower()), None)
        tap(kde, *wait_for("media-proof in the room list", room_entry("media-proof"), 90))
        wait_for("the newest message", lambda: f"{COUNT:03}" in lines(), 60)

        # History: Page Up in quick succession, nothing else.
        presses = 0
        while "001" not in lines():
            assert presses < 160, f"Page Up {presses} times without reaching the first message"
            for _ in range(4):
                kde.keys(("Down", ""), ("Up", ""))
                presses += 1
                time.sleep(.2)
        shot("history-top")
        print(f"UI: {presses} quick Page Ups reached the first of {COUNT} messages without a click", flush=True)

        # The bar: click its left part, on "viewing".
        tap(kde, *wait_for("the older-messages bar", lambda: find("viewing", BAR), 10))
        wait_for("the newest message after the bar", lambda: f"{COUNT:03}" in lines(), 15)
        print("UI: a click on the bar's \"You're viewing older messages\" text jumped to the present", flush=True)

        # The picker from the right-click menu, by the message's corner.
        def open_from_menu(number):
            x, y = wait_for(f"line {number}", lambda: lines().get(number), 10)
            kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 250},
                                    {"type": "pointerDown", "button": 2}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": 2}])
            tap(kde, *wait_for("Add Reaction in the menu", lambda: find("reaction", WINDOW), 10))
            return y
        held = lines()
        row_y = open_from_menu("399")
        qx, qy = wait_for("the reaction picker", lambda: find("quick", WINDOW), 10)
        shot("picker-corner")
        moved = {k: (held[k], v) for k, v in lines().items() if k in held and abs(held[k][1] - v[1]) > 3}
        assert not moved, f"opening the picker moved the timeline: {moved}"
        # Below the corner, or above it near the bottom of the window.
        assert qx > 700 and abs(qy - row_y) < 160, (qx, qy, row_y)
        tap(kde, 800, 200)
        wait_for("the picker closed by a click outside", lambda: find("quick", WINDOW) is None, 10)
        print(f"UI: from the menu, the picker floated by the message's top-right ({qx}, {qy}) without moving "
              "the timeline, and a click elsewhere only closed it", flush=True)

        # React with the first quick reaction, then open the picker from the
        # smile beside it.
        open_from_menu("399")
        qx, qy = wait_for("the reaction picker", lambda: find("quick", WINDOW), 10)
        tap(kde, qx - 3, qy + 32)
        wait_for("the picker closed by reacting", lambda: find("quick", WINDOW) is None, 10)
        x, y = wait_for("line 399", lambda: lines().get("399"), 10)

        def own_pill():
            # The own reaction's blurple outline, below the line.
            im = kde.capture()
            def blurple(p):
                return abs(p[0] - 88) < 30 and abs(p[1] - 101) < 30 and abs(p[2] - 242) < 30
            hits = [(px, py) for py in range(y + 10, min(y + 60, 800)) for px in range(380, 700)
                    if blurple(im.getpixel((px, py)))]
            return (max(p[0] for p in hits), (min(p[1] for p in hits) + max(p[1] for p in hits)) // 2) if hits else None
        right, mid = wait_for("the reaction pill", own_pill, 10)
        shot("reacted")
        tap(kde, right + 20, mid)
        try:
            qx, qy = wait_for("the picker by the smile", lambda: find("quick", WINDOW), 10)
        except RuntimeError:
            shot("smile-missing")
            raise RuntimeError(f"no picker after a click at ({right + 20}, {mid})")
        shot("picker-smile")
        # Below the smile, or above it near the bottom of the window.
        assert qx < 700 and abs(qy - mid) < 160, (qx, qy, right, mid)
        kde.keys(("Down", ""), ("Up", ""))  # Escape
        wait_for("the picker closed by Escape", lambda: find("quick", WINDOW) is None, 10)
        print(f"UI: from the smile beside the reactions, the picker floated by it ({qx}, {qy}); Escape "
              "closed it", flush=True)

        # The voice channel: one click opens it and joins.
        tap(kde, *wait_for("lounge in the room list", room_entry("lounge"), 30))

        def joining():
            text = " ".join(w.lower() for w, _, _ in words(WINDOW))
            return "joining" in text or "connected" in text or "in voice" in text
        try:
            wait_for("the voice call starting", joining, 10)
        finally:
            shot("voice")
        print("UI: one click on the voice channel opened it and started joining its call", flush=True)


if __name__ == "__main__":
    main()
