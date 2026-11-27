"""Media viewer polish (ROADMAP P3), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_viewer_polish.py

The test account sends ten screenshots within a minute: originals of
1600×1000 in ten colours, each with the same grey thumbnail, so the viewer
shows grey only while an original is still loading. The release client in
a private KWin session (as qa_kwin.py needs) must:
- lay the ten out as a gallery: a grid of three columns for the first nine;
- open the first in the viewer, with a strip of thumbnails along the bottom;
- step through all ten with the right arrow, each original already showing
  in the first screenshot after the key press (no loading pause).
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
from qa_matrix_sdk import http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
GREY = (128, 128, 128)
COLOURS = [(220, 40, 40), (40, 180, 60), (40, 80, 220), (230, 200, 30), (200, 50, 200),
           (30, 200, 200), (240, 130, 20), (120, 60, 200), (20, 120, 90), (250, 250, 250)]
RIGHT = ""


def near(pixel, colour, slack=45):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-viewer-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)

        def upload(im, name):
            buf = io.BytesIO()
            im.save(buf, "PNG")
            data = buf.getvalue()
            mxc = http("POST", f"{hs}/_matrix/media/v3/upload?filename={name}",
                       headers={**hdr, "Content-Type": "image/png"}, data=data).json()["content_uri"]
            return mxc, len(data)
        thumb, thumb_size = upload(Image.new("RGB", (320, 200), GREY), "thumb.png")
        noise = Image.merge("RGB", [Image.effect_noise((1600, 1000), 60)] * 3)
        for n, colour in enumerate(COLOURS, 1):
            original = Image.blend(Image.new("RGB", (1600, 1000), colour), noise, 0.06)
            mxc, size = upload(original, f"shot-{n}.png")
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(fixture['room'], safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json={
                     "msgtype": "m.image", "body": f"shot-{n}.png", "url": mxc,
                     "info": {"w": 1600, "h": 1000, "mimetype": "image/png", "size": size,
                              "thumbnail_url": thumb,
                              "thumbnail_info": {"w": 320, "h": 200, "mimetype": "image/png",
                                                 "size": thumb_size}}})
        print(f"Synapse at {hs}", flush=True)
        look(tmp, args.shots)
        print("PASS: ten screenshots lay out as a gallery and step through the viewer without a loading pause",
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
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)

        def shot(name):
            im = kde.capture().crop(WINDOW).convert("RGB")
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def runs(im, y, colour, x0=330, x1=1180):
            """Stretches of `colour` along row `y`, as (start, end)."""
            px, found, start = im.load(), [], None
            for x in range(x0, x1):
                if near(px[x, y], colour, 20):
                    start = x if start is None else start
                elif start is not None:
                    found.append((start, x))
                    start = None
            return [r for r in found if r[1] - r[0] > 40]

        def grid():
            im = kde.capture().crop(WINDOW).convert("RGB")
            for y in range(120, 820, 6):
                cells = runs(im, y, GREY)
                if len(cells) == 3:
                    return y, cells
            return None

        # The gallery: three grey cells side by side.
        try:
            y, cells = wait_for("a three-column gallery", grid, 90)
        finally:
            shot("gallery")
        print(f"UI: the screenshots lay out as a gallery, three to a row ({cells})", flush=True)

        # The first one, whose original loads as the viewer opens.
        first = (cells[0][0] + cells[0][1]) // 2
        tap(kde, first, y)
        try:
            wait_for("the first original", lambda: near(kde.capture().crop(WINDOW).convert("RGB")
                                                         .getpixel((590, 470)), COLOURS[0]), 30)
        finally:
            im = shot("viewer-1")
        # The strip: a row of grey thumbnails near the bottom.
        strip = max((runs(im, y, GREY, 0, 1180) for y in range(760, 880, 2)), key=len)
        # The first one and the seven after it.
        assert len(strip) == 8, f"the strip shows {len(strip)} thumbnails"
        print(f"UI: the viewer opened the first original, with a strip of {len(strip)} thumbnails", flush=True)
        time.sleep(2)  # the next ones load ahead meanwhile

        late = []
        for n in range(2, len(COLOURS) + 1):
            kde.keys(("Down", RIGHT), ("Up", RIGHT))
            im = shot(f"viewer-{n}")
            if not near(im.getpixel((590, 470)), COLOURS[n - 1]):
                late.append((n, im.getpixel((590, 470))))
            time.sleep(0.6)
        assert not late, f"originals not there at once: {late}"
        print("UI: the viewer stepped through all ten, each original showing at once", flush=True)


if __name__ == "__main__":
    main()
