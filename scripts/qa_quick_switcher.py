"""The quick switcher (ROADMAP N1), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_quick_switcher.py

The test account has a few more rooms, each with a message. The release
client in a private KWin session (as qa_kwin.py needs), driven only by the
keyboard, must:
- open the switcher with Ctrl+K while a draft is being typed in the
  composer, and leave the draft as it was;
- open a room from three letters and Enter;
- open the second match after the Down arrow;
- close with Escape, and with Ctrl+K again.
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

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
SWITCHER = (300, 110, 880, 700)
CONTROL, ENTER, ESCAPE, DOWN = "", "", "", ""


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-switcher-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        for name, body in [("design-reviews", "Design notes here"), ("gaming", "Game night soon"),
                           ("board-games", "Catan tonight")]:
            room = http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr,
                        json={"name": name, "preset": "private_chat"}).json()["room_id"]
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json={"msgtype": "m.text", "body": body})
        print(f"Synapse at {hs}", flush=True)
        look(tmp, args.shots)
        print("PASS: Ctrl+K, a few letters and Enter open a room without the mouse", flush=True)
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
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)

        def shot(name):
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect):
            return {w.strip(".,:|?!#").lower() for w, _, _ in kde.words(rect)}

        def has(rect, *needles):
            found = words(rect)
            return all(n in found for n in needles)

        def press(key):
            kde.keys(("Down", key), ("Up", key))

        def switcher():
            kde.keys(("Down", CONTROL), ("Down", "k"), ("Up", "k"), ("Up", CONTROL))

        def opened():
            return has(SWITCHER, "where", "would")

        def go(query, name, after=None):
            switcher()
            wait_for("the switcher", opened, 10)
            kde.type(query)
            try:
                wait_for(f"{name} listed", lambda: any(name in w for w in words(SWITCHER)), 10)
            finally:
                shot(f"switcher-{query}")
            if after:
                press(after)
            press(ENTER)

        # A draft in the composer, then the keyboard only.
        wait_for("media-proof", lambda: any("media-proof" in w for w in words(SIDEBAR)), 90)
        tap(kde, 700, 847)
        kde.type("draft words")
        wait_for("the draft", lambda: has(MAIN, "draft", "words"), 10)

        go("des", "design-reviews")
        try:
            wait_for("design-reviews open", lambda: not opened() and has(MAIN, "design", "notes"), 20)
        finally:
            shot("design-open")
        print("UI: Ctrl+K while typing, \"des\" and Enter opened design-reviews", flush=True)

        go("ga", "board-games", after=DOWN)
        try:
            wait_for("board-games open", lambda: not opened() and has(MAIN, "catan", "tonight"), 20)
        finally:
            shot("down-open")
        print("UI: Down then Enter opened the second match, board-games", flush=True)

        go("med", "media-proof")
        try:
            # The draft is as it was: no stray k from Ctrl+K.
            wait_for("media-proof's draft intact", lambda: not opened() and has(MAIN, "draft", "words"), 20)
        finally:
            shot("draft-intact")
        print("UI: back in media-proof, the draft typed before Ctrl+K was untouched", flush=True)

        switcher()
        wait_for("the switcher", opened, 10)
        press(ESCAPE)
        wait_for("Escape closing it", lambda: not opened(), 10)
        switcher()
        wait_for("the switcher", opened, 10)
        switcher()
        try:
            wait_for("Ctrl+K closing it", lambda: not opened(), 10)
        finally:
            shot("closed")
        print("UI: Escape and Ctrl+K each closed the switcher", flush=True)


if __name__ == "__main__":
    main()
