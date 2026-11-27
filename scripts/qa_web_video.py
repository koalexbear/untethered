"""Video in the browser (ROADMAP P4), live against a disposable Synapse.

Build the web client first (scripts/rust-web.sh --release), and the debug
client (it seeds the profile). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_web_video.py

The test account's room has an MP4 (H.264, 320×240, two seconds) with a
magenta poster. In Chromium the web client must open it in the media viewer
and play it in the page's own player over the viewer's stage (its time
moving on), and take the player away when the viewer closes.
--shots DIR keeps the screenshots.
"""
import argparse
import contextlib
import functools
import io
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import ThreadingHTTPServer
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http  # noqa: E402
from qa_matrix_sdk_web import VIEWPORT, Quiet  # noqa: E402
from qa_media_previews import homeserver, interop  # noqa: E402
from qa_web_login import login_client  # noqa: E402

MAGENTA = (225, 30, 190)
DIST = ROOT / "rust-client/web/dist"


def near(pixel, colour, slack=50):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    assert (DIST / "index.html").is_file(), "scripts/rust-web.sh --release first"
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-webvideo-"))
    synapse = None
    try:
        clip = tmp / "clip.mp4"
        subprocess.run(["gst-launch-1.0", "-q", "videotestsrc", "num-buffers=60", "pattern=ball", "!",
                        "video/x-raw,width=320,height=240,framerate=30/1", "!", "videoconvert", "!", "openh264enc",
                        "!", "h264parse", "!", "mp4mux", "!", "filesink", f"location={clip}"],
                       check=True, capture_output=True, timeout=60)
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)

        def upload(data, name, mime):
            return http("POST", f"{hs}/_matrix/media/v3/upload?filename={name}",
                        headers={**hdr, "Content-Type": mime}, data=data).json()["content_uri"]
        poster = io.BytesIO()
        Image.new("RGB", (320, 240), MAGENTA).save(poster, "JPEG")
        http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(fixture['room'], safe='')}/send/m.room.message/"
             f"{secrets.token_hex(8)}", headers=hdr, json={
                 "msgtype": "m.video", "body": "clip.mp4", "url": upload(clip.read_bytes(), "clip.mp4", "video/mp4"),
                 "info": {"w": 320, "h": 240, "duration": 2000, "mimetype": "video/mp4",
                          "size": clip.stat().st_size,
                          "thumbnail_url": upload(poster.getvalue(), "poster.jpg", "image/jpeg"),
                          "thumbnail_info": {"w": 320, "h": 240, "mimetype": "image/jpeg",
                                             "size": len(poster.getvalue())}}})
        print(f"Synapse at {hs}", flush=True)
        look(fixture, args.shots)
        print("PASS: the web client plays a video in the media viewer, in the page's own player", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(fixture, shots):
    from playwright.sync_api import sync_playwright
    chromium = os.environ.get("CHROMIUM") or shutil.which("chromium")
    with contextlib.ExitStack() as stack:
        server = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(DIST)))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        stack.callback(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}/"
        pw = stack.enter_context(sync_playwright())
        browser = pw.chromium.launch(executable_path=chromium, args=["--disable-dev-shm-usage",
                                                                      "--autoplay-policy=no-user-gesture-required"])
        stack.callback(browser.close)
        page = browser.new_context(viewport=VIEWPORT).new_page()

        def shot(name):
            data = page.screenshot()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                (shots / f"{name}.png").write_bytes(data)
            return Image.open(io.BytesIO(data)).convert("RGB")

        def poster():
            im = shot("room")
            px = im.load()
            pts = [(x, y) for y in range(0, im.height, 6) for x in range(330, im.width, 6) if near(px[x, y], MAGENTA)]
            if len(pts) < 20:
                return None
            return sum(p[0] for p in pts) // len(pts), sum(p[1] for p in pts) // len(pts)

        def until(what, check, seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if (got := check()):
                    return got
                time.sleep(0.5)
            raise RuntimeError(f"Timed out waiting for {what}")

        login_client(page, url, fixture)
        print("Chromium: signed in to the web client", flush=True)
        at = until("the video's poster", poster, 90)
        page.mouse.click(*at)
        state = """() => { const v = document.getElementById('untethered-video');
            if (!v) return null;
            const r = v.getBoundingClientRect();
            return {ready: v.readyState, time: v.currentTime, paused: v.paused, w: v.videoWidth,
                    left: r.left, top: r.top, width: r.width, height: r.height}; }"""
        try:
            got = until("the page's player playing", lambda: (s := page.evaluate(state)) and s["ready"] >= 2
                        and s["time"] > 0.3 and s, 60)
        finally:
            shot("playing")
        assert got["w"] == 320, got
        assert got["left"] >= 60 and got["top"] >= 50 and got["width"] < VIEWPORT["width"] - 100, got
        print(f"Chromium: the viewer played the MP4 in the page's player ({got['w']} px wide, at "
              f"{got['time']:.1f} s), over its stage", flush=True)
        page.mouse.click(VIEWPORT["width"] - 34, 30)  # the viewer's Close
        until("the player gone", lambda: page.evaluate(state) is None, 20)
        shot("closed")
        print("Chromium: closing the viewer took the player away", flush=True)


if __name__ == "__main__":
    main()
