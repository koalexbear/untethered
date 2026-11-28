"""Composer details (ROADMAP M13), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_composer_details.py

The test account (shown as Quill) shares a room, general, with Enzo, who
writes through the homeserver's API. The room is unencrypted, so what the
client sends can be read straight from Synapse. Enzo has filled general with
older messages, then sent an action (`m.emote`), a Rust code block, a line
with an inline emoji and custom emoji, a message of only emoji and one of
only custom emoji. The release client in a private KWin session (as
qa_kwin.py needs) must:
- show Enzo's action as "* Enzo dances wildly";
- colour the code block's keywords, string and number;
- draw the messages of only emoji, Unicode and custom, larger than the
  same emoji inline;
- list the slash commands when / starts the composer;
- send `/me waves hello` as an `m.emote`, shown as "* Quill waves hello";
- send `/shrug no idea` as "no idea ¯\\_(ツ)_/¯" with the HTML beside it;
- send `/spoiler the butler did it` with `data-mx-spoiler` in the HTML and
  the `||text||` fallback, hidden until clicked;
- open the last message for editing on ↑ in the empty composer, and save
  the edit;
- with general scrolled up past two new messages from Enzo (so nothing
  reads them), mark it read on Escape, and keep the composer focused.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
from contextlib import closing
import io
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import sys
import tempfile
import time
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import CHAT_X, Ui, clean, tesseract  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
MAIN = (330, 40, 1180, 900)
# Message text starts right of the avatars.
TEXT_LEFT = CHAT_X + 70
ENTER, ESCAPE, UP, END, PAGE_UP = "", "", "", "", ""
GREEN = (40, 220, 60)
SPOILER = (0x1E, 0x1F, 0x22)
# base16-ocean, which colours code blocks (highlight.rs).
SYNTAX = {"keyword": (0xB4, 0x8E, 0xAD), "string": (0xA3, 0xBE, 0x8C),
          "number": (0xD0, 0x87, 0x70), "function": (0x8F, 0xA1, 0xB3)}
SHRUG = "¯\\_(ツ)_/¯"
CODE = 'fn main() {\n    let answer = 42;\n    println!("hello {answer}");\n}'


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
    tmp = Path(tempfile.mkdtemp(prefix="untethered-composer-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, _, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))
        for user, h, name in ((me, hdr, "Quill"), (enzo, enzo_hdr, "Enzo")):
            http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(user, safe='')}/displayname",
                 headers=h, json={"displayname": name})
        general = http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json={
            "name": "general", "preset": "private_chat", "invite": [enzo]}).json()["room_id"]
        path = f"{hs}/_matrix/client/v3/rooms/{quote(general, safe='')}"
        http("POST", f"{path}/join", headers=enzo_hdr, json={})
        blob = http("POST", f"{hs}/_matrix/media/v3/upload?filename=blob.png", data=png(GREEN, 64),
                    headers={**hdr, "Content-Type": "image/png"}).json()["content_uri"]

        def say(content, h=enzo_hdr):
            txn = secrets.token_hex(8)
            return http("PUT", f"{path}/send/m.room.message/{txn}", headers=h, json=content).json()["event_id"]

        def html(body, formatted):
            return {"msgtype": "m.text", "body": body, "format": "org.matrix.custom.html",
                    "formatted_body": formatted}

        img = f'<img data-mx-emoticon src="{blob}" alt=":blob:" title=":blob:" height="32">'
        for n in range(1, 41):
            say({"msgtype": "m.text", "body": f"filler {n}"})
        say({"msgtype": "m.emote", "body": "dances wildly"})
        say(html(f"```rust\n{CODE}\n```", '<pre><code class="language-rust">'
                 + CODE.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
                 + "\n</code></pre>"))
        say(html("inline party 🎉 and :blob: here", f"inline party 🎉 and {img} here"))
        say({"msgtype": "m.text", "body": "🎉🎉🎉"})
        say(html(":blob: :blob:", f"{img} {img}"))
        say({"msgtype": "m.text", "body": "last line"})
        print(f"Synapse at {hs}: {me} (Quill) with {enzo} (Enzo) in general", flush=True)
        look(tmp, args.shots, hs, hdr, me, general, say)
        print("PASS: actions show as * Name text, code is coloured, emoji-only messages are larger, "
              "/ lists the commands, /me sends m.emote, /shrug and /spoiler send what Element reads, "
              "↑ edits the last message, and Escape marks the channel read keeping the composer focused",
              flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, hs, hdr, me, general, say):
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

        def shot(name, im=None):
            im = im or grab()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=MAIN):
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def exact(text, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if w == text), None)

        def hover(pos):
            kde.actions("pointer", [{"type": "pointerMove", "x": pos[0], "y": pos[1]}])

        def press(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def line(im, y, left=TEXT_LEFT - 10, right=1150, height=26):
            """The words on the text line at `y`, read at four times their size."""
            box = (left, y - height // 2, right, y + height // 2)
            return [(clean(w), box[0] + x, box[1] + wy) for w, x, wy in tesseract(im.crop(box), 7, scale=4)]

        def settled(label, find, timeout=20):
            """Where `find` reads twice in a row, a moment apart."""
            def still():
                first = find()
                time.sleep(0.8)
                second = find()
                return first if first and second and abs(first[0] - second[0]) < 4 \
                    and abs(first[1] - second[1]) < 4 else None
            return wait_for(label, still, timeout)

        def bands(im, rows, keep, left=TEXT_LEFT, right=1100):
            """Heights of the runs of rows in `rows` that have pixels `keep` accepts."""
            px = im.load()
            hits = [y for y in rows if sum(1 for x in range(left, right, 2) if keep(px[x, y])) >= 2]
            runs = []
            for y in hits:
                if runs and y - runs[-1][-1] <= 2:
                    runs[-1].append(y)
                else:
                    runs.append([y])
            return [r[-1] - r[0] + 1 for r in runs]

        def colourful(p):
            # An emoji picture's saturated pixels, not the custom emoji's green.
            return max(p) - min(p) > 90 and not near(p, GREEN, 60)

        def latest(check, label, timeout=30):
            def found():
                chunk = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(general, safe='')}/messages"
                             "?dir=b&limit=20", headers=hdr).json()["chunk"]
                return next((e for e in chunk if e["sender"] == me and e["type"] == "m.room.message"
                             and check(e["content"])), None)
            return wait_for(label, found, timeout)

        def receipts():
            with closing(sqlite3.connect((tmp / "homeserver.db").as_uri() + "?mode=ro", uri=True)) as db:
                return dict(db.execute("SELECT receipt_type,event_id FROM receipts_linearized "
                                       "WHERE room_id=? AND user_id=? AND thread_id IS NULL", (general, me)))

        toolbar = Ui(lambda: words(WINDOW), hover, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
        composer = toolbar.composer()

        # general opens with Enzo's messages at the bottom.
        press("", "k")
        time.sleep(0.6)
        kde.type("general")
        time.sleep(0.6)
        press(ENTER)
        last = settled("Enzo's last line", lambda: exact("last"), 60)
        dances = wait_for("Enzo's action", lambda: exact("dances"), 20)
        wait_for("both custom emoji pictures", lambda: len(bands(grab(), range(dances[1], last[1]),
                                                                 lambda p: near(p, GREEN))) >= 2, 30)
        time.sleep(1)
        im = shot("composer-received")

        # The action: "* Enzo dances wildly", the name before the text.
        read = line(im, dances[1])
        names = [x for w, x, _ in read if w == "enzo"]
        assert names and names[0] < dances[0], f"no name before Enzo's action: {read}"
        print("UI: Enzo's m.emote shows as * Enzo dances wildly", flush=True)

        # The code block: keywords, the string and the number in their colours.
        main_word = wait_for("the code block", lambda: exact("main"), 10)
        px = im.load()
        found = {name: sum(1 for y in range(main_word[1] - 14, main_word[1] + 70)
                           for x in range(TEXT_LEFT, 900) if near(px[x, y], colour, 22))
                 for name, colour in SYNTAX.items()}
        coloured = [name for name, n in found.items() if n >= 12]
        assert len(coloured) >= 3, f"code block colours: {found}"
        print(f"UI: the Rust code block is highlighted ({', '.join(coloured)})", flush=True)

        # Only emoji: larger than the same emoji inline.
        here = exact("here") or exact("inline")
        inline = bands(im, range(here[1] - 16, here[1] + 16), colourful)
        jumbo = bands(im, range(here[1] + 16, last[1] - 14), colourful)
        greens = bands(im, range(dances[1], last[1] - 14), lambda p: near(p, GREEN))
        assert inline and jumbo and max(jumbo) >= 30 and max(jumbo) > max(inline) * 1.5, \
            f"Unicode emoji inline {inline}, alone {jumbo}"
        assert len(greens) >= 2 and max(greens) >= 40 and min(greens) <= 30, f"custom emoji heights {greens}"
        print(f"UI: emoji alone draw at {max(jumbo)} px against {max(inline)} px inline; custom emoji "
              f"{max(greens)} px against {min(greens)} px", flush=True)

        # / lists the commands.
        tap(kde, *composer)
        time.sleep(0.5)
        kde.type("/")
        popup = (CHAT_X, 400, 1180, composer[1] - 20)
        seen = set()

        def listed():
            seen.clear()
            seen.update(w for w, _, _ in words(popup))
            # OCR often reads the slash as a letter ("ishrug").
            return "commands" in seen and all(any(w.endswith(name) and len(w) <= len(name) + 1
                                                  for w in seen) for name in ("me", "shrug", "spoiler"))
        try:
            wait_for("the command list", listed, 15)
        finally:
            shot("composer-commands")
            print(f"OCR over the composer: {sorted(seen)}", flush=True)
        print("UI: / lists the commands /me, /shrug and /spoiler", flush=True)

        # /me sends an m.emote.
        kde.type("me waves hello")
        press(ENTER)
        emote = latest(lambda c: c.get("msgtype") == "m.emote", "the /me action")["content"]
        assert emote["body"] == "waves hello", emote
        waves = wait_for("the action shown", lambda: exact("waves"), 20)
        read = line(grab(), waves[1])
        names = [x for w, x, _ in read if w == "quill"]
        assert names and names[0] < waves[0], f"no name before our action: {read}"
        print("Matrix: /me waves hello reached the room as msgtype m.emote; UI: * Quill waves hello",
              flush=True)

        # /shrug appends the shrug, HTML beside it.
        kde.type("/shrug no idea")
        press(ENTER)
        shrug = latest(lambda c: SHRUG in c.get("body", ""), "the /shrug message")["content"]
        assert shrug["msgtype"] == "m.text" and shrug["body"] == f"no idea {SHRUG}", shrug
        assert shrug.get("formatted_body") == f"no idea {SHRUG}", shrug
        wait_for("the shrug shown", lambda: exact("idea"), 20)
        print(f"Matrix: /shrug no idea sent body and formatted_body 'no idea {SHRUG}'", flush=True)

        # /spoiler: Element's spoiler, Discord's fallback, hidden here.
        kde.type("/spoiler the butler did it")
        press(ENTER)
        spoiler = latest(lambda c: "butler" in c.get("body", ""), "the /spoiler message")["content"]
        assert spoiler["body"] == "||the butler did it||", spoiler
        assert spoiler.get("formatted_body") == "<span data-mx-spoiler>the butler did it</span>", spoiler
        time.sleep(2)

        def hidden_box():
            im = grab()
            px = im.load()
            rows = [y for y in range(300, composer[1] - 30)
                    if sum(1 for x in range(TEXT_LEFT, 900) if px[x, y] == SPOILER) > 80]
            return rows and (TEXT_LEFT + 40, rows[-1] - 6)
        box = wait_for("the hidden spoiler", hidden_box, 15)
        im = shot("composer-spoiler")
        assert not exact("butler"), "the spoiler's text shows before it's clicked"
        tap(kde, *box)
        wait_for("the spoiler revealed", lambda: exact("butler"), 10)
        print("Matrix: /spoiler sent <span data-mx-spoiler> and ||the butler did it||; UI: hidden until "
              "clicked", flush=True)

        # ↑ in the empty composer edits our last message.
        tap(kde, *composer)
        kde.type("plain words here")
        press(ENTER)
        plain = latest(lambda c: c.get("body") == "plain words here", "the plain message")
        time.sleep(1.5)
        press(UP)
        wait_for("the inline editor", lambda: exact("save") or exact("cancel"), 15)
        shot("composer-edit")
        press(END)
        kde.type(" fixed")
        press(ENTER)
        edit = latest(lambda c: (c.get("m.relates_to") or {}).get("rel_type") == "m.replace", "the edit")
        assert edit["content"]["m.relates_to"]["event_id"] == plain["event_id"], edit
        assert edit["content"]["m.new_content"]["body"] == "plain words here fixed", edit
        print("Matrix: ↑ opened our last message and the edit replaced it with 'plain words here fixed'",
              flush=True)

        # Escape marks general read: scrolled up, two new messages stay unread.
        time.sleep(1.5)
        tap(kde, *composer)
        for _ in range(4):
            press(PAGE_UP)
            time.sleep(0.4)
        time.sleep(2)
        shot("composer-scrolled")
        print(f"receipts before Enzo's new messages: {receipts()}", flush=True)
        say({"msgtype": "m.text", "body": "unread one"})
        newest = say({"msgtype": "m.text", "body": "unread two"})
        time.sleep(5)
        before = receipts()
        shot("composer-unread")
        assert newest not in before.values(), f"read while scrolled up: {before}, newest {newest}"
        press(ESCAPE)
        wait_for("Escape's read receipt", lambda: newest in receipts().values(), 20)
        print(f"Matrix: Escape moved our receipt to Enzo's newest message ({receipts()})", flush=True)
        kde.type("still focused")
        wait_for("typing after Escape", lambda: exact("focused", (CHAT_X, composer[1] - 30, 1180,
                                                                  composer[1] + 30)), 10)
        shot("composer-after-escape")
        print("UI: the composer kept its focus after Escape", flush=True)


if __name__ == "__main__":
    main()
