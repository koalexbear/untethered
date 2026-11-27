"""Image previews and the media viewer, live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI; debug builds panic under synthetic input). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_media_previews.py

Seeds a plain room with a long backlog, 140 messages with an image after
every seventh: one over 4 MiB without a thumbnail, one with no `info` at all,
one without a size, one with a thumbnail as Element sends it, the rest with
size and dimensions only. The release client opens it in a private KWin
session (KWin, KDE inputsynth as KWIN_INPUTSYNTH, spectacle, kdotool and
tesseract, as qa_kwin.py needs), and scrolls back to the first message with
Page Up. At every step, within a few seconds:
- every image on screen is a picture: no "Show image preview" or "Retry"
  button, no file card, no empty placeholder left;
- the lines on screen haven't moved while the pictures arrived.
Halfway back it opens a picture in the media viewer and closes it again:
the timeline must be where it was. Back at the newest messages, it selects
message text by triple click, by drag and by double click and copies it
with Ctrl+C: pasted into the composer, it must be exactly what was
selected. Last, it types in the composer, drags a picture in from Dolphin
(needs `dolphin`) and types on: the composer must keep the keyboard, and
Enter must send the picture and the text.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import random
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote

import requests
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, free_port, http  # noqa: E402

COUNT = 140
EVERY = 7
WINDOW = (0, 0, 1180, 900)
# The timeline, between the room header and the composer, right of the room list.
FEED = (330, 60, 1180, 800)


def interop():
    spec = importlib.util.spec_from_file_location("interop", ROOT / "scripts/test-media-interop.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def png(w, h, *, noise=False):
    """A magenta picture the screenshots can find; `noise` makes it large."""
    if noise:
        rng = random.Random(7)
        raw = bytes(v for _ in range(w * h) for v in (rng.randint(200, 250), rng.randint(10, 60),
                                                       rng.randint(160, 220)))
        im = Image.frombytes("RGB", (w, h), raw)
    else:
        im = Image.new("RGB", (w, h), (225, 30, 190))
        d = ImageDraw.Draw(im)
        for k in range(1, 6):
            d.line([(0, h * k // 6), (w, h * k // 6)], fill=(245, 120, 225), width=max(2, h // 60))
    buf = io.BytesIO()
    im.save(buf, "PNG", compress_level=1 if noise else 6)
    return buf.getvalue()


def pink(p):
    """A test picture's pixel, its lighter stripes included."""
    r, g, b = p[:3]
    return r > 190 and g < 140 and 140 < b < 240


def tap(kde, x, y):
    """A click that moves first: inputsynth can deliver a press before the
    pointer's move reaches the client."""
    kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 250},
                            {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 80},
                            {"type": "pointerUp", "button": 0}])


def magenta(im):
    return sum(n for n, c in im.getcolors(im.width * im.height) if pink(c))


def teal(im):
    return sum(n for n, (r, g, b) in im.getcolors(im.width * im.height) if r < 70 and g > 160 and 140 < b < 220)


def placeholders(im):
    # The loading box's colour (view.rs FLOAT).
    return sum(n for n, c in im.getcolors(im.width * im.height) if c == (17, 18, 20))


