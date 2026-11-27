"""Remember where you were (ROADMAP N2), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_remember_place.py

The test account has a room with 150 numbered messages and two short rooms.
The release client in a private KWin session (as qa_kwin.py needs) must:
- after Page Up has scrolled the long room back into its history, and two
  other rooms have been visited, open the long room at the same message;
- quit and start again on the same profile, and open that room at that
  message once more.
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

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
FEED = (330, 80, 1180, 820)
PAGE_UP = ""
COUNT = 150


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-places-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)

        def room(name):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr,
                        json={"name": name, "preset": "private_chat"}).json()["room_id"]

        def say(room_id, body):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room_id, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json={"msgtype": "m.text", "body": body})
        history = room("history")
        for n in range(1, COUNT + 1):
            say(history, f"Entry {n:03} of the long history")
        say(room("second"), "The second room")
        say(room("third"), "The third room")
        print(f"Synapse at {hs}", flush=True)
        look(tmp, args.shots)
        print("PASS: a room left mid-history opens at the same message after visiting others and after a "
              "restart", flush=True)
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

        def place_window():
            kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)
        place_window()

        def shot(name):
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect):
            return [(w, x, y) for w, x, y in kde.words(rect)]

        def containing(rect, part):
            return next(((x, y) for w, x, y in words(rect) if part in w.lower()), None)

        def entries():
            """The numbered messages showing, top to bottom."""
            seen = [(y, int(m[1])) for w, _, y in words(FEED) if (m := re.fullmatch(r"(\d{3})", w.strip(".,:")))]
            return [n for _, n in sorted(seen)]

        def top():
            shown = entries()
            return shown[0] if len(shown) >= 3 else None

        def near(expected):
            return lambda: (t := top()) is not None and abs(t - expected) <= 2

        def open_room(name, check):
            tap(kde, *wait_for(name, lambda: containing(SIDEBAR, name), 30))
            wait_for(f"{name} open", check, 30)

        open_room("history", lambda: COUNT in entries())
        # Back into the history, past what the first screen loaded.
        for _ in range(5):
            kde.keys(("Down", PAGE_UP), ("Up", PAGE_UP))
            time.sleep(1.2)
        time.sleep(2)
        try:
            first = wait_for("a settled place in the history", top, 20)
        finally:
            shot("scrolled")
        assert first < COUNT - 40, f"Page Up only reached entry {first}"
        print(f"UI: Page Up scrolled the long room back to entry {first}", flush=True)

        open_room("second", lambda: containing(FEED, "second"))
        open_room("third", lambda: containing(FEED, "third"))
        open_room("history", lambda: entries())
        try:
            wait_for(f"history back at entry {first}", near(first), 30)
        finally:
            shot("returned")
        print(f"UI: after visiting two other rooms, the long room opened at entry {top()} again", flush=True)

        time.sleep(2)  # saved within a second
        kde.restart_native()
        place_window()
        try:
            wait_for(f"history at entry {first} after a restart", near(first), 60)
        finally:
            shot("restarted")
        print(f"UI: after a restart, the long room opened at entry {top()} again", flush=True)


if __name__ == "__main__":
    main()
