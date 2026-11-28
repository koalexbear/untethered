"""Custom emoji and stickers (ROADMAP M11), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_emotes.py

The test account owns a space, Workshop, whose encrypted channel general it
shares with Enzo on a gomuks device. Workshop has an image pack under the
spec's name (`m.room.image_pack`): a green and an orange emoji and a magenta
sticker. A room outside it, lounge, has a cyan emoji in a pack under the
unstable name Cinny writes (`im.ponies.room_emotes`); a third room,
elsewhere, has none. The release client in a private KWin session (as
qa_kwin.py needs) must:
- show Enzo's Cinny-style custom emoji inline in general;
- send one from `:` autocomplete as Cinny reads it: the picture in
  formatted_body (`data-mx-emoticon`), its shortcode in body;
- react with one from the reaction picker, its shortcode beside the mxc key;
- send the sticker from the composer's sticker picker as `m.sticker`;
- not offer Workshop's emoji in elsewhere, and offer lounge's there once
  "Use everywhere" is ticked, saved under both names.
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
from qa_matrix_sdk import Gomuks, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
MAIN = (330, 40, 1180, 900)
ENTER, TAB, CONTROL, ESCAPE = "", "", "", ""
GREEN = (40, 220, 60)
ORANGE = (250, 140, 20)
CYAN = (20, 220, 230)
MAGENTA = (225, 30, 190)


def near(pixel, colour, slack=40):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def png(colour, size):
    out = io.BytesIO()
    Image.new("RGB", (size, size), colour).save(out, "PNG")
    return out.getvalue()


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-emotes-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))
        encryption = [{"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def state(room, kind, key, content):
            http("PUT", f"{path(room)}/state/{kind}/{quote(key, safe='')}", headers=hdr, json=content)

        def upload(data, name):
            return http("POST", f"{hs}/_matrix/media/v3/upload?filename={name}", data=data,
                        headers={**hdr, "Content-Type": "image/png"}).json()["content_uri"]

        workshop = create(name="Workshop", preset="private_chat", invite=[enzo],
                          creation_content={"type": "m.space"})
        general = create(name="general", preset="private_chat", invite=[enzo], initial_state=encryption)
        lounge = create(name="lounge", preset="private_chat")
        elsewhere = create(name="elsewhere", preset="private_chat")
        state(workshop, "m.space.child", general, {"via": ["test.invalid"]})
        state(general, "m.space.parent", workshop, {"via": ["test.invalid"], "canonical": True})
        for room in (workshop, general):
            http("POST", f"{path(room)}/join", headers=enzo_hdr, json={})
        urls = {
            "blobwave": upload(png(GREEN, 64), "blobwave.png"),
            "hammer": upload(png(ORANGE, 64), "hammer.png"),
            "bigblob": upload(png(MAGENTA, 256), "bigblob.png"),
            "cat": upload(png(CYAN, 64), "cat.png"),
        }
        state(workshop, "m.room.image_pack", "", {
            "pack": {"display_name": "Workshop emoji"},
            "images": {
                "blobwave": {"url": urls["blobwave"], "body": "Waving blob"},
                "hammer": {"url": urls["hammer"], "usage": ["emoticon"]},
                "bigblob": {"url": urls["bigblob"], "usage": ["sticker"], "body": "A big blob",
                            "info": {"w": 256, "h": 256, "mimetype": "image/png"}},
            }})
        state(lounge, "im.ponies.room_emotes", "", {
            "pack": {"display_name": "Lounge cats"}, "images": {"cat": {"url": urls["cat"]}}})

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        peer.joined(general)
        img = (f'<img data-mx-emoticon src="{urls["blobwave"]}" alt=":blobwave:" title=":blobwave:" '
               f'height="32">')
        peer.rpc("send_event", {"room_id": general, "type": "m.room.message", "content": {
            "msgtype": "m.text", "body": "hi :blobwave:", "format": "org.matrix.custom.html",
            "formatted_body": f"hi {img}"}})
        hello = find(peer, "Enzo's message", lambda e: e.get("room_id") == general
                     and (e.get("decrypted") or {}).get("body") == "hi :blobwave:")
        time.sleep(2)
        print(f"Synapse at {hs}: {me} with {enzo} on gomuks; packs in Workshop (spec) and lounge (Cinny)",
              flush=True)
        rooms = dict(general=general, lounge=lounge, elsewhere=elsewhere)
        look(tmp, args.shots, hs, hdr, me, peer, rooms, urls, hello["event_id"])
        print("PASS: custom emoji show inline, send from autocomplete as Cinny reads them, react with "
              "their shortcode, a sticker sends as m.sticker, and a pack used everywhere follows you",
              flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def find(peer, label, check, timeout=60):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        for e in peer.events:
            if check(e) and str(e.get("event_id", "")).startswith("$"):
                return e
        peer.frame(1)
    raise RuntimeError(f"gomuks never saw {label}")


def look(tmp, shots, hs, hdr, me, peer, rooms, urls, hello):
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

        def grab():
            return kde.capture().crop(WINDOW).convert("RGB")

        def shot(name):
            im = grab()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=MAIN):
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def word(part, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if part in w), None)

        def exact(text, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if w == text), None)

        def hover(pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}])

        def blobs(colour, rect=MAIN, minimum=40):
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

        def settled(label, find, timeout=20):
            """Where `find` reads twice in a row, a moment apart."""
            def still():
                first = find()
                time.sleep(0.8)
                second = find()
                return first if first and second and abs(first[0] - second[0]) < 4 \
                    and abs(first[1] - second[1]) < 4 else None
            return wait_for(label, still, timeout)

        def chord(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def open_room(name):
            chord(CONTROL, "k")
            time.sleep(0.6)
            kde.type(name)
            time.sleep(0.6)
            chord(ENTER)
            wait_for(f"#{name} open", lambda: word(name, (330, 30, 900, 70)), 20)
            time.sleep(1)

        def peer_event(label, room, kind, check, timeout=90):
            def matches(e):
                if e.get("room_id") != room or e.get("sender") != me:
                    return False
                content = e.get("decrypted") if e.get("type") == "m.room.encrypted" else e.get("content")
                return (e.get("decrypted_type") or e.get("type")) == kind and content and check(e, content)
            return find(peer, label, matches, timeout)

        toolbar = Ui(lambda: words(WINDOW), hover, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
        send = toolbar.send()
        sticker_button = (send[0] - 46, send[1])
        emoji_button = (send[0] - 92, send[1])

        open_room("general")
        try:
            wait_for("Enzo's custom emoji inline", lambda: exact("hi") and blobs(GREEN), 60)
        finally:
            shot("emote-inline")
        print("UI: Enzo's Cinny-style custom emoji shows inline in general", flush=True)

        # `:` autocomplete: the space's pack, first.
        tap(kde, *toolbar.composer())
        kde.type(":blobw")
        wait_for("blobwave in the popup", lambda: word("blobwave", WINDOW) and len(blobs(GREEN, WINDOW)) >= 2, 15)
        shot("emote-autocomplete")
        chord(TAB)
        kde.type("works")
        chord(ENTER)
        sent = peer_event("the custom emoji message", rooms["general"], "m.room.message",
                          lambda e, c: c.get("body") == ":blobwave: works")["decrypted"]
        assert sent.get("format") == "org.matrix.custom.html", sent
        assert f'<img data-mx-emoticon src="{urls["blobwave"]}" alt=":blobwave:"' in sent["formatted_body"], sent
        wait_for("both custom emoji inline", lambda: len(blobs(GREEN)) >= 2, 30)
        print("Matrix: the autocompleted emoji reached Enzo as Cinny sends one: data-mx-emoticon in "
              "formatted_body, :blobwave: in body", flush=True)

        # React with it from the reaction picker.
        row = settled("Enzo's message", lambda: exact("hi"))
        try:
            tap(kde, *wait_for("Add Reaction", lambda: toolbar.message(row, "add reaction"), 20))
        finally:
            shot("emote-toolbar")
        header = wait_for("the pack in the reaction picker", lambda: word("workshop", WINDOW), 15)
        shot("emote-reaction-picker")
        cell = wait_for("the green emoji in the picker", lambda: next(
            (b for b in blobs(GREEN, WINDOW) if 0 < b[1] - header[1] < 60), None), 10)
        tap(kde, *cell)

        def reacted(e, content):
            rel = (e.get("content") or {}).get("m.relates_to") or content.get("m.relates_to") or {}
            return rel.get("key") == urls["blobwave"] and e.get("relates_to") == hello
        reaction = peer_event("the custom reaction", rooms["general"], "m.reaction", reacted)
        decrypted = reaction.get("decrypted") or reaction.get("content")
        assert decrypted.get("shortcode") == ":blobwave:", reaction
        print("Matrix: the reaction's key is the emoji's mxc URI, with shortcode :blobwave: beside it",
              flush=True)

        # A sticker from the composer's sticker picker.
        time.sleep(1)
        tap(kde, *sticker_button)
        try:
            wait_for("the sticker picker", lambda: word("stickers", WINDOW) and blobs(MAGENTA, WINDOW), 15)
        finally:
            shot("emote-stickers")
        tap(kde, *blobs(MAGENTA, WINDOW)[-1])
        sticker = peer_event("the sticker", rooms["general"], "m.sticker",
                             lambda e, c: c.get("url") == urls["bigblob"])
        assert (sticker.get("decrypted") or {}).get("body") == "A big blob", sticker
        print("Matrix: the sticker went out as m.sticker with its picture and description", flush=True)

        # Elsewhere has no pack; lounge's, once used everywhere, follows you.
        open_room("elsewhere")
        tap(kde, *emoji_button)
        wait_for("the emoji picker", lambda: word("emoji", (330, 300, 1180, 840)), 15)
        time.sleep(1.5)
        shot("emote-elsewhere-picker")
        # Unicode's emoji are orange and green too: look for the pack's header.
        assert not word("workshop", WINDOW) and not blobs(GREEN), "Workshop's emoji offered outside it"
        chord(ESCAPE)
        open_room("lounge")
        tap(kde, *emoji_button)
        wait_for("lounge's pack", lambda: word("lounge", (330, 300, 1180, 840)) and blobs(CYAN, WINDOW), 30)
        shot("emote-lounge-picker")
        tap(kde, *wait_for("Use everywhere", lambda: word("everywhere", WINDOW), 10))

        def saved(kind):
            got = http("GET", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/{kind}",
                       headers=hdr)
            return got.status_code == 200 and "" in (got.json().get("rooms") or {}).get(rooms["lounge"], {})
        wait_for("lounge's pack used everywhere", lambda: saved("m.image_pack.rooms")
                 and saved("im.ponies.emote_rooms"), 20)
        chord(ESCAPE)
        open_room("elsewhere")
        tap(kde, *toolbar.composer())
        kde.type(":ca")
        wait_for("cat in the popup", lambda: word("cat", WINDOW) and blobs(CYAN, WINDOW), 15)
        chord(TAB)
        kde.type("here")
        chord(ENTER)

        def in_elsewhere():
            events = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(rooms['elsewhere'], safe='')}"
                          "/messages?dir=b&limit=10", headers=hdr).json()["chunk"]
            return next((e["content"] for e in events if e["type"] == "m.room.message"
                         and e["content"].get("body") == ":cat: here"), None)
        content = wait_for("the cat in elsewhere", in_elsewhere, 30)
        assert urls["cat"] in content.get("formatted_body", ""), content
        shot("emote-everywhere")
        print("UI: Workshop's emoji stay in Workshop; lounge's Cinny pack, once used everywhere (saved under "
              "both names), sends from elsewhere", flush=True)


if __name__ == "__main__":
    main()