def homeserver(tmp, extra=""):
    """A disposable Synapse in `tmp`: its process and address. `extra` is more
    homeserver configuration (YAML)."""
    subprocess.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config",
                    "-H", SERVER_NAME, "--report-stats", "no"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
    port = free_port()
    hs = f"http://127.0.0.1:{port}"
    limit = "{per_second: 1000, burst_count: 1000}"
    (tmp / "local.yaml").write_text(f"""listeners:
  - port: {port}
    type: http
    tls: false
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [client, media]}}]
public_baseurl: {hs}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: []
rc_message: {limit}
rc_room_creation: {limit}
rc_joins: {{local: {limit}, remote: {limit}}}
rc_login: {{address: {limit}, account: {limit}, failed_attempts: {limit}}}
{extra}""")
    synapse = subprocess.Popen([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "-c", "local.yaml"],
                               cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(300):
        if synapse.poll() is not None:
            raise RuntimeError("Disposable homeserver exited")
        try:
            http("GET", hs + "/_matrix/client/versions")
            break
        except requests.ConnectionError:
            time.sleep(.1)
    return synapse, hs


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-previews-", dir=os.environ.get("TMPDIR")))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        room = fixture["room"]

        def send(content):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=hdr, json=content)

        def upload(data, name):
            r = requests.post(hs + "/_matrix/media/v3/upload", params={"filename": name},
                              headers={**hdr, "Content-Type": "image/png"}, data=data, timeout=60)
            r.raise_for_status()
            return r.json()["content_uri"]

        kinds = {}
        for n in range(1, COUNT + 1):
            send({"msgtype": "m.text", "body": f"Backlog message {n:03}"})
            if n % EVERY:
                continue
            name = f"picture-{n}.png"
            if n == 7:
                data = png(1700, 1300, noise=True)
                assert len(data) > 4 * 1024 * 1024, len(data)
                info, kinds[n] = {"w": 1700, "h": 1300, "mimetype": "image/png", "size": len(data)}, "over 4 MiB"
            elif n == 14:
                data, info, kinds[n] = png(300, 200), None, "no info"
            elif n == 21:
                data = png(640, 400)
                info, kinds[n] = {"w": 640, "h": 400, "mimetype": "image/png"}, "no size"
            elif n == 28:
                data, small = png(1600, 900), png(800, 450)
                info = {"w": 1600, "h": 900, "mimetype": "image/png", "size": len(data),
                        "thumbnail_url": upload(small, "thumb.png"),
                        "thumbnail_info": {"w": 800, "h": 450, "mimetype": "image/png", "size": len(small)}}
                kinds[n] = "with a thumbnail"
            else:
                w, h = random.Random(n).choice([(1200, 800), (800, 1200), (500, 500), (1920, 1080)])
                data = png(w, h)
                info = {"w": w, "h": h, "mimetype": "image/png", "size": len(data)}
            content = {"msgtype": "m.image", "body": name, "url": upload(data, name)}
            if info:
                content["info"] = info
            send(content)
        print(f"Synapse at {hs}: {COUNT} messages and {COUNT // EVERY} images in {room} "
              f"({', '.join(f'{n}: {k}' for n, k in kinds.items())})", flush=True)
        look(tmp, args.shots)
        print("PASS: every image previewed without moving the timeline, the viewer kept its position, "
              "message text selected and copied, and a drop kept the composer's keyboard", flush=True)
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

        def shot(name, im=None):
            im = im or kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def lines():
            """The backlog numbers on screen and where they are."""
            found = {}
            for w, x, y in kde.words(FEED):
                if re.fullmatch(r"\d{3}", w) and x > 400:
                    found.setdefault(w, (x, y))
            return found

        def feed_words():
            return [w.lower().strip(".,:|") for w, _, _ in kde.words(FEED)]

        def media_proof():
            return next(((x, y) for w, x, y in kde.words((72, 40, 330, 900)) if "media-proof" in w.lower()), None)
        tap(kde, *wait_for("media-proof in the room list", media_proof, 90))
        wait_for("the newest message", lambda: f"{COUNT:03}" in lines(), 60)

        def settled():
            im = kde.capture().crop(FEED)
            words = feed_words()
            stuck = [w for w in words if w in ("show", "retry", "preview") or w.startswith("picture-")]
            boxes = placeholders(im)
            return None if stuck or boxes > 4000 else (im, magenta(im))

        def picture_center(im):
            """The middle of the tallest picture wholly on screen."""
            rows = [y for y in range(im.height) if sum(pink(im.getpixel((x, y))) for x in range(0, im.width, 4)) > 20]
            bands, start = [], None
            for a, b in zip(rows, rows[1:] + [None]):
                start = a if start is None else start
                if b != a + 1:
                    bands.append((start, a))
                    start = None
            whole = [(t, b) for t, b in bands if b - t >= 80 and t >= 4 and b <= im.height - 5]
            if not whole:
                return None
            top, bottom = max(whole, key=lambda b: b[1] - b[0])
            mid = (top + bottom) // 2
            xs = [x for x in range(im.width) if pink(im.getpixel((x, mid)))]
            return FEED[0] + (xs[0] + xs[-1]) // 2, FEED[1] + mid

        viewer_checked = False
        seen_pictures = 0
        for step in range(40):
            if step:
                kde.keys(("Down", ""), ("Up", ""))  # Page Up
                time.sleep(.4)
            before = lines()
            im, drawn = wait_for(f"pictures settled after Page Up {step}", settled, 15)
            after = lines()
            moved = {w: (before[w], after[w]) for w in before.keys() & after.keys() if abs(before[w][1] - after[w][1]) > 3}
            shot(f"step-{step:02}", im)
            assert not moved, f"lines moved while pictures loaded at step {step}: {moved}"
            seen_pictures += drawn > 20000
            if not viewer_checked and step >= 4 and (center := picture_center(im)):
                held = lines()

                def viewer():
                    words = [w.lower() for w, _, _ in kde.words(WINDOW)]
                    return any(w.startswith("picture-") for w in words) or {"of", "download"} <= set(words)
                # One more try if a press still went astray.
                for attempt in range(2):
                    tap(kde, *center)
                    try:
                        wait_for("the media viewer", viewer, 8)
                        break
                    except RuntimeError:
                        shot(f"viewer-missing-{attempt}")
                else:
                    raise RuntimeError("the picture never opened the media viewer")
                shot("viewer-open")
                kde.keys(("Down", ""), ("Up", ""))  # Escape
                wait_for("the viewer closed", lambda: not viewer(), 10)
                time.sleep(.5)
                back = lines()
                shot("viewer-closed")
                shifted = {w: (held.get(w), back.get(w)) for w in held.keys() | back.keys()
                           if w not in held or w not in back or abs(held[w][1] - back[w][1]) > 3}
                assert held and not shifted, f"the viewer moved the timeline: {shifted}"
                print(f"UI: opening a picture in the viewer and closing it left lines {sorted(held)} in place",
                      flush=True)
                viewer_checked = True
            if "001" in after:
                break
        else:
            raise AssertionError("Page Up never reached the first message")
        assert viewer_checked, "no whole picture on screen to open in the viewer"
        top_pink = magenta(kde.capture().crop(FEED))
        assert top_pink > 20000, f"the oldest pictures aren't drawn ({top_pink} magenta pixels)"
        print(f"UI: Page Up {step} times to the first message; every step settled with pictures drawn "
              f"({seen_pictures} steps showed pictures), never a preview button, file card or leftover "
              "placeholder, and no line moved while pictures arrived", flush=True)
        def jump():
            # The bar sits just above the composer, below the timeline's rows.
            return next(((x, y) for w, x, y in kde.words((330, 600, 1180, 830)) if w.lower() == "jump"), None)
        tap(kde, *wait_for("Jump to present", jump, 10))
        wait_for("the newest message", lambda: f"{COUNT:03}" in lines(), 15)
        selected(kde, shot)
        dropped(stack, kde, tmp, shot)


