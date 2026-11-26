# Verify rich text against Element

Build the native debug client and use the [Element interoperability test
environment](replies-element.md). From the checkout:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 RICH_QA_DEBUG=/tmp/rich \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --rich-text
```

Element types four messages into its own composer, so the HTML is exactly
what Element sends: inline bold, italic, strikethrough, inline code and a
link; a heading, a quote, a bulleted and a numbered list; a fenced Rust code
block; and `/spoiler`. The fixture checks each decrypted `formatted_body`
for the expected tags. A fifth message has only a plain `body`, as from a bot:
Markdown bold and a `||spoiler||`.

In Untethered, OCR must find every word without its Markdown markers, and the
link, inline code background, quote bar and code block background must have
their colours. Both spoilers must be unreadable; clicking each reveals only
that one. Hovering the code block shows **Copy**; after a click it reads
**Copied**, and Ctrl+V in the composer pastes the code.

`RICH_QA_DEBUG` keeps `element.png`, `untethered.png` and
`untethered-revealed.png` to compare the two clients by eye.
