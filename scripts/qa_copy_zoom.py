"""Copy Image and zoom in the media viewer (part of ROADMAP P3), live against a
disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_copy_zoom.py

The test account has a 1200×800 picture, magenta on the left and teal on the
right, in its room. The release client in a private KWin session (as
qa_kwin.py needs) must:
- copy the picture from its right-click menu: wl-paste reads a 1200×800 PNG
  with those colours (the original, not the preview);
- in the media viewer, copy it again with the Copy button, and say so;
- zoom in on a click at that point (twice the size: the colour edge moves
  away from the click by as much as the click was from it) and out on the
  next click, and zoom in and out a step with the mouse wheel.
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
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
MAIN = (330, 40, 1180, 900)
MAGENTA, TEAL = (225, 30, 190), (30, 200, 180)
W, H = 1200, 800


def near(pixel, colour):
    return all(abs(a - b) < 40 for a, b in zip(pixel[:3], colour))


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-copy-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        picture = Image.new("RGB", (W, H), MAGENTA)
        picture.paste(TEAL, (W // 2, 0, W, H))
        buf = io.BytesIO()
        picture.save(buf, "PNG")
        mxc = http("POST", hs + "/_matrix/media/v3/upload?filename=split.png",
                   headers={**hdr, "Content-Type": "image/png"}, data=buf.getvalue()).json()["content_uri"]
        http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(fixture['room'], safe='')}/send/m.room.message/"
             f"{secrets.token_hex(8)}", headers=hdr, json={
                 "msgtype": "m.image", "body": "split.png", "url": mxc,
                 "info": {"w": W, "h": H, "mimetype": "image/png", "size": len(buf.getvalue())}})
        print(f"Synapse at {hs}", flush=True)
        look(tmp, args.shots)
        print("PASS: Copy Image puts the original on the clipboard from the menu and the viewer, and the viewer "
              "zooms with a click and the mouse wheel", flush=True)
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
            return {w.strip(".,:|?!·").lower() for w, _, _ in kde.words(rect)}

        def containing(rect, part):
            return next(((x, y) for w, x, y in kde.words(rect) if part in w.lower()), None)

        def magenta_box():
            im = kde.capture().crop(WINDOW).convert("RGB")
            px = im.load()
            xs, ys = [], []
            for y in range(40, 900, 4):
                for x in range(330, 1180, 4):
                    if near(px[x, y], MAGENTA):
                        xs.append(x)
                        ys.append(y)
            return (min(xs), min(ys), max(xs), max(ys)) if len(xs) > 200 else None

        def edge(y):
            """Where magenta turns teal along row `y`."""
            im = kde.capture().crop(WINDOW).convert("RGB")
            px = im.load()
            last = None
            for x in range(0, 1180):
                if near(px[x, y], MAGENTA):
                    last = x
                elif near(px[x, y], TEAL) and last is not None and x - last <= 4:
                    return last
            return None

        def clipboard():
            """The PNG on the clipboard, as another app gets it."""
            out = subprocess.run(["wl-paste", "--no-newline", "--type", "image/png"], env=kde.env,
                                 capture_output=True, timeout=10)
            if out.returncode != 0 or not out.stdout:
                return None
            got = Image.open(io.BytesIO(out.stdout)).convert("RGB")
            return got if got.size == (W, H) else None

        def original(got):
            return got and near(got.getpixel((100, 400)), MAGENTA) and near(got.getpixel((1100, 400)), TEAL)

        def forget_clipboard():
            subprocess.run(["wl-copy", "--clear"], env=kde.env, timeout=10)

        # From the chat: right-click the picture, Copy Image.
        box = wait_for("the picture in the chat", magenta_box, 90)
        x, y = (box[0] + box[2]) // 2, (box[1] + box[3]) // 2
        kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 250},
                                {"type": "pointerDown", "button": 2}, {"type": "pause", "duration": 80},
                                {"type": "pointerUp", "button": 2}])
        try:
            entry = wait_for("Copy Image in the menu", lambda: containing(WINDOW, "image"), 10)
        finally:
            shot("menu")
        forget_clipboard()
        tap(kde, *entry)
        wait_for("the original on the clipboard", lambda: original(clipboard()), 30)
        print("UI: Copy Image in the right-click menu put the 1200×800 original on the clipboard", flush=True)

        # In the viewer: Copy, then zoom.
        tap(kde, x, y)
        wait_for("the viewer", lambda: "split.png" in words((0, 840, 1180, 900)), 20)
        middle = wait_for("the picture at its fitted size", lambda: magenta_box(), 20)
        row = (middle[1] + middle[3]) // 2
        fitted = wait_for("the colour edge", lambda: edge(row), 20)
        forget_clipboard()
        tap(kde, 1058, 58)  # Copy, left of Download and Close
        try:
            wait_for("the viewer saying it copied", lambda: "copied" in words((0, 840, 1180, 900)), 30)
        finally:
            shot("viewer-copied")
        assert original(clipboard()), "the viewer's copy isn't the original"
        print("UI: the viewer's Copy button copied the original and said so", flush=True)

        at = fitted - 200
        tap(kde, at, row)
        try:
            zoomed = wait_for("zoomed in on the click", lambda: (e := edge(row)) and e > fitted + 150 and e, 10)
        finally:
            shot("zoomed")
        assert abs(zoomed - (fitted + 200)) < 30, (fitted, zoomed)
        tap(kde, at, row)
        wait_for("zoomed out again", lambda: (e := edge(row)) and abs(e - fitted) < 6, 10)
        print(f"UI: a click 200 px left of the colour edge doubled the picture there (edge {fitted} → "
              f"{zoomed}), and the next click fitted it again", flush=True)

        kde.actions("wheel", [{"type": "scroll", "x": at, "y": row, "deltaX": 0, "deltaY": -120}])
        try:
            stepped = wait_for("a wheel step in", lambda: (e := edge(row)) and e > fitted + 20 and e, 10)
        finally:
            shot("wheel")
        kde.actions("wheel", [{"type": "scroll", "x": at, "y": row, "deltaX": 0, "deltaY": 120}])
        wait_for("a wheel step out", lambda: (e := edge(row)) and abs(e - fitted) < 6, 10)
        print(f"UI: the mouse wheel zoomed in a step at the pointer (edge {fitted} → {stepped}) and out again",
              flush=True)


if __name__ == "__main__":
    main()
