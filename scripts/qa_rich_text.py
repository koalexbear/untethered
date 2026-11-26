"""Element sends each rich-text construct through its own composer; Untethered
shows it styled, without Markdown markers, and keeps spoilers hidden until
clicked. A bot-style message with only a plain `body` checks the Markdown and
`||spoiler||` path. RICH_QA_DEBUG=<dir> keeps both clients' screenshots for a
side-by-side look."""
import os
import time
from pathlib import Path

from qa_native_paste import wait_for
from qa_ui import CHAT_X, HEADER

LINK = (0x00, 0xA8, 0xFC)
CODE = (0x2B, 0x2D, 0x31)
SPOILER = (0x1E, 0x1F, 0x22)
QUOTE_BAR = (0x4E, 0x50, 0x58)
# Each message is typed into Element's composer line by line (Shift+Enter).
MESSAGES = [
    ["Inline **bold** *italic* ~~struck~~ `inline code` and [docs link](https://example.org/docs)"],
    ["# Release notes", "> quoted wisdom", "", "- first item", "- second item", "", "1. alpha", "2. beta"],
    ["```rust", "fn main() {", '    println!("zebra");', "}", "```"],
    ["/spoiler Rosebud was the sled"],
]
# Only a plain body, as from a bot or bridge.
PLAIN = "Bot says **loud** and ||hidden gem|| here"


def pixels(im, box, color, tolerance=6):
    x0, y0, x1, y1 = (max(0, box[0]), max(0, box[1]), min(im.width, box[2]), min(im.height, box[3]))
    return sum(1 for x in range(x0, x1) for y in range(y0, y1)
               if all(abs(a - b) <= tolerance for a, b in zip(im.getpixel((x, y)), color)))


def hidden_boxes(im):
    """Filled spoiler-coloured boxes in the conversation, top to bottom. The
    rail and the header's search field share the colour but are outside the
    conversation; a code block's 1-px edge has no inside."""
    px = im.load()
    inside = set()
    for y in range(HEADER + 2, im.height - 2):
        for x in range(CHAT_X + 2, im.width - 2):
            if all(px[p] == SPOILER for p in ((x, y), (x - 2, y), (x + 2, y), (x, y - 2), (x, y + 2))):
                inside.add((x, y))
    boxes = []
    for x, y in sorted(inside, key=lambda p: (p[1], p[0])):
        for b in boxes:
            if b[0] - 3 <= x <= b[2] + 3 and b[1] - 3 <= y <= b[3] + 3:
                b[0], b[1], b[2], b[3] = min(b[0], x), min(b[1], y), max(b[2], x), max(b[3], y)
                break
        else:
            boxes.append([x, y, x, y])
    return [b for b in boxes if b[2] - b[0] > 20 and b[3] - b[1] > 6]


