"""Link previews (ROADMAP M7), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with 'matrix-synapse[url-preview]' --with requests \
    --with websocket-client --with playwright --with pillow python scripts/qa_link_previews.py

Synapse previews links (`url_preview_enabled`, with 127.0.0.1 allowed) from a
local page with Open Graph tags and an image, and a plain image. In a plain
room, the release client in a private KWin session (as qa_kwin.py needs) must:
- show the page's preview under its message: title, description and image,
  and the plain image link as the picture;
- after **Hide Link Previews** in the channel's right-click menu, show no
  preview, save Element's switch (`org.matrix.room.preview_urls` with
  `disable: true`), and leave a new link unfetched: the page server sees no
  request for it;
- after **Show Link Previews**, preview that link.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import threading
import time
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import free_port, http  # noqa: E402
from qa_media_previews import homeserver, interop, magenta, tap, teal  # noqa: E402

WINDOW = (0, 0, 1180, 900)
FEED = (330, 60, 1180, 800)
SIDEBAR = (72, 40, 330, 900)


def png(w, h, color):
    buf = io.BytesIO()
    Image.new("RGB", (w, h), color).save(buf, "PNG")
    return buf.getvalue()


class Pages:
    """A local site to preview, which remembers what was asked for."""

    def __init__(self):
        self.port = free_port()
        self.asked = []
        pages = {
            "/page.html": ("text/html", self.html("Untethered preview test", "A page about previews.",
                                                  "/wide.png").encode()),
            "/second.html": ("text/html", self.html("Second page title", "Fetched only when allowed.",
                                                    None).encode()),
            "/wide.png": ("image/png", png(1200, 630, (30, 200, 180))),
            "/plain.png": ("image/png", png(600, 400, (225, 30, 190))),
        }
        site = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                site.asked.append(self.path)
                kind, body = pages.get(self.path, ("text/plain", b"missing"))
                self.send_response(200 if self.path in pages else 404)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_):
                pass
        self.server = ThreadingHTTPServer(("127.0.0.1", self.port), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def html(self, title, description, image):
        meta = [f'<meta property="og:title" content="{title}">',
                f'<meta property="og:description" content="{description}">',
                '<meta property="og:site_name" content="Preview Site">']
        if image:
            meta.append(f'<meta property="og:image" content="{self.url(image)}">')
        return f"<html><head><title>{title}</title>{''.join(meta)}</head><body>{description}</body></html>"

    def url(self, path):
        return f"http://127.0.0.1:{self.port}{path}"


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-previews-"))
    synapse = None
    pages = Pages()
    try:
        synapse, hs = homeserver(tmp, """url_preview_enabled: true
url_preview_ip_range_blacklist: ['10.0.0.0/8', '192.168.0.0/16']
url_preview_ip_range_whitelist: ['127.0.0.1']
""")
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        room = fixture["room"]
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]

        def say(body):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json={"msgtype": "m.text", "body": body})

        def switch():
            r = http("GET", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/rooms/{quote(room, safe='')}"
                     "/account_data/org.matrix.room.preview_urls", headers=hdr)
            return r.json()
        say(f"Look at {pages.url('/page.html')} please")
        say(f"And this picture {pages.url('/plain.png')}")
        print(f"Synapse at {hs} previews {pages.url('/')}", flush=True)
        look(tmp, args.shots, pages, say, switch)
        print("PASS: link previews show with their image, hide and stop fetching from the channel menu, "
              "and show again", flush=True)
    finally:
        pages.server.shutdown()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, pages, say, switch):
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

        def words(rect=FEED):
            return [w.strip(".,:|").lower() for w, _, _ in kde.words(rect)]

        def entry():
            return next(((x, y) for w, x, y in kde.words(SIDEBAR) if "media-proof" in w.lower()), None)
        tap(kde, *wait_for("media-proof in the room list", entry, 90))

        def previews():
            im = kde.capture().crop(FEED)
            ws = words()
            return "untethered" in ws and "previews" in ws and teal(im) > 20000 and magenta(im) > 20000
        try:
            wait_for("the page's preview with its image, and the plain image", previews, 60)
        finally:
            shot("previews")
        print("UI: the page shows its title, description and image under the message, and the image link "
              "shows the picture", flush=True)

        def menu(label):
            x, y = wait_for("media-proof in the room list", entry, 10)
            kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 250},
                                    {"type": "pointerDown", "button": 2}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": 2}])
            tap(kde, *wait_for(f"{label} in the channel menu",
                               lambda: next(((x, y) for w, x, y in kde.words(WINDOW) if w.lower() == label), None), 10))
        menu("hide")

        def hidden():
            im = kde.capture().crop(FEED)
            return "untethered" not in words() and teal(im) < 2000 and magenta(im) < 2000
        wait_for("the previews gone", hidden, 15)
        wait_for("Element's switch saved", lambda: switch().get("disable") is True, 15)
        say(f"Second link {pages.url('/second.html')}")
        wait_for("the second link's message", lambda: "second" in words(), 30)
        time.sleep(6)
        shot("hidden")
        assert "/second.html" not in pages.asked, pages.asked
        assert "title" not in words(), "a preview showed while hidden"
        print("UI: Hide Link Previews removed both previews, saved disable: true where Element keeps it, and "
              "a new link went unfetched", flush=True)
        menu("show")

        def second():
            return "/second.html" in pages.asked and {"second", "page", "title"} <= set(words())
        try:
            wait_for("the second link's preview", second, 30)
        finally:
            shot("shown-again")
        assert switch().get("disable") is False, switch()
        print("UI: Show Link Previews brought them back and previewed the new link", flush=True)


if __name__ == "__main__":
    main()
