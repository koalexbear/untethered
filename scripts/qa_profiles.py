"""Profile customization (ROADMAP M15), live against a disposable Synapse.

Build the debug client (it seeds the profiles) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_profiles.py

The test account, Quinn, has an orange profile picture and runs a space,
Guild, with a channel, general; Quinn also shares lobby, a room in no space,
with Rowan, who signs in on a second Untethered profile in the same private
KWin session (as qa_kwin.py needs). Quinn's client must:
- set a server picture for Guild from the server menu's Edit Server
  Profile, an animated green-and-purple GIF picked in KDE's file dialog:
  Quinn's member events in Guild and general carry it (the upload byte for
  byte) and lobby's keeps the orange one, as Element reads them; it plays
  next to Quinn's messages in general, and lobby shows the orange one;
- give news, a channel Rowan adds to Guild that Quinn joins afterwards, the
  server picture too;
- in User Settings → My Account, say the profile card is Untethered-only,
  and save an animated cyan-and-pink banner, an About Me and an accent
  colour as extended profile fields: the homeserver's profile has them;
- reset the server picture from Edit Server Profile, which puts the orange
  one back in every room of Guild.
Rowan's client must show Quinn's server picture playing in general and the
orange one in lobby, and Quinn's card with the About Me, the banner playing
and the server picture. A homeserver without profile fields hides the
editors; Synapse always offers them, so unit tests cover that
(profile_fields.rs).
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build. From a worktree, point UNTETHERED_BACKEND at the main
checkout's `.dev/backends/…/gomuks` (it only prepares the accounts).
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
from qa_matrix_sdk import SERVER_NAME, http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, read  # noqa: E402

TITLE_BAR = 28
# Quinn's window, and Rowan's beside it on the 2100 px wide screen.
QUINN = (0, 0, 1180, 900)
ROWAN = (1190, 0, 2100, 900)
RAIL = 72
SIDE = 258
FIRST_SPACE_Y = 186
ALL_ROOMS_Y = 120
ENTER = "\ue007"
ESCAPE = "\ue00c"

ORANGE = (250, 140, 20)
GREEN = (40, 220, 60)
PURPLE = (160, 50, 220)
CYAN = (20, 220, 230)
PINK = (240, 80, 200)
# The second accent colour on offer (profile_fields.rs ACCENTS).
ACCENT = 0x3CCFAE
ACCENT_RGB = (0x3C, 0xCF, 0xAE)
BIO = "Plays the cello in the guild band"
KEYS = {key: "io.github.koalexbear.untethered." + key for key in ("banner", "bio", "accent")}
HELLO = "Hello from the guild hall"
LOBBY = "Hello from the lobby"


def near(pixel, colour, slack=36):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def png(colour, side):
    out = io.BytesIO()
    Image.new("RGB", (side, side), colour).save(out, "PNG")
    return out.getvalue()


def gif(colours, w, h):
    """An animated GIF showing each colour in turn, forever."""
    frames = [Image.new("RGB", (w, h), c) for c in colours]
    out = io.BytesIO()
    frames[0].save(out, "GIF", save_all=True, append_images=frames[1:], duration=350, loop=0)
    return out.getvalue()


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-profiles-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        api = hs + "/_matrix/client/v3"
        _, hdr = interop().seed(tmp, hs, encrypted=False, room=False)
        (tmp / "rowan").mkdir()
        _, rowan_hdr = interop().seed(tmp / "rowan", hs, encrypted=False, room=False)
        me = http("GET", api + "/account/whoami", headers=hdr).json()["user_id"]
        rowan = http("GET", api + "/account/whoami", headers=rowan_hdr).json()["user_id"]

        def upload(headers, data, name, kind):
            return http("POST", f"{hs}/_matrix/media/v3/upload?filename={name}", data=data,
                        headers={**headers, "Content-Type": kind}).json()["content_uri"]

        def profile(user, key, value):
            http("PUT", f"{api}/profile/{quote(user, safe='')}/{key}", headers=hdr if user == me else rowan_hdr,
                 json={key: value})

        orange = upload(hdr, png(ORANGE, 128), "orange.png", "image/png")
        profile(me, "displayname", "Quinn")
        profile(me, "avatar_url", orange)
        profile(rowan, "displayname", "Rowan")

        def path(room):
            return f"{api}/rooms/{quote(room, safe='')}"

        def create(headers, **body):
            return http("POST", api + "/createRoom", headers=headers, json=body).json()["room_id"]

        def say(headers, room, body):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers,
                 json={"msgtype": "m.text", "body": body})

        space = create(hdr, name="Guild", preset="private_chat", invite=[rowan], creation_content={"type": "m.space"})
        general = create(hdr, name="general", preset="private_chat", invite=[rowan])
        lobby = create(hdr, name="lobby", preset="private_chat", invite=[rowan])
        http("PUT", f"{path(space)}/state/m.space.child/{quote(general, safe='')}", headers=hdr,
             json={"via": [SERVER_NAME]})
        for room in (space, general, lobby):
            http("POST", f"{path(room)}/join", headers=rowan_hdr, json={})
        say(hdr, general, HELLO)
        say(hdr, lobby, LOBBY)
        rooms = dict(space=space, general=general, lobby=lobby)

        def member(room):
            """Quinn's member event content in `room`, as Rowan (or Element) reads it."""
            return http("GET", f"{path(room)}/state/m.room.member/{quote(me, safe='')}", headers=rowan_hdr).json()

        def media(mxc):
            server, media_id = mxc.removeprefix("mxc://").split("/", 1)
            return http("GET", f"{hs}/_matrix/client/v1/media/download/{server}/{media_id}",
                        headers=rowan_hdr).content

        def fields():
            """Quinn's whole global profile, as Rowan reads it."""
            return http("GET", f"{api}/profile/{quote(me, safe='')}", headers=rowan_hdr).json()

        def add_news():
            news = create(rowan_hdr, name="news", preset="private_chat", invite=[me])
            http("POST", f"{path(news)}/join", headers=hdr, json={})
            # Linked once Quinn is in: from another device of Quinn's, the
            # client sees a new channel of Guild that still shows the global
            # picture. Rowan may link it: Quinn's power is in Guild.
            power = http("GET", f"{path(space)}/state/m.room.power_levels", headers=hdr).json()
            power.setdefault("users", {})[rowan] = 100
            http("PUT", f"{path(space)}/state/m.room.power_levels", headers=hdr, json=power)
            http("PUT", f"{path(space)}/state/m.space.child/{quote(news, safe='')}", headers=rowan_hdr,
                 json={"via": [SERVER_NAME]})
            rooms["news"] = news
            return news

        files = tmp / "files"
        files.mkdir()
        pictures = {
            "server": (files / "server.gif", gif([GREEN, PURPLE], 128, 128)),
            "banner": (files / "banner.gif", gif([CYAN, PINK], 600, 240)),
        }
        for file, data in pictures.values():
            file.write_bytes(data)
        print(f"Synapse at {hs}: {me} (Quinn, orange picture) runs Guild with general and shares lobby with "
              f"{rowan} (Rowan) on a second Untethered", flush=True)
        world = dict(tmp=tmp, rooms=rooms, member=member, media=media, fields=fields, add_news=add_news,
                     pictures=pictures, orange=orange)
        look(args.shots, world)
        print("PASS: a server picture set for Guild shows only there, plays, reaches a channel joined later and "
              "resets; a banner, About Me and accent colour saved in My Account show on Quinn's card in "
              "Rowan's Untethered", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(shots, world):
    from qa_kwin import KWin
    from qa_native_paste import child, isolated_env, wait_for
    tmp, rooms, member, media = world["tmp"], world["rooms"], world["member"], world["media"]
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off")
        kde = KWin(stack, ui, ROOT, None, env=env)
        quinn = wait_for("Quinn's window", lambda: kde.window("Untethered"), 60)
        kde.position(quinn, QUINN[0], QUINN[1], QUINN[2] - QUINN[0], QUINN[3] - QUINN[1])
        # Rowan's Untethered: the same build on Rowan's profile.
        stack.enter_context(child([str(release)], {**kde.env, "UNTETHERED_PROFILE": str(tmp / "rowan" / "seed")}))

        def other():
            r = kde.command("kdotool", "search", "--title", "Untethered", check=False)
            return next((w for w in r.stdout.split() if w != quinn), None)

        rowan_win = wait_for("Rowan's window", other, 60)
        kde.position(rowan_win, ROWAN[0], ROWAN[1], ROWAN[2] - ROWAN[0], ROWAN[3] - ROWAN[1])

        def grab():
            return kde.capture().convert("RGB")

        def shot(name):
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                grab().crop((0, 0, ROWAN[2], ROWAN[3])).save(shots / f"profiles-{name}.png")

        class Window:
            """One client's window: where its parts are, and its words."""

            def __init__(self, box):
                self.box = box
                self.x, self.top = box[0], box[1] + TITLE_BAR
                self.sidebar = (self.x + RAIL, self.top, self.x + RAIL + SIDE, box[3])
                self.main = (self.x + RAIL + SIDE, self.top, box[2], box[3])
                self.ui = Ui(lambda: self.words(), lambda p: kde.actions(
                    "pointer", [{"type": "pointerMove", "x": p[0], "y": p[1]}]), grab,
                    width=box[2] - box[0], height=box[3] - self.top, origin=(self.x, self.top))

            def words(self, rect=None):
                """Cleaned words in `rect` (the window by default), grey ones
                too, read at four times their size."""
                x0, y0, x1, y1 = rect or self.box
                return [(clean(w), x, y) for w, x, y in read(grab(), sidebar=True, dialogs=False)
                        if x0 <= x < x1 and y0 <= y < y1]

            def exact(self, text, rect=None):
                return next(((x, y) for w, x, y in self.words(rect) if w == text), None)

            def phrase(self, *parts, rect=None, gap=200):
                """Where `parts` read in order on one line."""
                ws = self.words(rect)
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

            def away(self):
                """Move the pointer off the rail, whose tooltips cover the sidebar."""
                kde.actions("pointer", [{"type": "pointerMove", "x": self.main[0] + 300, "y": self.box[1] + 640}])
                time.sleep(0.4)

            def open_room(self, name, space):
                """Open `name` in Guild (`space`) or in All Rooms."""
                def section():
                    head = self.words((self.sidebar[0], self.top, self.sidebar[2], self.top + 48))
                    return ("guild" in {w for w, _, _ in head}) == space

                def opened():
                    if not section():
                        tap(kde, self.x + RAIL // 2, self.box[1] + (FIRST_SPACE_Y if space else ALL_ROOMS_Y))
                        self.away()
                        time.sleep(1)
                        return None
                    if self.exact(name, (self.main[0], self.top, self.box[2], self.top + 48)):
                        return True
                    at = self.exact(name, self.sidebar)
                    if at:
                        tap(kde, *at)
                        self.away()
                    return None
                try:
                    wait_for(f"{name} open", opened, 60)
                except RuntimeError:
                    shot(f"missing-{name}")
                    raise

        q, r = Window(QUINN), Window(ROWAN)

        def colours(rect, *wanted, slack=36):
            """How many sampled pixels in `rect` are near each colour."""
            px = grab().load()
            counts = [0] * len(wanted)
            for y in range(rect[1], rect[3], 2):
                for x in range(rect[0], rect[2], 2):
                    for i, c in enumerate(wanted):
                        if near(px[x, y], c, slack):
                            counts[i] += 1
            return counts

        def plays(label, rect, first, second, minimum=30, timeout=20):
            """Both colours of an animated picture show in `rect`, one
            capture or another apart: it plays."""
            seen = [False, False]

            def both():
                a, b = colours(rect, first, second)
                seen[0] |= a >= minimum
                seen[1] |= b >= minimum
                return all(seen)
            try:
                wait_for(label, both, timeout)
            except RuntimeError:
                shot("still-" + label.replace(" ", "-")[:40])
                raise

        def chord(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def pick(win, button, title, file):
            """Press `button` (its words), pick `file` in KDE's dialog."""
            tap(kde, *wait_for(" ".join(button), lambda: win.phrase(*button), 15))
            dialog = wait_for(f"the {title!r} dialog", lambda: kde.window(title), 30)
            kde.command("kdotool", "windowactivate", dialog)
            time.sleep(1)
            kde.select_all()
            kde.type(str(file))
            time.sleep(0.8)
            # The name field's completion can take the first Enter.
            for _ in range(3):
                chord(ENTER)
                try:
                    wait_for("the dialog closed", lambda: not kde.window(title), 5)
                    break
                except RuntimeError:
                    continue
            else:
                shot("file-dialog")
                raise RuntimeError(f"the {title!r} dialog stayed open")
            win.away()

        def dialog():
            return q.phrase("edit", "server", "profile") and q.phrase("server", "picture")

        def edit_server_profile():
            tap(kde, *wait_for("Edit Server Profile in the server menu",
                               lambda: q.ui.space_menu("Edit Server Profile", lambda p: tap(kde, *p)), 20))
            try:
                wait_for("Edit Server Profile", dialog, 10)
            finally:
                shot("edit-server-profile")

        def stored(room):
            return member(rooms[room]).get("avatar_url")

        def everywhere(expected, label):
            for key in ("space", "general", "news"):
                if key in rooms:
                    wait_for(f"{label} in {key} on the homeserver", lambda k=key: stored(k) == expected, 60)

        def steps():
            q.open_room("general", True)
            wait_for("Quinn's message in general", lambda: q.exact("guild", q.main), 60)
            wait_for("Quinn's orange picture in general", lambda: colours(q.main, ORANGE)[0] >= 60, 30)
            shot("orange-general")
            print("UI: Quinn's message in general shows the orange profile picture", flush=True)

            # A server picture, from Edit Server Profile.
            edit_server_profile()
            pick(q, ("change", "picture"), "Choose a server picture", world["pictures"]["server"][0])
            dialog_box = (q.x + 300, q.top + 150, q.box[2] - 300, q.box[3] - 150)
            wait_for("the picked picture in the dialog", lambda: colours(dialog_box, GREEN)[0] >= 40, 15)
            shot("server-picture-picked")
            tap(kde, *wait_for("Save Changes", lambda: q.phrase("save", "changes"), 10))
            wait_for("the dialog closed", lambda: not dialog(), 60)
            server = stored("space")
            assert server and server != world["orange"], member(rooms["space"])
            assert media(server) == world["pictures"]["server"][1], "the server picture changed on upload"
            everywhere(server, "the server picture")
            assert stored("lobby") == world["orange"], member(rooms["lobby"])
            print(f"Matrix: Quinn's member events carry the server picture {server} in Guild and general "
                  "(byte for byte) and the orange one in lobby, as Element reads them", flush=True)
            plays("the server picture playing in general", q.main, GREEN, PURPLE)
            assert colours(q.main, ORANGE)[0] < 20, "the orange picture still shows in general"
            shot("server-picture-general")
            q.open_room("lobby", False)
            wait_for("Quinn's lobby message", lambda: q.exact("lobby", q.main), 60)
            wait_for("the orange picture in lobby", lambda: colours(q.main, ORANGE)[0] >= 60, 30)
            assert colours(q.main, GREEN, PURPLE) == [0, 0] or max(colours(q.main, GREEN, PURPLE)) < 10, \
                "the server picture shows in lobby"
            shot("orange-lobby")
            print("UI: Quinn's server picture plays next to Quinn's message in general; lobby shows the orange "
                  "one", flush=True)

            # Rowan's Untethered sees the same.
            tap(kde, r.main[0] + 200, r.box[1] + 400)  # a click lets the new window take keys
            r.open_room("general", True)
            wait_for("Quinn's message in Rowan's general", lambda: r.exact("guild", r.main), 60)
            plays("Quinn's server picture in Rowan's general", r.main, GREEN, PURPLE, timeout=60)
            r.open_room("lobby", False)
            wait_for("Quinn's message in Rowan's lobby", lambda: r.exact("lobby", r.main), 60)
            wait_for("the orange picture in Rowan's lobby", lambda: colours(r.main, ORANGE)[0] >= 60, 30)
            shot("rowan-sees")
            print("UI: Rowan's Untethered shows Quinn's server picture playing in general and the orange one in "
                  "lobby", flush=True)

            # A channel joined after the server picture was set takes it.
            news = world["add_news"]()
            wait_for("the server picture in news", lambda: stored("news") == server, 90)
            print("Matrix: news, added to Guild by Rowan and joined afterwards, took the server picture too",
                  flush=True)

            # The profile card's fields, in My Account.
            # Your name at the foot of the sidebar opens the status menu.
            tap(kde, *q.ui.settings())
            tap(kde, *wait_for("User Settings in the status menu", lambda: q.phrase("user", "settings"), 10))
            wait_for("My Account", lambda: q.phrase("my", "account"), 15)
            q.away()
            try:
                wait_for("the profile card editors", lambda: q.phrase("untethered", "only")
                         and q.phrase("change", "banner"), 20)
            finally:
                shot("my-account")
            print("UI: My Account offers a banner, accent colour and About Me, marked Untethered only", flush=True)
            pick(q, ("change", "banner"), "Choose a profile banner", world["pictures"]["banner"][0])
            wait_for("the banner saved", lambda: (world["fields"]().get(KEYS["banner"]) or "").startswith("mxc://"), 60)
            banner = world["fields"]()[KEYS["banner"]]
            assert media(banner) == world["pictures"]["banner"][1], "the banner changed on upload"
            plays("the banner playing in My Account", q.box, CYAN, PINK)
            im = grab()
            spots = [(x, y) for y in range(q.top + 100, q.box[3] - 40, 3) for x in range(q.x + 330, q.box[2] - 20, 3)
                     if near(im.getpixel((x, y)), ACCENT_RGB, 10)]
            assert spots, "no accent swatch on the page"
            sx = sorted(p[0] for p in spots)[len(spots) // 2]
            sy = sorted(p[1] for p in spots)[len(spots) // 2]
            tap(kde, sx, sy)
            q.away()
            wait_for("the accent saved", lambda: world["fields"]().get(KEYS["accent"]) == "#3ccfae", 30)
            # The About Me editor is near the foot of the page: scroll to it.
            kde.actions("wheel", [{"type": "scroll", "x": 760, "y": 600, "deltaX": 0, "deltaY": 400}])
            kde.actions("pointer", [{"type": "pointerMove", "x": 762, "y": 602}])
            time.sleep(0.8)

            def about_label():
                """The ABOUT ME label over the editor: the lowest "about me"."""
                ws = q.words()
                found = [(x, y) for w, x, y in ws if w == "about" and any(
                    v == "me" and abs(vy - y) < 9 and 0 < vx - x < 80 for v, vx, vy in ws)]
                low = max(found, key=lambda p: p[1], default=None)
                return low if low and q.phrase("save", "about") else None
            label_at = wait_for("the About Me editor", about_label, 15)
            tap(kde, label_at[0] + 60, label_at[1] + 40)
            time.sleep(0.4)
            kde.type(BIO)
            time.sleep(0.4)
            tap(kde, *wait_for("Save About Me", lambda: q.phrase("save", "about"), 10))
            q.away()
            wait_for("the About Me saved", lambda: world["fields"]().get(KEYS["bio"]) == BIO, 30)
            shot("my-account-saved")
            saved = {k: v for k, v in world["fields"]().items() if k in KEYS.values()}
            print(f"Matrix: Quinn's profile has {saved}", flush=True)
            chord(ESCAPE)
            wait_for("back from My Account", lambda: not q.phrase("my", "account"), 10)

            # Quinn's card in Rowan's Untethered.
            r.open_room("general", True)
            at = wait_for("Quinn's name in Rowan's general", lambda: r.exact("quinn", r.main), 30)
            tap(kde, *at)
            try:
                wait_for("the About Me on Quinn's card", lambda: r.phrase("cello", "in", "the"), 20)
                plays("the banner playing on Quinn's card", r.main, CYAN, PINK, timeout=40)
                plays("the server picture on Quinn's card", r.main, GREEN, PURPLE)
            finally:
                shot("rowan-card")
            print("UI: Quinn's card in Rowan's Untethered shows the About Me, the banner playing and the server "
                  "picture", flush=True)
            chord(ESCAPE)

            # Reset the server picture.
            tap(kde, q.main[0] + 200, q.box[1] + 400)
            q.open_room("general", True)
            edit_server_profile()
            tap(kde, *wait_for("Reset Picture", lambda: q.phrase("reset", "picture"), 10))
            wait_for("the dialog closed", lambda: not dialog(), 60)
            everywhere(world["orange"], "the orange picture")
            wait_for("the orange picture back in general", lambda: colours(q.main, ORANGE)[0] >= 60, 30)
            shot("reset-general")
            print("Matrix and UI: Reset Picture put the orange picture back in Guild, general and news", flush=True)

        try:
            steps()
        except Exception:
            shot("failure")
            raise


if __name__ == "__main__":
    main()
