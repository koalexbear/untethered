"""Roles and channel permissions (ROADMAP C2), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_roles.py

Enzo creates a space, Workshop, in room version 12 (so he is its Owner,
above every level), with the channels general, announcements and vault.
The test account, Quinn, is Admin (100) in Workshop, general and
announcements, and an ordinary member of vault. Mira and Nora are members.
The release client in a private KWin session (as qa_kwin.py needs) opens
Server Settings' Roles tab and must:
- show Enzo as Owner, and make a custom role, Helper, at power level 25 in
  orange: Untethered's own `io.github.koalexbear.untethered.roles` event on
  Workshop holds it;
- make announcements an announcement channel: Send messages needs
  Moderator, Add reactions needs Helper. Its power levels change only there,
  with everything else kept (other users, an unknown event type and field);
  the page says where it differs from the server. Nora, an ordinary member,
  is refused posting and reacting there by the homeserver (403);
- give Mira Moderator: her level is 50 in Workshop, general and
  announcements, as Element reads it from each m.room.power_levels, and not
  in vault, which Quinn can't change. Now she posts in announcements;
- give Nora Helper: she may react in announcements now, and still not post;
- set the server's default for Invite people to Moderator: Workshop, general
  and announcements need 50 to invite, and the homeserver refuses Nora's
  invite while it takes Mira's;
- show Helper in orange and MOD as the badges on Nora's and Mira's
  messages in general;
- once Enzo makes Quinn an ordinary member again, show the Roles tab
  read-only, and announcements' composer as read-only for Quinn, whose
  message menu there offers no Add Reaction; once Enzo makes Quinn a Helper
  in announcements, it offers Add Reaction again (posting stays read-only).
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build. From a worktree, point UNTETHERED_BACKEND (the seed step) at
the main checkout's `.dev/backends/…/gomuks`.
"""
import argparse
import contextlib
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, line_words, read  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
RAIL_X = 36
FIRST_SPACE = 186
RAIL_STEP = 56
# Server Settings: the tabs left of this, the page right of it.
NAV = (0, 40, 400, 900)
PAGE = (400, 40, 1180, 900)
ESCAPE = "\ue00c"  # WebDriver's Escape key
ROLES_EVENT = "io.github.koalexbear.untethered.roles"
ORANGE = (0xe6, 0x7e, 0x22)
POSTS = ("m.room.message", "m.room.encrypted", "m.sticker", "m.poll.start", "org.matrix.msc3381.poll.start")
LIMIT = "{per_second: 1000, burst_count: 1000}"
EXTRA = (f"rc_invites: {{per_room: {LIMIT}, per_user: {LIMIT}, per_issuer: {LIMIT}}}\n"
         f"rc_registration: {LIMIT}\n")


