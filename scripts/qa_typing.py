"""Typing indicators (ROADMAP M3) against Element Web. Element typing shows
"Mira is typing…" under Untethered's composer within a second, and it clears
when Element stops or sends; with a second typer it reads "Mira and Jo are
typing…". Untethered's typing shows in Element and stops on send,
when the draft is cleared and after a few idle seconds. With User Settings →
Privacy → Send typing indicators off, Element sees none; turned back on from
another device (the account data), it does again."""
import os
import time
from pathlib import Path
from urllib.parse import quote

from qa_native_paste import wait_for
from qa_ui import CHAT_X, SIDE, line_words

TYPING_DATA = "com.untethered.typing_notices.v1"
BLURPLE = (0x58, 0x65, 0xF2)  # A switch's track while on.


def xdotool_input(x, ui):
    """The native client's keyboard and wheel on its X display."""
    def wheel():
        x("mousemove", str(ui.width // 2), str(ui.height // 2))
        x("click", "--repeat", "2", "5")
    return dict(type_text=lambda s: x("type", "--clearmodifiers", s),
                keys=lambda *k: x("key", "--clearmodifiers", *k), wheel=wheel, browser=False)


WEB_KEYS = {"ctrl+a": "Control+a", "BackSpace": "Backspace", "Return": "Enter"}


def browser_input(client, ui):
    """The web client's keyboard and wheel in its Chromium page."""
    def keys(*names):
        for name in names:
            client.keyboard.press(WEB_KEYS.get(name, name))
    def wheel():
        client.mouse.move(ui.width // 2, ui.height // 2)
        client.mouse.wheel(0, 120)
    return dict(type_text=lambda s: client.keyboard.type(s, delay=5), keys=keys, wheel=wheel, browser=True)


def check_typing(page, room, me, hs, hdr, http, *, click, type_text, keys, wheel, browser, words, ui, capture):
    """`page` is Element's. The client under test is driven through `click`,
    `type_text`, `keys` (xdotool key names), `wheel` (scroll down a little
    under the pointer) and read through `capture` and `words` (OCR)."""
    editor = page.locator('[contenteditable=true]:visible').last

    def footer():
        """Words on the line under the composer: the room's note, or who is
        typing in its place."""
        return {w for w, _, _ in line_words(capture(), CHAT_X + 76, ui.height - 14, width=560)}

    def named(w, name):
        """`name` is among the footer's words. In headless Chromium (WebGL on
        SwiftShader, where the client's SVG icons don't draw either) the
        line's first glyph sometimes comes out garbled, so the browser run
        also takes the name without its first letter."""
        return name in w or browser and name[1:] in w

    def until(what, check, limit):
        """Seconds until `check()` holds, measured to the start of the poll
        that saw it (each poll takes a screenshot and OCR or a JS call)."""
        start = time.monotonic()
        while True:
            at = time.monotonic()
            if check():
                return at - start
            if at - start > limit:
                if shots := os.environ.get("TYPING_QA_SHOTS"):
                    Path(shots).mkdir(parents=True, exist_ok=True)
                    capture().save(Path(shots) / "timeout.png")
                print("Client footer words:", sorted(footer()), "Element sees typing:", element_sees(),
                      "Element's typing members:", element_typers(), flush=True)
                raise RuntimeError(f"timed out after {limit} s waiting for {what}")
            time.sleep(.05)

    def shows(*names):
        def check():
            w = footer()
            return "typing" in w and all(named(w, n) for n in names)
        return check

    def clear():
        w = footer()
        return "typing" not in w and "encrypted" in w

    def element_sees():
        return page.evaluate("([room,me])=>!!window.mxMatrixClientPeg.get().getRoom(room)?.getMember(me)?.typing",
                             [room, me])

    def element_typers():
        return page.evaluate(
            "room=>window.mxMatrixClientPeg.get().getRoom(room)?.getMembers().filter(m=>m.typing).map(m=>m.name)", room)

    def trace(what):
        """TYPING_QA_TRACE=1: timestamps for the two-typer step."""
        if os.environ.get("TYPING_QA_TRACE"):
            print(f"{time.monotonic():.2f} {what}", flush=True)

    def locate(what, fn, limit=45):
        """Whole-window OCR, which can take seconds a pass on a busy machine."""
        try:
            return wait_for(what, lambda: fn(words()), limit)
        except RuntimeError:
            print("Client visible words:", [w for w, _, _ in words()], flush=True)
            raise

    def element_clear():
        editor.click()
        page.keyboard.press("Control+a")
        page.keyboard.press("Backspace")

    wait_for("the composer's room note", clear, 20)

    # 1. Element types: the line shows within a second, and clears when it
    #    stops (an emptied composer) or sends.
    editor.click()
    page.keyboard.type("H")
    shown = until("Mira is typing… under the composer", lambda: shows("mira")() and "is" in footer(), 10)
    page.keyboard.type("ello", delay=40)
    assert shown < 1.5, f"Element's typing took {shown:.2f} s to show"
    element_clear()
    stopped = until("the line to clear after Element emptied its composer", clear, 10)
    page.keyboard.type("Typed in Element", delay=20)
    until("Mira is typing… again", shows("mira"), 10)
    page.keyboard.press("Enter")
    sent = until("the line to clear after Element sent", clear, 10)
    print(f"Element → Untethered: 'Mira is typing…' showed {shown:.2f} s after the first key; "
          f"cleared {stopped:.2f} s after Element emptied its composer and {sent:.2f} s after it sent.", flush=True)

    # 2. Two people: a third account, Jo, types through the API while
    #    Element does. Jo has never posted: the name comes from membership.
    jo = http("POST", hs + "/_matrix/client/v3/register", json={
        "auth": {"type": "m.login.dummy"}, "username": "jo" + str(int(time.time())), "password": "typing-test-jo"}).json()
    jh = {"Authorization": "Bearer " + jo["access_token"]}
    api = hs + "/_matrix/client/v3/rooms/" + quote(room, safe="")
    http("PUT", hs + "/_matrix/client/v3/profile/" + quote(jo["user_id"], safe="") + "/displayname",
         headers=jh, json={"displayname": "Jo"})
    http("POST", api + "/invite", headers=hdr, json={"user_id": jo["user_id"]})
    http("POST", api + "/join", headers=jh, json={})
    jo_typing = api + "/typing/" + quote(jo["user_id"], safe="")
    time.sleep(2)  # Untethered syncs Jo's membership.
    editor.click()
    page.keyboard.type("Mira first", delay=20)
    trace("Element typed")
    until("Mira is typing…", shows("mira"), 10)
    trace("Jo starts typing")
    http("PUT", jo_typing, headers=jh, json={"typing": True, "timeout": 30000})

    def both():
        w = footer()
        trace(f"client: {sorted(w)} Element's typing members: {element_typers()}")
        return {"typing", "and", "are"} <= w and named(w, "mira") and named(w, "jo")
    until("Mira and Jo are typing…", both, 10)
    # Element stops by itself 10 s after its last key.
    page.keyboard.type(" again", delay=20)
    http("PUT", jo_typing, headers=jh, json={"typing": False})
    until("Jo to drop out of the line",
          lambda: (w := footer()) >= {"is", "typing"} and named(w, "mira") and "jo" not in w, 10)
    element_clear()
    until("the line to clear", clear, 10)
    print("Two typers: 'Mira and Jo are typing…' (Jo's name from membership), then Mira alone, then the room note.",
          flush=True)

    # 3. Untethered's composer: Element sees typing at once, and the client
    #    never lists its own user; it stops after a few idle seconds, on
    #    clearing the draft and on send.
    click(ui.composer())
    type_text("H")
    seen = until("Element to see the Untethered user typing", element_sees, 10)
    type_text("ello")
    last_key = time.monotonic()
    assert "typing" not in footer(), "the client shows its own typing"
    # Typing lasts while the draft keeps changing...
    time.sleep(3)
    assert element_sees(), "typing stopped before the idle limit"
    # ...and stops a few seconds after it last did.
    until("Element to see the typing stop when idle", lambda: not element_sees(), 15)
    idle = time.monotonic() - last_key
    assert 4 <= idle <= 9, f"the idle stop came {idle:.1f} s after the last key"
    type_text("!")
    until("Element to see typing again", element_sees, 10)
    keys("ctrl+a", "BackSpace")
    cleared = until("Element to see typing stop on an emptied draft", lambda: not element_sees(), 10)
    type_text("Typed in Untethered")
    until("Element to see typing again", element_sees, 10)
    keys("Return")
    on_send = until("Element to see typing stop on send", lambda: not element_sees(), 10)
    page.get_by_text("Typed in Untethered", exact=True).last.wait_for(timeout=30000)
    print(f"Untethered → Element: typing seen {seen:.2f} s after the first key; kept through a 3 s pause; "
          f"stopped {idle:.1f} s after the last key, {cleared:.2f} s after emptying the draft and "
          f"{on_send:.2f} s after sending.", flush=True)

    # 4. The switch: off in User Settings → Privacy, synced as account data;
    #    then Element sees no typing.
    account_data = hs + "/_matrix/client/v3/user/" + quote(me, safe="") + "/account_data/" + TYPING_DATA
    click(ui.settings())
    locate("User Settings", lambda ws: {"privacy", "receipts"} & {w for w, _, _ in ws} or None)

    def switch():
        """The Send typing indicators switch: its centre, and whether it's on.
        Scrolls down a little at a time until the row is on screen."""
        for _ in range(12):
            ws = words()
            rows = [y for w, xx, y in ws if w == "indicators"
                    and any(v == "typing" and abs(vy - y) < 6 and vx < xx for v, vx, vy in ws)]
            if rows and rows[0] < ui.height - 60:
                break
            wheel()
            time.sleep(.3)
        else:
            print("Client visible words:", [w for w, _, _ in ws], flush=True)
            raise RuntimeError("no Send typing indicators row in User Settings")
        y = rows[0]
        im = capture()
        box = [(xx, yy) for yy in range(max(0, y - 14), min(im.height, y + 40)) for xx in range(ui.width // 2, ui.width - 70)
               if sum(abs(a - b) for a, b in zip(im.getpixel((xx, yy)), SIDE)) > 60]
        right = max(xx for xx, _ in box)
        knob = [(xx, yy) for xx, yy in box if xx > right - 50]
        cx, cy = sum(xx for xx, _ in knob) // len(knob), sum(yy for _, yy in knob) // len(knob)
        on = sum(sum(abs(a - b) for a, b in zip(im.getpixel(p), BLURPLE)) < 40 for p in knob) > 40
        return (cx, cy), on

    at, on = switch()
    assert on, "Send typing indicators starts on"
    click(at)
    def saved():
        try:
            return http("GET", account_data, headers=hdr).json()
        except Exception:  # 404 until the first save.
            return None
    wait_for("the switch saved as account data", lambda: saved() == {"send": False}, 15)
    wait_for("the switch to show off", lambda: not switch()[1], 10)
    keys("Escape")
    wait_for("the room again", clear, 15)
    click(ui.composer())
    type_text("Quiet typing")
    end = time.monotonic() + 7
    while time.monotonic() < end:
        assert not element_sees(), "Element saw typing with the switch off"
        time.sleep(.2)
    print("Switch off: User Settings wrote {'send': false}; Element saw no typing for 7 s of a draft.", flush=True)

    # 5. Turned back on elsewhere (the account data), typing goes out again.
    keys("ctrl+a", "BackSpace")
    http("PUT", account_data, headers=hdr, json={"send": True})
    time.sleep(2)  # Untethered syncs it.
    type_text("Loud")
    until("Element to see typing after the switch came back on", element_sees, 10)
    keys("ctrl+a", "BackSpace")
    until("Element to see typing stop", lambda: not element_sees(), 10)
    print("Switch back on from another device: Element saw typing again.", flush=True)