def selected(kde, shot):
    """Select message text three ways, copy it with Ctrl+C and paste it into
    the composer: the paste must be exactly what was selected."""
    from qa_native_paste import wait_for
    # The composer's text, between its + and emoji buttons.
    composer = (372, 825, 1070, 870)

    def line(number):
        words = kde.words(FEED)
        n = next(((x, y) for w, x, y in words if w == number), None)
        found = n and {w.lower(): (x, y) for w, x, y in words if abs(y - n[1]) < 8}
        return found if found and {"backlog", "message"} <= found.keys() else None

    def clicks(x, y, count):
        # Iced counts a click as the next of a series within 300 ms.
        press = [{"type": "pointerDown", "button": 0}, {"type": "pointerUp", "button": 0}]
        kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 250}]
                    + press * count)

    def typed():
        """The composer's words, without its caret and empty placeholder."""
        words = [w.strip("|").rstrip(")]") for w, _, _ in kde.words(composer)]  # the caret OCRs as | ) or ]
        return [w for w in words if w and w not in ("Message", "#media-proof", "media-proof")]

    def pasted(what):
        kde.keys(("Down", "\ue009"), ("Down", "c"), ("Up", "c"), ("Up", "\ue009"))
        tap(kde, 700, WINDOW[3] - 53)
        kde.keys(("Down", "\ue009"), ("Down", "v"), ("Up", "v"), ("Up", "\ue009"))
        try:
            return wait_for(f"{what} pasted", lambda: typed() or None, 10)
        finally:
            shot(f"selected-{what}")
            kde.select_all()
            kde.keys(("Down", "\ue003"), ("Up", "\ue003"))  # Backspace
            wait_for("the composer emptied", lambda: not typed(), 10)

    words = wait_for("line 139", lambda: line("139"), 10)
    clicks(*words["message"], 3)
    got = pasted("line")
    assert got == ["Backlog", "message", "139"], got
    words = wait_for("line 138", lambda: line("138"), 10)
    mx, my = words["message"]
    kde.actions("pointer", [{"type": "pointerMove", "x": mx, "y": my}, {"type": "pause", "duration": 250},
                            {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 120},
                            {"type": "pointerMove", "x": mx + 150, "y": my, "duration": 250},
                            {"type": "pointerMove", "x": mx + 400, "y": my, "duration": 250},
                            {"type": "pause", "duration": 150}, {"type": "pointerUp", "button": 0}])
    got = pasted("drag")
    assert got and got[-1] == "138" and "Backlog" not in got and got[0].endswith("sage"), got
    words = wait_for("line 137", lambda: line("137"), 10)
    clicks(*words["backlog"], 2)
    got = pasted("word")
    assert got == ["Backlog"], got
    print("UI: message text selects by triple click (the line), drag (from the middle of a word to "
          "past its end) and double click (a word), and Ctrl+C copies exactly that", flush=True)


