# Verify read state across Linux, Chromium and Element

Build the [native and web clients](../README.md) and use the same local
Synapse, Element Web, Playwright, Xvfb, xdotool, ImageMagick and Tesseract
environment as the [browser notification test](room-alerts.md). This
fixture does not require desktop notification permission or Dunst.

```sh
cargo build --locked -j2 --manifest-path rust-client/Cargo.toml
bash scripts/rust-web.sh
export ELEMENT_WEB_DIR=/path/to/element-v1.12.35
export PYTHON=/path/to/qa/venv/bin/python
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --read-state'
```

The same disposable account runs a native Linux client and a distinct
Chromium device. A second verified account uses Element Web. The fixture
checks the visible controls, real focus/visibility, decrypted messages,
the homeserver’s `m.fully_read` and `m.marked_unread` account data, and its
`m.read.private` and `m.read` receipt rows. It also checks the event ID that
Element sees for the recipient's public read receipt.

| Workflow | Acceptance evidence |
|---|---|
| Read the latest message | After 55 older posts, neither a hidden tab nor an unfocused or scrolled-up native window advances the private receipt. Focusing the native timeline at the bottom advances it to the exact event. |
| Read on the second device | A new mention appears after the native **NEW** divider. Keeping Chromium in search leaves the marker unchanged; returning to the timeline advances it and clears the native unread state. |
| Save a reminder | **Mark unread** sets room account data. Restarting Linux, reloading the web app, and receiving another message do not discard it. **Mark read** on web clears it and advances to that new event. The untouched encrypted room keeps its highlight badge. |
| Publish only when requested | The private reads do not reach Element. After opting in on web, Element observes the exact next event read on Linux. After opting out, the public receipt remains at that event while the private receipt advances. |
| Edit at the timeline end | A visible edited message marks the replacement event read, even though it has no separate rendered row. The next message appears after the unread divider. |
| See another reader | The fixture verifies a public receipt from Element on a native-authored message in Synapse. Linux and Chromium both display **Seen by Cedar** beneath that message. |
