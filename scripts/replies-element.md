# Verify encrypted replies and edits with Element

Build the native debug client. Use a local copy of Element Web (tested with
1.12.35), Chromium, Xvfb, xdotool, ImageMagick, Tesseract and the Python QA
environment from `test-media-interop.py` (including `cryptography`). Run from
the checkout:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --replies
```

The runner creates a loopback Synapse, two verified Matrix accounts, a fresh
Chromium context and a private X display. Element sends two identical
originals, each followed by a different marker. The test selects the first
using the **Reply** control in Untethered’s message hover toolbar and the second using Element’s. It inserts
80 real encrypted filler messages before sending either reply and checks the
decrypted `m.in_reply_to.event_id` values in Element. Using the native inline
editor, it edits the first reply twice. Using Element’s editor, it edits the
second once. Synapse must have exactly 89 distinct Megolm events, including
three replacements. Both native edits must target the first reply’s ID rather
than the preceding edit, and the Element edit must target the other reply.

A native jump while the timeline is still loaded must use no `/context`
request. After the native client restarts and Element reloads,
both clients must display the two final texts on the original reply rows, and
neither original may be in Element’s initial live timeline. Clicking each
edited quote in Element must highlight the exact original. Each click on a native reply’s quoted line must
make a room-scoped GET for the corresponding event ID and show that original
with the correct neighboring text. The fixture also drives **Preceding
messages** and **Back to latest**.

## Reactions on an edited reply

Run the same command with `--reactions` instead of `--replies`. This shorter
fixture creates an encrypted parent, reply, edit and sibling reply. It opens
Untethered’s **Add Reaction** picker on the edited text and chooses 👍 (**Like**). Element
clicks the 👍 chip on the same reply. The test reads both visible counts and
queries Synapse’s relations endpoint: exactly two senders on the base reply
ID, none on the edit or neighboring messages.

Both clients reconnect with the count at 2. Untethered then clicks the chip
to remove its own reaction. The test checks which annotation was redacted
and compares the surviving event ID and sender to Element’s original
annotation. A second restart/reload must still show 1 on both clients;
Untethered offers **Add your reaction**, while Element’s chip remains selected.

## Composer autocomplete

Run the same command with `--composer-mentions`. Element's account is
renamed **Mira** before the native client starts. In the native composer the
fixture types `Ping @mi` and **Tab**, `see #med` and **Enter**, `party :tad`
and **Tab**, then `then :wave: done`, and checks that each popup listed the
expected row. Element must decrypt exactly that body with Mira's name, the
room's `matrix.to` link (`via` the native user's server), 🎉 and 👋, an
`m.mentions` of Mira's ID only, and both links in `formatted_body`; its tile
must be highlighted with two pills.

The native inline editor then appends `for @mi` + **Tab** to a message that
mentioned nobody. The replacement's new content carries the pill and
Mira's ID, and so does its outer `m.mentions`, because the mention is new;
Element highlights the original tile. Finally a native reply to an Element
message types `*literal* @mi` + **Tab**: the body and `formatted_body` keep the
asterisks literal around the pill. Set `COMPOSER_QA_SHOTS=<dir>` to keep
screenshots of the popups.

## Typing indicators

Run the same command with `--typing` (ROADMAP M3), or `--typing web` for
the browser build (after `scripts/rust-web.sh --release`; Chromium drives it
instead of xdotool). Element's account is renamed **Mira**. The fixture reads
the line under Untethered's composer and asks Element's client whether the
Untethered user is typing
(`RoomMember.typing`), timing each change from the key press that caused it
to the first screenshot or check that sees it:

| Step | Required observation |
|---|---|
| Element types one character | **Mira is typing…** under Untethered's composer within 1.5 s (allowing for the screenshot and OCR). The Untethered user never appears in it. |
| Element empties its composer, then types and sends | The line goes back to the room's note each time. |
| A third account, **Jo**, joins and types through the API while Element types | **Mira and Jo are typing…**: Jo's name comes from the membership, as Jo hasn't posted. When Jo stops, **Mira is typing…** again. |
| Untethered's composer gets one character | Element sees the Untethered user typing. It still does after a 3 s pause, and stops 4 to 9 s after the last key (the client stops after 5 idle seconds). |
| The draft is emptied (Ctrl+A, Backspace), or sent | Element sees the typing stop. |
| **User Settings → Privacy → Send typing indicators** off | The account data `com.untethered.typing_notices.v1` becomes `{"send": false}` and the switch shows off; 7 s of typing a draft shows nothing in Element. |
| The account data set back to `{"send": true}` (as another device would) | Typing shows in Element again. |

`TYPING_QA_SHOTS=<dir>` saves the client's window when a step times out;
`TYPING_QA_TRACE=1` prints both clients' view at every poll of the
two-typer step. In the browser run, headless Chromium draws WebGL on SwiftShader, where the
client's SVG icons don't appear and the typing line's first glyph is
sometimes garbled; that run also accepts a name without its first letter
("ira" for Mira).

Recorded result, 2026-11-27, Element Web 1.12.35, native debug build and
the release browser build, at load averages of 60 to 235: every step passed
in both. Element's typing showed under the composer 0.15 to 0.64 s after its
first key (most of that is the screenshot and OCR; the browser's line was
there on the first screenshot) and cleared within 0.55 s after Element
emptied its composer or sent. Element saw Untethered's typing within 0.07 s
of the first key, the idle stop came 5.4 s after the last key, and emptying
or sending the draft stopped it within 0.11 s.
