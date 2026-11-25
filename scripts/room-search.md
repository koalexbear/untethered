# Verify room-scoped local search

Build the native debug client and use the [Element interoperability test
environment](replies-element.md). From the checkout:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --room-search
```

Element sends a reply, changes its wording to “Saffron comet launch,” and
adds 150 newer messages. A second encrypted room contains “Saffron comet
diversion.” The fixture checks that the native device decrypted both, then
restarts Untethered and gomuks and scrolls away from the bottom of the
recent timeline. It records the visible ledger rows and their pixel positions.

The Iced search must report 153 readable messages in the selected room:
parent, reply, neighbor and 150 ledger entries. The edit is not an additional
message. Searching the retired wording must return no matches; searching
“saffron comet” must return the edited reply alone. A loopback proxy checks
that searching makes no Matrix search, event lookup or context request.

Opening the hit must request `/context` for the **base reply**, render its
edited body, identify the quoted parent and show the neighboring message.
**Back to results** must retain the hit. While the user is in context again,
three messages arrive and an offscreen row above the bookmark grows to
five lines. **Return to timeline** must restore the same ledger row within
one pixel of its prior screen position.
