# KWin / Dolphin ↔ Element file round trip

This is a Linux runtime test. It starts an actual KWin virtual output with
OpenGL compositing, its own session bus, Dolphin and a fresh native Matrix
profile. It does not start XWayland, use the primary compositor's socket,
or inject input into the user's desktop. It gives Element Web a **different
Matrix account** in a fresh Chromium profile, then invites that user into
the encrypted room. All services and files are disposable.

Prerequisites: `kwin_wayland` with virtual OpenGL output support, `dolphin`,
`spectacle`, `kdotool`, `qdbus6`, KDE portal/file chooser, `tesseract`, Chromium,
and KDE's `selenium-webdriver-at-spi-inputsynth` with its matching installed
desktop file. `KWIN_INPUTSYNTH` can point to a **private installation prefix**;
the fixture adds its `share` directory to the test's `XDG_DATA_DIRS`.
Tested with KWin 6.8.3, Dolphin 26.08.3, and KDE inputsynth built at
`4199d3ea3ccdcaa175b5a06ed3ad8f1af498f38a`. These are test tools, not
Untethered dependencies.

Set `ELEMENT_WEB_DIR` to the extracted official `element-v1.12.35.tar.gz`
(SHA-256 `5aaff32e348eba976aea8da11cd50f9ca7f824f235aa66e24d1176f829e6bba5`).
The fixture serves the release locally with its own config, without changing
the extracted files. Build the native debug client first and set
`UNTETHERED_BACKEND` if the pinned gomuks binary isn't already in `.dev/backends`.

```sh
cargo build --locked -j2 --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow --with cryptography \
    python scripts/test-media-interop.py --kwin-element
```

The test selects a PNG and PDF together in Dolphin and physically drags them
into Untethered, checking the Wayland transfer request and staged composer
cards. It checks that nothing was uploaded before Send, switches rooms while
the first upload is held, and compares Element's decrypted timeline event IDs
with the homeserver's encrypted events. It clicks Element's download controls,
compares both files, then uploads those downloaded bytes back under new names.
Finally it uses **Save attachment** in the native timeline and the KDE file
chooser, compares both downloads byte-for-byte, and checks sender, room, and
event counts. `KWIN_QA_DEBUG` optionally names a directory for test-only
compositor screenshots and diagnostic logs.

## Verify plain rooms and upload limits

Run `--file-limits native` in the KWin test environment above and
`--file-limits web` after building the web app with `bash scripts/rust-web.sh`.
The web test uses Chromium's real multi-file chooser and downloads; it
does not need KWin. Both variants work in a loopback-only namespace:

```sh
export PYTHON=/path/to/qa/venv/bin/python
export ELEMENT_WEB_DIR=/path/to/extracted/element-v1.12.35
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --file-limits native'
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --file-limits web'
```

Each run checks that the room has no encryption state and that Synapse's
authenticated media config advertises a **65,536-byte** limit. Submit one
selection containing `Map.png`, `Plan.pdf` and `Oversize.bin`. Verify exactly
two ready cards and a named rejection with the file size and limit, before
any upload or message. In the browser, first try the oversized file alone
with a caption: neither Enter nor the Send button may post that caption.
The mixed batch then delivers the retained caption on the first image.

Element downloads the two originals with its visible controls and sends the
downloaded files back. The receiver saves each through the KDE save dialog
or Chromium's download UI after reloading. Independently, the fixture
checks authenticated downloads from all four `mxc://` descriptors, event
type, sender, filenames, and exact byte counts, and counts the actual
upload bodies at the gateway.

| File | Size | Recorded result on Linux and Chromium, 2026-11-25 |
|---|---:|---|
| Map.png | 437 bytes | Round-trip SHA-256 `2ab7aaf5397c9315b797cce0ee29435f0596d94145bb5ccb8b44b7af4f450981` |
| Plan.pdf | 65,536 bytes | Round-trip SHA-256 `36aeab87b3b98bc1372d84a6b5d74b1d07f2a495bc53349cf7cd1a66912e887b` |
| Oversize.bin | 65,537 bytes | Skipped; zero upload requests or room events for this file |

An additional Rust test delivers a smaller server limit *after* files have
been staged in two rooms. Send must wait for the config response; the new
limit removes only the oversized files and preserves the caption.
