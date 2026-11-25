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
request. After the native client and gomuks restart and Element reloads,
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