def near(pixel, colour, slack=30):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-roles-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp, EXTRA)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        api = hs + "/_matrix/client/v3"
        me = http("GET", api + "/account/whoami", headers=hdr).json()["user_id"]
        accounts = {}
        for name in ("Enzo", "Mira", "Nora", "Zed"):
            user, _, headers = register(hs, name.lower() + secrets.token_hex(3))
            http("PUT", f"{api}/profile/{quote(user, safe='')}/displayname", headers=headers,
                 json={"displayname": name})
            accounts[name] = (user, headers)
        http("PUT", f"{api}/profile/{quote(me, safe='')}/displayname", headers=hdr, json={"displayname": "Quinn"})
        enzo, enzo_hdr = accounts["Enzo"]
        mira, mira_hdr = accounts["Mira"]
        nora, nora_hdr = accounts["Nora"]
        zed, _ = accounts["Zed"]

        def path(room):
            return f"{api}/rooms/{quote(room, safe='')}"

        def create(headers, **body):
            return http("POST", api + "/createRoom", headers=headers, json=body).json()["room_id"]

        def put_state(headers, room, kind, key, content):
            return http("PUT", f"{path(room)}/state/{kind}/{quote(key, safe='')}", headers=headers, json=content)

        def say(headers, room, body):
            http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers,
                 json={"msgtype": "m.text", "body": body})

        # Version 12: Enzo, the creator, has no power level and outranks
        # every one; Quinn is Admin except in vault.
        admin = {"users": {me: 100}}
        common = dict(preset="private_chat", room_version="12", invite=[me, mira, nora])
        workshop = create(enzo_hdr, name="Workshop", creation_content={"type": "m.space"},
                          power_level_content_override=admin, **common)
        general = create(enzo_hdr, name="general", power_level_content_override=admin, **common)
        announcements = create(enzo_hdr, name="announcements", power_level_content_override=admin, **common)
        vault = create(enzo_hdr, name="vault", **common)
        for child in (general, announcements, vault):
            put_state(enzo_hdr, workshop, "m.space.child", child, {"via": [SERVER_NAME]})
            put_state(enzo_hdr, child, "m.space.parent", workshop, {"via": [SERVER_NAME], "canonical": True})
        version = http("GET", f"{path(workshop)}/state/m.room.create", headers=enzo_hdr).json()["room_version"]
        assert version == "12", version
        # What the page doesn't edit must survive its changes.
        power = http("GET", f"{path(announcements)}/state/m.room.power_levels", headers=enzo_hdr).json()
        power["events"]["org.example.audit"] = 80
        power["org.example.policy"] = {"label": "Retain this extension"}
        put_state(enzo_hdr, announcements, "m.room.power_levels", "", power)
        for headers in (hdr, mira_hdr, nora_hdr):
            for room in (workshop, general, announcements, vault):
                http("POST", f"{path(room)}/join", headers=headers, json={})
        say(nora_hdr, general, "Hello from Nora")
        say(mira_hdr, general, "Mira checking in")
        say(enzo_hdr, announcements, "Welcome to the announcements")

        def state(room, kind, key="", headers=enzo_hdr):
            """A state event's content as Enzo reads it."""
            r = requests.get(f"{path(room)}/state/{kind}/{quote(key, safe='')}", headers=headers, timeout=10)
            return r.json() if r.status_code == 200 else None

        def levels(room):
            return state(room, "m.room.power_levels") or {}

        def send(headers, room, body):
            return requests.put(f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=headers,
                                json={"msgtype": "m.text", "body": body}, timeout=10)

        def react(headers, room):
            target = http("GET", f"{path(room)}/messages", headers=headers,
                          params={"dir": "b", "limit": 20}).json()["chunk"]
            event = next(e["event_id"] for e in target if e["type"] == "m.room.message")
            return requests.put(f"{path(room)}/send/m.reaction/{secrets.token_hex(8)}", headers=headers,
                                json={"m.relates_to": {"rel_type": "m.annotation", "event_id": event,
                                                       "key": "👍"}}, timeout=10)

        def invite(headers, room, user):
            return requests.post(f"{path(room)}/invite", headers=headers, json={"user_id": user}, timeout=10)

        def demote():
            """Enzo, the Owner, makes Quinn an ordinary member everywhere."""
            for room in (workshop, general, announcements):
                content = levels(room)
                content["users"].pop(me, None)
                put_state(enzo_hdr, room, "m.room.power_levels", "", content)

        def helper():
            """Enzo makes Quinn a Helper (25) in announcements."""
            content = levels(announcements)
            content["users"][me] = 25
            put_state(enzo_hdr, announcements, "m.room.power_levels", "", content)

        print(f"Synapse at {hs}: {enzo} owns Workshop (version 12) with general, announcements and vault; "
              f"{me} (Quinn) is Admin but in vault; {mira} and {nora} are members", flush=True)
        rooms = dict(workshop=workshop, general=general, announcements=announcements, vault=vault)
        people = dict(me=me, mira=mira, nora=nora, zed=zed)
        api_calls = dict(state=state, levels=levels, send=send, react=react, invite=invite, demote=demote, helper=helper,
                         mira=mira_hdr, nora=nora_hdr, power=power)
        look(tmp, args.shots, rooms, people, api_calls)
        print("PASS: the Roles tab names a custom role, gives roles across the space's rooms, sets an "
              "announcement channel and a server default the homeserver enforces on a second account, colours "
              "badges with the custom role, and is read-only once Quinn can't change power levels", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, rooms, people, calls):
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    state, levels = calls["state"], calls["levels"]
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
                im.save(shots / f"roles-{name}.png")
            return im

        def words(rect=WINDOW):
            """Cleaned words in `rect`, grey ones too (read at four times
            their size)."""
            x0, y0, x1, y1 = rect
            return [(clean(w), x, y) for w, x, y in read(grab(), sidebar=True, dialogs=False)
                    if x0 <= x < x1 and y0 <= y < y1]

        def exact(text, rect=WINDOW):
            """The topmost place `text` reads in `rect`."""
            found = [(y, x) for w, x, y in words(rect) if w == text]
            return (min(found)[1], min(found)[0]) if found else None

        def phrase(*parts, rect=WINDOW, gap=200):
            """Where `parts` read in order on one line."""
            ws = words(rect)
            for w, x, y in sorted(ws, key=lambda v: v[2]):
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

        def on_row(text, row, rect=PAGE, right_of=0):
            """`text` on the line of `row` (a (x, y)), right of `right_of`."""
            # A picker sits level with its label and description, which can
            # take two lines.
            return next(((x, y) for w, x, y in words(rect) if w == text and -10 < y - row[1] < 26 and x > right_of),
                        None)

        def entry(text, picker, first="member", last="admin"):
            """`text` in the menu a picker at `picker` opened, below it or
            above it where there's no room: between the menu's first and last
            entries, in the picker's column."""
            ws = [(w, x, y) for w, x, y in words(PAGE) if abs(x - picker[0]) < 120 and abs(y - picker[1]) > 8]
            for _, _, top in (v for v in ws if v[0] == first):
                for _, _, bottom in (v for v in ws if v[0] == last and 0 < v[2] - top < 200):
                    if top < picker[1] < bottom:
                        continue
                    found = [(x, y) for w, x, y in ws if w == text and top <= y <= bottom]
                    if found:
                        return found[0]
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
                    if abs(g[-1][0] - x) < 30 and abs(g[-1][1] - y) < 30:
                        g.append((x, y))
                        break
                else:
                    groups.append([(x, y)])
            out = [(sum(p[0] for p in g) // len(g), sum(p[1] for p in g) // len(g)) for g in groups
                   if len(g) * 4 >= minimum]
            return sorted(out, key=lambda p: p[1])

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
            def item():
                return phrase("server", "settings", rect=(72, TITLE_BAR + 48, 330, TITLE_BAR + 320))
            if not item():
                tap(kde, 72 + 80, TITLE_BAR + 24)
            tap(kde, *wait_for("Server Settings in the server menu", item, 15))
            wait_for("the settings page", lambda: phrase("server", "overview", rect=PAGE), 15)
            away()
            tap(kde, *wait_for("the Roles tab", lambda: exact("roles", NAV), 10))
            wait_for("the Roles tab open", lambda: phrase("roles", "are", "matrix", rect=PAGE), 10)
            away()

        def view(name):
            """One of the Roles tab's views: Roles, Members, Permissions."""
            tap(kde, *wait_for(f"the {name} view", lambda: exact(name, PAGE), 10))
            away()

        def close_settings():
            chord(ESCAPE)
            wait_for("the settings page closed", lambda: not exact("esc", PAGE), 10)

        def choose(label, row, current, choice):
            """In `row`'s picker, which shows `current`, pick `choice`."""
            at = wait_for(f"{label}'s picker", lambda: on_row(current, row(), right_of=800), 10)
            tap(kde, *at)
            tap(kde, *wait_for(f"{choice} in {label}'s picker", lambda: entry(choice, at), 10))
            away()

        def give(name, current, role):
            view("members")
            choose(name, lambda: exact(name, PAGE), current, role)

        def permission(first, second, current, role):
            choose(f"{first} {second}", lambda: phrase(first, second, rect=PAGE), current, role)

        def show(place, current):
            """The Permissions view's channel picker: `place` (a channel's
            name, or "server")."""
            at = wait_for("the channel picker", lambda: phrase(*current, rect=PAGE), 10)
            tap(kde, *at)
            tap(kde, *wait_for(f"{place} in the channel picker", lambda: entry(place, at, "server", "vault"), 10))
            away()

        def press(button, pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}, {"type": "pause", "duration": 250},
                                    {"type": "pointerDown", "button": button}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": button}])

        def message_menu(*line):
            """Right-click the message that reads `line`: the menu's words."""
            at = wait_for("the message", lambda: phrase(*line, rect=MAIN), 15)
            for attempt in range(3):
                press(2, at)
                try:
                    wait_for("the message menu", lambda: phrase("copy", "text", rect=WINDOW), 6)
                    break
                except RuntimeError:
                    if attempt == 2:
                        raise
            found = {w for w, _, _ in words()}
            chord(ESCAPE)
            wait_for("the message menu closed", lambda: not phrase("copy", "text", rect=WINDOW), 10)
            return found

        def offers_reaction(*line):
            """The message's menu has Add Reaction."""
            return "reaction" in message_menu(*line)

        def level_of(room, user):
            content = levels(room)
            return content.get("users", {}).get(user, content.get("users_default", 0))

        def tagged(name, tag):
            """Where `name` reads with the badge `tag` right after it (the
            10 px badge read at four times its size)."""
            im = kde.capture()
            for w, x, y in words(MAIN):
                if w != name:
                    continue
                line = [clean(v) for v, vx, _ in line_words(im, x, y) if v and vx > x - 30]
                if len(line) >= 2 and line[0] == name and line[1] == tag:
                    return x, y
            return None

        me, mira, nora, zed = people["me"], people["mira"], people["nora"], people["zed"]
        toolbar = Ui(lambda: words(), lambda p: None, grab, width=WINDOW[2], height=WINDOW[3] - TITLE_BAR,
                     origin=(0, TITLE_BAR))

        def steps():
            open_space("general", "announcements", "vault")
            shot("workshop")
            open_settings()
            # Roles: Owner above every level, and a custom role.
            try:
                wait_for("Owner, Admin, Moderator and Member", lambda: all(
                    exact(w, PAGE) for w in ("owner", "admin", "moderator", "member")), 20)
            finally:
                shot("roles")
            tap(kde, *wait_for("Create Role", lambda: phrase("create", "role", rect=PAGE), 10))
            time.sleep(0.5)
            kde.type("Helper")
            label = wait_for("the power level field", lambda: phrase("power", "level", rect=PAGE), 10)
            tap(kde, label[0] + 20, label[1] + 32)
            time.sleep(0.3)
            kde.type("25")
            swatch = wait_for("the orange swatch", lambda: blobs(ORANGE, PAGE, 100), 10)[0]
            tap(kde, *swatch)
            tap(kde, *wait_for("Save Role", lambda: phrase("save", "role", rect=PAGE), 10))
            away()
            saved = wait_for("Helper saved", lambda: state(rooms["workshop"], ROLES_EVENT), 20)
            assert saved == {"roles": {"25": {"name": "Helper", "color": "#e67e22"}}}, saved
            try:
                wait_for("Helper listed", lambda: exact("helper", PAGE), 15)
            finally:
                shot("helper")
            print(f"Matrix: Helper (25, #e67e22) is in Workshop's {ROLES_EVENT}; the Roles view lists Enzo's "
                  "Owner role above Admin", flush=True)

            # An announcement channel: Send messages needs Moderator, Add
            # reactions needs Helper.
            view("permissions")
            show("announcements", ("server", "defaults"))
            wait_for("announcements' permissions", lambda: phrase("send", "messages", rect=PAGE), 10)
            before = levels(rooms["announcements"])
            permission("send", "messages", "member", "moderator")
            wait_for("Send messages: Moderator", lambda: all(
                levels(rooms["announcements"]).get("events", {}).get(k) == 50 for k in POSTS), 20)
            permission("add", "reactions", "member", "helper")
            wait_for("Add reactions: Helper", lambda: levels(rooms["announcements"])["events"].get("m.reaction") == 25,
                     20)
            after = levels(rooms["announcements"])
            expected = dict(before, events=dict(before["events"], **{k: 50 for k in POSTS}, **{"m.reaction": 25}))
            assert after == expected, (before, after)
            assert after["org.example.policy"] == {"label": "Retain this extension"}
            assert "m.reaction" not in levels(rooms["general"]).get("events", {}), "general changed too"
            try:
                wait_for("where announcements differs", lambda: phrase("differs", "from", rect=PAGE), 15)
            finally:
                shot("announcements")
            refused = calls["send"](calls["nora"], rooms["announcements"], "Nora tries to announce")
            assert refused.status_code == 403 and refused.json()["errcode"] == "M_FORBIDDEN", refused.text
            refused = calls["react"](calls["nora"], rooms["announcements"])
            assert refused.status_code == 403, refused.text
            refused = calls["send"](calls["mira"], rooms["announcements"], "Mira before her role")
            assert refused.status_code == 403, refused.text
            print("Matrix: announcements needs 50 to post (messages, encrypted, stickers, polls) and 25 to react; "
                  "its other levels and unknown fields stayed. Nora's post and reaction and Mira's post: HTTP 403",
                  flush=True)

            # Mira: Moderator across the space, but vault.
            give("mira", "member", "moderator")
            for key in ("workshop", "general", "announcements"):
                wait_for(f"Mira at 50 in {key}", lambda k=key: level_of(rooms[k], mira) == 50, 20)
            assert level_of(rooms["vault"], mira) == 0, "vault changed: Quinn can't change it"
            shot("mira-moderator")
            posted = calls["send"](calls["mira"], rooms["announcements"], "Mira announces")
            assert posted.status_code == 200, posted.text
            print("Matrix: Mira is 50 in Workshop, general and announcements' m.room.power_levels (Element's "
                  "Moderator), still 0 in vault; she posts in announcements: HTTP 200", flush=True)

            # Nora: Helper reacts, still can't post.
            give("nora", "member", "helper")
            for key in ("workshop", "general", "announcements"):
                wait_for(f"Nora at 25 in {key}", lambda k=key: level_of(rooms[k], nora) == 25, 20)
            reacted = calls["react"](calls["nora"], rooms["announcements"])
            assert reacted.status_code == 200, reacted.text
            refused = calls["send"](calls["nora"], rooms["announcements"], "Nora tries again")
            assert refused.status_code == 403, refused.text
            print("Matrix: Nora is 25 (Helper); she reacts in announcements (HTTP 200) and still can't post (403)",
                  flush=True)

            # A server default: Invite people needs Moderator.
            view("permissions")
            show("server", ("announcements",))
            permission("invite", "people", "member", "moderator")
            for key in ("workshop", "general", "announcements"):
                wait_for(f"invite at 50 in {key}", lambda k=key: levels(rooms[k]).get("invite") == 50, 20)
            assert levels(rooms["vault"]).get("invite", 0) == 0
            shot("server-invite")
            refused = calls["invite"](calls["nora"], rooms["general"], zed)
            assert refused.status_code == 403, refused.text
            invited = calls["invite"](calls["mira"], rooms["general"], zed)
            assert invited.status_code == 200, invited.text
            print("Matrix: Invite people needs 50 in Workshop, general and announcements (not vault); Nora's "
                  "invite to general: HTTP 403, Mira's: 200", flush=True)
            close_settings()

            # Badges in general.
            tap(kde, *wait_for("general in the sidebar", lambda: exact("general", SIDEBAR), 15))
            wait_for("Nora's message", lambda: phrase("hello", "from", "nora", rect=MAIN), 30)
            try:
                nora_at = wait_for("Nora's HELPER badge", lambda: tagged("nora", "helper"), 30)
                wait_for("Mira's MOD badge", lambda: tagged("mira", "mod"), 30)
                orange = blobs(ORANGE, (nora_at[0] - 40, nora_at[1] - 12, nora_at[0] + 160, nora_at[1] + 12), 8)
                assert orange, "Nora's name and badge aren't orange"
            finally:
                shot("badges")
            print("UI: in general, Nora's name and badge read HELPER in orange, Mira's MOD", flush=True)

            # Quinn an ordinary member: read-only.
            calls["demote"]()
            for key in ("workshop", "announcements"):
                wait_for(f"Quinn a member in {key}", lambda k=key: level_of(rooms[k], me) == 0, 20)
            time.sleep(3)
            tap(kde, *wait_for("announcements in the sidebar", lambda: exact("announcements", SIDEBAR), 15))
            try:
                wait_for("the read-only composer", lambda: phrase("read-only", "for", "your", rect=MAIN), 30)
            finally:
                shot("announcements-read-only")
            open_settings()
            try:
                wait_for("the read-only notice", lambda: phrase("you", "can", "view", rect=PAGE), 20)
                assert not phrase("create", "role", rect=PAGE), "a member was offered Create Role"
                view("members")
                row = wait_for("Mira's row", lambda: exact("mira", PAGE), 10)
                role = wait_for("Mira's role", lambda: on_row("moderator", row, right_of=800), 10)
                tap(kde, *role)
                away()
                time.sleep(1)
                assert not entry("helper", role), "a member opened a role picker"
            finally:
                shot("read-only")
            close_settings()
            print("UI: as an ordinary member, Quinn sees announcements' composer read-only and the Roles tab "
                  "read-only, without Create Role or role pickers", flush=True)

            # Reactions: offered only where the room's level allows them.
            try:
                assert not offers_reaction("mira", "announces"), "Add Reaction offered to a member"
            finally:
                shot("member-menu")
            calls["helper"]()
            wait_for("Quinn a Helper in announcements", lambda: level_of(rooms["announcements"], me) == 25, 20)
            try:
                wait_for("Add Reaction offered to a Helper", lambda: offers_reaction("mira", "announces"), 40)
                assert phrase("read-only", "for", "your", rect=MAIN), "a Helper may post in announcements"
            finally:
                shot("helper-menu")
            print("UI: in announcements Quinn's message menu has no Add Reaction as a member, and has it as a "
                  "Helper (reactions need 25), while the composer stays read-only", flush=True)

        try:
            steps()
        except Exception:
            shot("failure")
            raise
        del toolbar


if __name__ == "__main__":
    main()