def check_rich(page, room, locate, click, hover, key, capture, composer):
    debug = Path(d) if (d := os.environ.get("RICH_QA_DEBUG")) else None
    editor = page.locator('[contenteditable=true]:visible').last
    for lines in MESSAGES:
        editor.click()
        for i, line in enumerate(lines):
            if i:
                page.keyboard.press("Shift+Enter")
            page.keyboard.type(line, delay=4)
        page.keyboard.press("Enter")
        time.sleep(.6)
    page.evaluate("""async ([room,body])=>{
        await window.mxMatrixClientPeg.get().sendEvent(room,'m.room.message',{msgtype:'m.text',body});
    }""", [room, PLAIN])

    def sent():
        events = page.evaluate("""room=>{
            const c=window.mxMatrixClientPeg.get();
            return c.getRoom(room).getLiveTimeline().getEvents()
                .filter(e=>e.getSender()===c.getUserId()&&e.getType()==='m.room.message'&&e.getId()?.startsWith('$'))
                .map(e=>({body:e.getContent().body,html:e.getContent().formatted_body||null}));
        }""", room)
        return events if len(events) == len(MESSAGES) + 1 else None
    events = wait_for("Element sent every construct", sent, 30)
    for e in events:
        print("Element sent:", e, flush=True)
    inline, blocks, code, spoiler, plain = (e["html"] or "" for e in events)
    assert all(t in inline for t in ("<strong>bold</strong>", "<em>italic</em>", "<code>inline code</code>",
                                     '<a href="https://example.org/docs">docs link</a>')), inline
    assert all(t in blocks for t in ("<h1>Release notes</h1>", "<blockquote>", "<ul>", "<ol>")), blocks
    assert '<pre><code class="language-rust">' in code, code
    assert "data-mx-spoiler" in spoiler, spoiler
    assert not plain, "the bot message must have only a plain body"
    page.get_by_text("Bot says").wait_for()
    if debug:
        page.screenshot(path=str(debug / "element.png"))

    def chat(ws):
        return [(w, x, y) for w, x, y in ws if x > CHAT_X]

    def laid_out(ws):
        found = {w for w, _, _ in chat(ws)}
        return ws if {"bold", "italic", "release", "quoted", "second", "beta", "bot", "loud"} <= found and any(
            "zebra" in w for w in found) else None
    ws = chat(locate("Untethered rich messages", laid_out))
    im = capture()
    if debug:
        im.save(debug / "untethered.png")
    at = {w: (x, y) for w, x, y in ws}

    def line(word):
        y = at[word][1]
        return [w for w, _, yy in ws if abs(yy - y) < 6]
    # No Markdown markers in place of the styles.
    for word in ("bold", "release", "loud"):
        assert not any(c in w for w in line(word) for c in "*#`"), (word, line(word))
    assert not any("``" in w for w, _, _ in ws), "code fence shown as text"
    for word in ("inline", "docs", "first", "alpha"):
        assert word in at, f"{word!r} missing from {sorted(at)}"
    # Element 1.12.35 leaves ~~ as typed; show whatever it sent.
    assert any("struck" in w for w, _, _ in ws)
    if "<del>" in inline:
        assert not any("~~" in w for w, _, _ in ws), "strikethrough shown as ~~"
    x, y = at["docs"]
    assert pixels(im, (x - 30, y - 8, x + 60, y + 8), LINK, 30) > 20, "link not in the link colour"
    # "Inline" also starts the message; "code" is only in the code span.
    x, y = at["code"]
    assert pixels(im, (x - 60, y - 9, x + 30, y + 9), CODE, 2) > 60, "inline code without its background"
    x, y = at["quoted"]
    assert pixels(im, (x - 50, y - 8, x - 10, y + 8), QUOTE_BAR, 4) > 20, "quote without its bar"
    zebra = next((x, y) for w, x, y in ws if "zebra" in w)
    assert pixels(im, (zebra[0] - 120, zebra[1] - 30, zebra[0] + 200, zebra[1] + 30), CODE, 2) > 2000, \
        "code block without its background"
    print("Untethered shows bold, italic, strikethrough, inline code, the link, heading, quote, "
          "both lists and the code block without Markdown markers.", flush=True)

    # Spoilers: hidden, then shown one at a time.
    def secret(ws, word):
        return any(word in w for w, _, _ in chat(ws))
    assert not secret(ws, "rosebud") and not secret(ws, "sled"), "Element spoiler visible before a click"
    assert not secret(ws, "gem"), "||spoiler|| visible before a click"
    boxes = hidden_boxes(im)
    assert len(boxes) == 2, f"expected two hidden spoilers, found {boxes}"
    click(((boxes[0][0] + boxes[0][2]) // 2, (boxes[0][1] + boxes[0][3]) // 2))
    ws = locate("Element spoiler revealed", lambda ws: secret(ws, "rosebud") and ws)
    assert not secret(ws, "gem"), "revealing one spoiler showed the other"
    click(((boxes[1][0] + boxes[1][2]) // 2, (boxes[1][1] + boxes[1][3]) // 2))
    locate("||spoiler|| revealed", lambda ws: secret(ws, "gem"))
    if debug:
        capture().save(debug / "untethered-revealed.png")
    print("Both spoilers stayed hidden until clicked, and each click revealed only its own.", flush=True)

    # The code block's Copy button shows on hover and fills the clipboard.
    hover(zebra)

    def copy(ws):
        return next(((x, y) for w, x, y in chat(ws) if w == "copy" and zebra[1] < y < zebra[1] + 80), None)
    click(locate("Copy on the hovered code block", copy))
    locate("Copied", lambda ws: any(w == "copied" for w, _, _ in chat(ws)))
    click(composer)
    key("ctrl+v")
    # The composer grows upwards from the bottom; the timeline moves up too.
    # OCR reads some lines twice, so look for the code by position.
    pasted = locate("pasted code in the composer", lambda ws: next(
        ((x, y) for w, x, y in chat(ws) if "zebra" in w and y > composer[1] - 120), None))
    if debug:
        capture().save(debug / "untethered-pasted.png")
    print("Pasted code at", pasted, flush=True)
    key("ctrl+a")
    key("Delete")
    print("Copy put the code block's text on the clipboard; it pasted into the composer.", flush=True)
