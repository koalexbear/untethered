"""The matrix-sdk engine in Chromium (step 7 of rust-client/docs/matrix-sdk-migration.md).

Serves a Trunk build of the web client, signs the fixture account in with
`?engine=matrix-sdk`, verifies the device with its recovery key in the page's
dialog, opens the encrypted room, and checks with the gomuks peer: text both
ways, a pasted image (decrypted by the peer byte for byte), the peer's
encrypted image saved byte for byte and previewed inline, and a browser
notification for a message in another room. Then it reloads: the session must come back from the page's
storage and IndexedDB without signing in again.
"""
import base64
import contextlib
import functools
import io
import os
import secrets
import shutil
import struct
import threading
import time
import zlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

from qa_ui import CHAT_X, read

VIEWPORT = {"width": 1180, "height": 760}


class Quiet(SimpleHTTPRequestHandler):
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm",
                      ".js": "text/javascript"}

    def log_message(self, *args):
        pass


def wait_for(label, check, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if result := check():
            return result
        time.sleep(.25)
    raise RuntimeError(f"Timed out waiting for {label}")


def words(page):
    shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
    return [(w.lower().strip(".,:|()"), x, y) for w, x, y in read(shot)]


def title(page, part, timeout=45):
    page.wait_for_function("p => document.title.includes(p)", arg=part, timeout=timeout * 1000)


def field(page, caption):
    """The input under a sign-in caption. The card recentres when it shows a
    notice, so find the caption rather than a fixed position."""
    def find():
        shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
        return next(((x, y) for w, x, y in read(shot) if w.strip(".:") == caption), None)
    _, y = wait_for(f"the {caption} field", find)
    return 590, y + 37


def open_room(page, name):
    """Click the room in the sidebar, then wait for its title."""
    point = wait_for(f"{name} in the sidebar",
                     lambda: next(((x, y) for w, x, y in words(page) if w == name and x < CHAT_X), None))
    page.mouse.click(*point)
    title(page, name)


def shows(page, token, timeout=45):
    wait_for(f"{token!r} in the conversation",
             lambda: any(token in w and x > CHAT_X for w, x, _ in words(page)), timeout)


# Records page notifications instead of showing them.
NOTIFICATIONS = """
window.__notes = [];
window.Notification = class {
    constructor(title, options) { window.__notes.push({title, body: options?.body || ""}); }
    close() {}
    static get permission() { return "granted"; }
    static requestPermission() { return Promise.resolve("granted"); }
};
"""

# Paste a file into the page the way a user's clipboard does.
PASTE = """([name, data]) => {
    const bytes = Uint8Array.from(atob(data), c => c.charCodeAt(0));
    const files = new DataTransfer();
    files.items.add(new File([bytes], name, {type: "image/png"}));
    window.dispatchEvent(new ClipboardEvent("paste", {clipboardData: files, bubbles: true}));
}"""


def magenta(width, height):
    """A solid magenta PNG: a colour the client's theme never paints."""
    rows = b"".join(b"\0" + bytes((255, 0, 255)) * width for _ in range(height))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def magenta_pixels(page):
    """Sampled magenta pixels in the conversation."""
    shot = Image.open(io.BytesIO(page.screenshot())).convert("RGB")
    return sum(1 for x in range(CHAT_X + 10, shot.width, 3) for y in range(60, shot.height - 90, 3)
               if (lambda r, g, b: r > 225 and g < 40 and b > 225)(*shot.getpixel((x, y))))


def check_web(dist, fixture, peer, png, room_name="sdk-proof"):
    chromium = os.environ.get("CHROMIUM") or shutil.which("chromium")
    assert chromium, "Set CHROMIUM or install chromium"
    assert (dist / "index.html").is_file(), f"No web build in {dist}"
    with contextlib.ExitStack() as stack:
        server = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(dist)))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        stack.callback(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}/?engine=matrix-sdk"
        pw = stack.enter_context(sync_playwright())
        browser = pw.chromium.launch(executable_path=chromium, args=["--disable-dev-shm-usage"])
        stack.callback(browser.close)
        context = browser.new_context(viewport=VIEWPORT)
        context.add_init_script(NOTIFICATIONS)
        page = context.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(url, wait_until="commit")
        try:
            title(page, "Preview", 90)
            # gomuks's browser database lives in the origin private file
            # system: with an entry there, the sign-in screen explains the
            # switch to the new engine.
            page.evaluate("""async () => {
                const root = await navigator.storage.getDirectory();
                await root.getFileHandle("gomuks-was-here", {create: true});
            }""")
            page.reload(wait_until="commit")
            title(page, "Preview", 90)
            page.mouse.click(1095, 73)  # Account
            title(page, "Account")
            wait_for("the upgrade notice", lambda: {"new", "engine", "recovery"} <= {w for w, _, _ in words(page)})
            print("matrix-sdk in Chromium: the sign-in screen explains the switch from gomuks", flush=True)
            for caption, value in [("HOMESERVER", "server"), ("USERNAME", "username"), ("PASSWORD", "password")]:
                page.mouse.click(*field(page, caption))
                page.keyboard.type(fixture[value], delay=6)
            page.keyboard.press("Enter")
            secret = page.get_by_label("Recovery key or passphrase")
            secret.fill(fixture["key"], timeout=90000)
            secret.press("Enter")
            page.wait_for_function("!document.title.includes('Account')", timeout=90000)
            assert any(w.url.endswith("/engine-worker.js") for w in page.workers), [w.url for w in page.workers]
            print("matrix-sdk in Chromium: signed in and verified with the recovery key, in a worker", flush=True)
            open_room(page, room_name)
            mine = f"web to gomuks {secrets.token_hex(6)}"
            page.mouse.click(545, 690)
            page.keyboard.type(mine, delay=4)
            page.keyboard.press("Enter")
            peer.decrypted(mine)
            print("gomuks peer decrypted the browser's message", flush=True)
            token = secrets.token_hex(6)
            peer.rpc("send_message", {"room_id": fixture["room"], "text": f"gomuks to web {token}"})
            shows(page, token)
            print("matrix-sdk in Chromium: decrypted and showed the peer's message", flush=True)

            content = lambda e: e.get("decrypted") or e.get("content") or {}
            mine = png(600, 400)
            page.evaluate(PASTE, ["from-web.png", base64.b64encode(mine).decode()])
            shows(page, "from-web.png")
            page.mouse.click(545, 690)
            page.keyboard.press("Enter")
            image = content(peer.find("the browser's image",
                                      lambda e: content(e).get("filename") == "from-web.png"
                                      and str(e.get("event_id", "")).startswith("$")))
            assert image.get("msgtype") == "m.image", image
            assert (image["info"].get("w"), image["info"].get("h")) == (600, 400), image
            assert peer.media(image["file"]["url"], True) == mine
            print("gomuks peer decrypted the browser's pasted image byte for byte", flush=True)

            theirs = magenta(320, 200)
            descriptor = peer.upload(theirs, "from-gomuks.png", encrypt=True)
            peer.rpc("send_message", {"room_id": fixture["room"], "base_content": descriptor, "text": ""})

            def lowest(match):
                """The lowest conversation word that `match`es: the newest message's."""
                hits = [(x, y) for w, x, y in words(page) if match(w) and x > CHAT_X]
                return max(hits, key=lambda p: p[1]) if hits else None
            # The card offers "Download from-gomuks.png" until the preview loads.
            name = wait_for("the peer's attachment", lambda: lowest(lambda w: w.startswith("from-gomuks")))
            with page.expect_download(timeout=45000) as saved:
                page.mouse.click(*name)
            assert Path(saved.value.path()).read_bytes() == theirs
            print("matrix-sdk in Chromium: saved the peer's encrypted image byte for byte", flush=True)
            page.mouse.click(*wait_for("Show image preview", lambda: lowest(lambda w: w == "preview")))
            wait_for("the peer's image inline", lambda: magenta_pixels(page) > 500)
            print("matrix-sdk in Chromium: previewed the peer's encrypted image inline", flush=True)

            alert = secrets.token_hex(6)
            peer.rpc("send_message", {"room_id": fixture["unread_room"], "text": f"alert {alert}"})
            page.wait_for_function("t => window.__notes.some(n => n.body.includes(t))", arg=alert, timeout=45000)
            print("matrix-sdk in Chromium: a browser notification for the other room's message", flush=True)

            page.reload(wait_until="commit")
            page.wait_for_function("document.title.includes('Untethered') && !document.title.includes('Preview')",
                                   timeout=90000)
            if room_name not in page.title():
                open_room(page, room_name)
            shows(page, token)
            print("matrix-sdk in Chromium: reload restored the session and the room's history", flush=True)
        except Exception:
            page.screenshot(path=str(dist.parent / "qa-web-failure.png"))
            print("Browser title:", page.title(), "errors:", errors[-5:], flush=True)
            print("Browser words:", [w for w, _, _ in words(page)], flush=True)
            raise
        assert not errors, errors
