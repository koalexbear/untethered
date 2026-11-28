"""Server settings (ROADMAP C1), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_server_settings.py

The test account owns a space, Workshop (topic "Making things"), with the
channels general (topic "Chatter") and random, and no image pack; Enzo is a
member. Enzo owns a second space, Library, where the test account is an
ordinary member; it has a pack with a cyan :leaf:. The release client in a
private KWin session (as qa_kwin.py needs) opens Server Settings from
Workshop's server menu and must:
- save a new name and topic, an icon and a banner picked in KDE's file
  dialog: Enzo reads them over the client-server API (the banner under
  Untethered's own `io.github.koalexbear.untethered.banner`), the uploads
  byte for byte, and the banner shows over the channel list;
- rename general and change its topic from the Channels tab;
- create the space's emoji pack, upload :blobwave: (green) into it, and
  find it in the emoji picker; offer it as a sticker too, and find it in the
  sticker picker; rename it :wave:, and find that in `:` autocomplete;
  delete it, and find the picker without it, each change read back from the
  pack's state event;
- show Library's settings read-only: no upload buttons, no fields, its name
  unchanged after typing at it.
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
import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, read  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
RAIL_X = 36
FIRST_SPACE = 186
RAIL_STEP = 56
# Server Settings: the tabs left of this, the page right of it.
NAV = (0, 40, 400, 900)
PAGE = (400, 40, 1180, 900)
ENTER, CONTROL, ESCAPE, BACKSPACE = "", "", "", ""
GREEN = (40, 220, 60)
ORANGE = (250, 140, 20)
BLUE = (30, 90, 230)
CYAN = (20, 220, 230)
ICON_GREY = (181, 186, 193)
BANNER_EVENT = "io.github.koalexbear.untethered.banner"


def near(pixel, colour, slack=40):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def png(colour, w, h):
    out = io.BytesIO()
    Image.new("RGB", (w, h), colour).save(out, "PNG")
    return out.getvalue()


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-settings-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, _, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))

        def create(headers, **body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=headers, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def put_state(headers, room, kind, key, content):
            http("PUT", f"{path(room)}/state/{kind}/{quote(key, safe='')}", headers=headers, json=content)

        def link(headers, space, child):
            put_state(headers, space, "m.space.child", child, {"via": [SERVER_NAME]})
            put_state(headers, child, "m.space.parent", space, {"via": [SERVER_NAME], "canonical": True})

        space = {"type": "m.space"}
        workshop = create(hdr, name="Workshop", topic="Making things", preset="private_chat",
                          invite=[enzo], creation_content=space)
        general = create(hdr, name="general", topic="Chatter", preset="private_chat", invite=[enzo])
        random_ = create(hdr, name="random", preset="private_chat")
        for child in (general, random_):
            link(hdr, workshop, child)
        for room in (workshop, general):
            http("POST", f"{path(room)}/join", headers=enzo_hdr, json={})
        http("PUT", f"{path(general)}/send/m.room.message/hello", headers=hdr,
             json={"msgtype": "m.text", "body": "Welcome to the workshop"})

        library = create(enzo_hdr, name="Library", preset="private_chat", invite=[me], creation_content=space)
        reading = create(enzo_hdr, name="reading-room", preset="private_chat", invite=[me])
        link(enzo_hdr, library, reading)
        leaf = http("POST", f"{hs}/_matrix/media/v3/upload?filename=leaf.png", data=png(CYAN, 64, 64),
                    headers={**enzo_hdr, "Content-Type": "image/png"}).json()["content_uri"]
        put_state(enzo_hdr, library, "m.room.image_pack", "", {
            "pack": {"display_name": "Library emoji"}, "images": {"leaf": {"url": leaf}}})
        for room in (library, reading):
            http("POST", f"{path(room)}/join", headers=hdr, json={})

        def state(room, kind, key=""):
            """A state event's content as Enzo, another member, reads it."""
            r = requests.get(f"{path(room)}/state/{kind}/{quote(key, safe='')}",
                                           headers=enzo_hdr, timeout=10)
            return r.json() if r.status_code == 200 else None

        def media(mxc):
            server, media_id = mxc.removeprefix("mxc://").split("/", 1)
            return http("GET", f"{hs}/_matrix/client/v1/media/download/{server}/{media_id}",
                        headers=enzo_hdr).content

        files = tmp / "files"
        files.mkdir()
        pictures = {
            "icon": (files / "icon.png", png(ORANGE, 256, 256)),
            "banner": (files / "banner.png", png(BLUE, 960, 540)),
            "blobwave": (files / "blobwave.png", png(GREEN, 64, 64)),
        }
        for file, data in pictures.values():
            file.write_bytes(data)
        print(f"Synapse at {hs}: {me} owns Workshop (general, random; no pack) with {enzo}; "
              f"{enzo}'s Library has :leaf:", flush=True)
        rooms = dict(workshop=workshop, general=general, library=library)
        look(tmp, args.shots, rooms, state, media, pictures)
        print("PASS: Server Settings changes a space's name, topic, icon and banner and a channel's name and "
              "topic as other members read them; makes, fills, renames and empties the emoji pack the "
              "pickers follow; and shows a space this member can't change read-only", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, rooms, state, media, pictures):
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
        client = wait_for("client window", lambda: kde.window("Untethered"), 60)
        kde.position(client, *WINDOW)

        def grab():
            return kde.capture().crop(WINDOW).convert("RGB")

        def shot(name):
            im = grab()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"settings-{name}.png")
            return im

        def words(rect=WINDOW):
            """Cleaned words in `rect`, grey ones too (read at four times
            their size)."""
            x0, y0, x1, y1 = rect
            return [(clean(w), x, y) for w, x, y in read(grab(), sidebar=True, dialogs=False)
                    if x0 <= x < x1 and y0 <= y < y1]

        def exact(text, rect=WINDOW):
            return next(((x, y) for w, x, y in words(rect) if w == text), None)

        def phrase(*parts, rect=WINDOW, gap=200):
            """Where `parts` read in order on one line."""
            ws = words(rect)
            for w, x, y in ws:
                if w != parts[0]:
                    continue
                at, ok = x, True
                for part in parts[1:]:
                    nxt = [(vx, vy) for v, vx, vy in ws if v == part and abs(vy - y) < 9 and 0 < vx - at < gap]
                    if not nxt:
                        ok = False
                        break
                    at = min(nxt)[0]
                if ok:
                    return x, y
            return None

        def away():
            kde.actions("pointer", [{"type": "pointerMove", "x": 760, "y": 640}])
            time.sleep(0.4)

        def chord(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def blobs(colour, rect, minimum=40):
            """Centres of the patches of `colour` in `rect`, top to bottom."""
            px = grab().load()
            pts = [(x, y) for y in range(rect[1], rect[3], 2) for x in range(rect[0], rect[2], 2)
                   if near(px[x, y], colour)]
            groups = []
            for x, y in pts:
                for g in groups:
                    if abs(g[-1][0] - x) < 40 and abs(g[-1][1] - y) < 40:
                        g.append((x, y))
                        break
                else:
                    groups.append([(x, y)])
            out = [(sum(p[0] for p in g) // len(g), sum(p[1] for p in g) // len(g)) for g in groups
                   if len(g) * 4 >= minimum]
            return sorted(out, key=lambda p: p[1])

        def replace_text(at, value):
            """Click into a text field at `at`, select what's there, type."""
            tap(kde, *at)
            time.sleep(0.3)
            kde.select_all()
            kde.type(value)
            time.sleep(0.3)

        def open_space(*marks):
            """Tap the rail's spaces until the sidebar shows `marks`."""
            def showing():
                ws = {w for w, _, _ in words(SIDEBAR)}
                return all(m in ws for m in marks)
            for i in range(4):
                tap(kde, RAIL_X, FIRST_SPACE + i * RAIL_STEP)
                away()
                try:
                    return wait_for(f"the space with {', '.join(marks)}", showing, 8)
                except RuntimeError:
                    continue
            shot("no-space")
            raise RuntimeError(f"no space shows {marks}")

        def open_settings():
            """Server Settings from the server menu."""
            def item():
                return phrase("server", "settings", rect=(72, TITLE_BAR + 48, 330, TITLE_BAR + 320))
            if not item():
                tap(kde, 72 + 80, TITLE_BAR + 24)
            tap(kde, *wait_for("Server Settings in the server menu", item, 15))
            wait_for("the settings page", lambda: phrase("server", "overview", rect=PAGE), 15)
            away()

        def tab(name):
            tap(kde, *wait_for(f"the {name} tab", lambda: exact(name, NAV), 10))
            away()

        def close_settings():
            chord(ESCAPE)
            wait_for("the settings page closed", lambda: not exact("esc", PAGE), 10)

        def save():
            tap(kde, *wait_for("Save Changes", lambda: phrase("save", "changes", rect=PAGE), 10))
            away()

        def pick(button, title, file):
            """Press `button` (its words), pick `file` in KDE's dialog."""
            tap(kde, *wait_for(f"{' '.join(button)}", lambda: phrase(*button, rect=PAGE), 10))
            dialog = wait_for(f"the {title!r} dialog", lambda: kde.window(title), 30)
            kde.command("kdotool", "windowactivate", dialog)
            time.sleep(1)
            kde.select_all()
            kde.type(str(file))
            chord(ENTER)
            wait_for("the dialog closed", lambda: not kde.window(title), 15)
            away()

        def pack():
            return (state(rooms["workshop"], "m.room.image_pack") or {}).get("images")

        open_space("general", "random")
        shot("workshop")
        toolbar = Ui(lambda: [(w, x, y) for w, x, y in words()], lambda p: kde.actions(
            "pointer", [{"type": "pointerMove", "x": p[0], "y": p[1]}]), grab,
            width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))

        # Overview: name, topic, icon, banner.
        open_settings()
        shot("overview")
        replace_text(wait_for("the name field", lambda: exact("workshop", PAGE), 10), "Makerspace")
        replace_text(wait_for("the topic field", lambda: exact("making", PAGE), 10), "Tools and projects")
        try:
            save()
            wait_for("the name and topic saved", lambda: (state(rooms["workshop"], "m.room.name") or {})
                     .get("name") == "Makerspace" and (state(rooms["workshop"], "m.room.topic") or {})
                     .get("topic") == "Tools and projects", 20)
        finally:
            shot("overview-saved")
        print("Matrix: Enzo reads the new name and topic from Workshop's m.room.name and m.room.topic", flush=True)

        icon_file, icon_bytes = pictures["icon"]
        pick(("upload", "image"), "Choose a server icon", icon_file)
        avatar = wait_for("the icon saved", lambda: (state(rooms["workshop"], "m.room.avatar") or {}).get("url"), 30)
        assert media(avatar) == icon_bytes, "the icon's upload differs from the picked file"
        banner_file, banner_bytes = pictures["banner"]
        pick(("upload", "banner"), "Choose a server banner", banner_file)
        banner = wait_for("the banner saved", lambda: (state(rooms["workshop"], BANNER_EVENT) or {}).get("url"), 30)
        assert media(banner) == banner_bytes, "the banner's upload differs from the picked file"
        info = state(rooms["workshop"], BANNER_EVENT)["info"]
        assert (info["w"], info["h"], info["mimetype"]) == (960, 540, "image/png"), info
        try:
            wait_for("the banner's preview", lambda: blobs(BLUE, PAGE, 400), 20)
        finally:
            shot("overview-pictures")
        print("Matrix: the icon (m.room.avatar) and the banner (" + BANNER_EVENT + ", with its size) are "
              "the picked files byte for byte", flush=True)

        # Channels: general's name and topic.
        tab("channels")
        replace_text(wait_for("general's name field", lambda: exact("general", PAGE), 10), "lobby")
        replace_text(wait_for("general's topic field", lambda: exact("chatter", PAGE), 10), "Say hello here")
        shot("channels")
        save()
        wait_for("general renamed", lambda: (state(rooms["general"], "m.room.name") or {}).get("name") == "lobby"
                 and (state(rooms["general"], "m.room.topic") or {}).get("topic") == "Say hello here", 20)
        print("Matrix: general is now lobby, with the topic Say hello here", flush=True)
        close_settings()
        try:
            wait_for("the new names in the sidebar", lambda: exact("lobby", SIDEBAR) and exact("makerspace", WINDOW), 30)
            wait_for("the banner over the channel list", lambda: blobs(BLUE, (72, TITLE_BAR, 312, TITLE_BAR + 135), 400), 30)
        finally:
            shot("sidebar")
        print("UI: the sidebar shows Makerspace over its blue banner, and lobby", flush=True)

        # Emoji: create the pack, upload, use as sticker, rename, delete.
        open_settings()
        tab("emoji")
        tap(kde, *wait_for("Create Emoji Pack", lambda: phrase("create", "emoji", rect=PAGE), 15))
        made = wait_for("the pack created", lambda: state(rooms["workshop"], "m.room.image_pack"), 20)
        assert made["pack"]["display_name"] == "Makerspace" and made["images"] == {}, made
        blob_file, _ = pictures["blobwave"]
        pick(("upload", "emoji"), "Upload an emoji", blob_file)
        image = wait_for(":blobwave: in the pack", lambda: (pack() or {}).get("blobwave"), 30)
        assert image["usage"] == ["emoticon"] and image["url"].startswith("mxc://"), image
        assert (image["info"]["w"], image["info"]["h"]) == (64, 64), image
        try:
            wait_for(":blobwave: listed", lambda: exact("blobwave", PAGE) and blobs(GREEN, PAGE), 20)
        finally:
            shot("emoji-uploaded")
        print("Matrix: Create Emoji Pack wrote m.room.image_pack; :blobwave: went into it as an emoticon",
              flush=True)

        def to_lobby():
            close_settings()
            tap(kde, *wait_for("lobby in the sidebar", lambda: exact("lobby", SIDEBAR), 15))
            wait_for("lobby open", lambda: exact("lobby", (330, TITLE_BAR, 900, TITLE_BAR + 48)), 15)
            time.sleep(1)

        to_lobby()
        send = toolbar.send()
        sticker_button, emoji_button = (send[0] - 46, send[1]), (send[0] - 92, send[1])
        tap(kde, *emoji_button)
        try:
            wait_for("Makerspace's emoji in the picker", lambda: exact("makerspace", (330, 200, 1180, 840))
                     and blobs(GREEN, (330, 200, 1180, 840)), 30)
        finally:
            shot("picker-blobwave")
        chord(ESCAPE)
        print("UI: the emoji picker offers :blobwave: under Makerspace", flush=True)

        open_settings()
        tab("emoji")
        row = wait_for(":blobwave:'s row", lambda: exact("blobwave", PAGE), 15)
        tap(kde, *wait_for("Both", lambda: next(((x, y) for w, x, y in words(PAGE)
                                                 if w == "both" and abs(y - row[1]) < 12), None), 10))
        wait_for(":blobwave: as both", lambda: (pack() or {}).get("blobwave", {}).get("usage")
                 == ["emoticon", "sticker"], 20)
        to_lobby()
        tap(kde, *sticker_button)
        try:
            wait_for(":blobwave: in the sticker picker", lambda: exact("stickers", WINDOW)
                     and blobs(GREEN, (330, 200, 1180, 840)), 30)
        finally:
            shot("stickers-blobwave")
        chord(ESCAPE)
        print("Matrix and UI: offered as both, :blobwave: is in the sticker picker too", flush=True)

        open_settings()
        tab("emoji")
        tap(kde, *wait_for(":blobwave:'s name", lambda: exact("blobwave", PAGE), 15))
        time.sleep(0.5)
        kde.select_all()
        kde.type("wave")
        chord(ENTER)
        wait_for(":wave: renamed", lambda: set(pack() or {}) == {"wave"}, 20)
        assert pack()["wave"]["url"] == image["url"], pack()
        to_lobby()
        tap(kde, *toolbar.composer())
        kde.type(":wav")
        try:
            wait_for(":wave: in autocomplete", lambda: exact("wave", WINDOW) and blobs(GREEN, WINDOW), 20)
        finally:
            shot("autocomplete-wave")
        kde.select_all()
        chord(BACKSPACE)
        chord(ESCAPE)
        print("Matrix and UI: renamed :wave: (same picture), it completes from :wav", flush=True)

        open_settings()
        tab("emoji")
        row = wait_for(":wave:'s row", lambda: exact("wave", PAGE), 15)
        both = wait_for("its usage", lambda: next(((x, y) for w, x, y in words(PAGE)
                                                    if w == "both" and abs(y - row[1]) < 12), None), 10)
        # The Delete button: the grey cross right of the usage control.
        px = grab().load()
        cross = [(x, y) for x in range(both[0] + 30, PAGE[2] - 70) for y in range(row[1] - 10, row[1] + 10)
                 if near(px[x, y], ICON_GREY, 20)]
        assert cross, "no Delete button on :wave:'s row"
        tap(kde, sum(p[0] for p in cross) // len(cross), sum(p[1] for p in cross) // len(cross))
        wait_for(":wave: deleted", lambda: pack() == {}, 20)
        shot("emoji-deleted")
        to_lobby()
        tap(kde, *emoji_button)
        wait_for("the emoji picker", lambda: exact("emoji", (330, 200, 1180, 840)) or phrase("frequently", rect=WINDOW), 15)
        time.sleep(2)
        try:
            assert not exact("makerspace", (330, 200, 1180, 840)) and not blobs(GREEN, (330, 200, 1180, 840)), \
                "the deleted emoji is still offered"
        finally:
            shot("picker-empty")
        chord(ESCAPE)
        print("Matrix and UI: deleted, the pack is empty and the picker has no Makerspace emoji", flush=True)

        # Library: Enzo's space, read-only here.
        open_space("reading-room")
        open_settings()
        try:
            wait_for("the read-only notice", lambda: phrase("you", "can", "view", rect=PAGE), 15)
            assert not phrase("upload", "image", rect=PAGE) and not phrase("upload", "banner", rect=PAGE)
            # Typing at the name changes nothing.
            tap(kde, *wait_for("Library's name", lambda: exact("library", PAGE), 10))
            kde.type("Hacked")
            time.sleep(1)
            assert not phrase("save", "changes", rect=PAGE), "a read-only page offered Save Changes"
        finally:
            shot("library-overview")
        tab("emoji")
        try:
            wait_for(":leaf: listed", lambda: exact("leaf", PAGE) and blobs(CYAN, PAGE), 20)
            assert not phrase("upload", "emoji", rect=PAGE) and not exact("both", PAGE), "Library's pack is editable"
        finally:
            shot("library-emoji")
        tab("channels")
        wait_for("reading-room listed", lambda: exact("reading-room", PAGE), 10)
        close_settings()
        assert state(rooms["library"], "m.room.name")["name"] == "Library"
        print("UI: Library's settings are read-only for a member: no uploads, fields or Save, and its pack "
              "lists :leaf: without controls", flush=True)


if __name__ == "__main__":
    main()