def dropped(stack, kde, tmp, shot):
    """Type, drop a file from Dolphin, type on: the composer keeps the
    keyboard through the drop, and Enter sends the file and the text."""
    from qa_native_paste import child, wait_for
    files = tmp / "DropFiles"
    files.mkdir()
    im = Image.new("RGB", (480, 320), (30, 200, 180))
    im.save(files / "drop-me.png")

    # The composer, above the footer (its grey placeholder doesn't OCR).
    tap(kde, 700, WINDOW[3] - 53)
    kde.type("dropped")
    dolphin = stack.enter_context(child(["dolphin", "--new-window", str(files)], kde.env))
    dw = wait_for("Dolphin window", lambda: kde.window("DropFiles.*Dolphin"), 30)
    kde.position(dw, 1190, 0, 900, 950)
    assert int(kde.command("kdotool", "getwindowpid", dw).stdout) == dolphin.pid
    kde.command("kdotool", "windowactivate", dw)
    kde.keys(("Down", "\ue009"), ("Down", "3"), ("Up", "3"), ("Up", "\ue009"))  # details view

    def label():
        return next(((x, y) for w, x, y in kde.words((1190, 60, 2090, 700)) if w.lower() == "drop-me.png"), None)
    sx, sy = wait_for("drop-me.png in Dolphin", label, 20)
    kde.click(1650, 590)
    kde.select_all()
    kde.drag(sx, sy, 700, 400)

    def staged():
        return any("drop-me" in w.lower() for w, _, _ in kde.words((330, 500, 1180, 900)))
    wait_for("the dropped file staged in the composer", staged, 20)
    shot("dropped")
    kde.type(" and sent")
    kde.keys(("Down", "\ue007"), ("Up", "\ue007"))  # Enter

    def sent():
        words = {w.lower().strip(".,") for w, _, _ in kde.words(FEED)}
        return {"dropped", "and", "sent"} <= words and teal(kde.capture().crop(FEED)) > 20000
    try:
        wait_for("the text and the dropped picture sent with Enter", sent, 30)
    finally:
        shot("dropped-sent")
    print("UI: typing went on in the composer after a Dolphin drop, and Enter sent the picture and "
          "\"dropped and sent\"", flush=True)


if __name__ == "__main__":
    main()
