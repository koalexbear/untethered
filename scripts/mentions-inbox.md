# Verify the cross-room mentions inbox

Use the [native Iced and Element test environment](replies-element.md).
Build the native debug client and web frontend first:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
bash scripts/rust-web.sh
export ELEMENT_WEB_DIR=/path/to/element-v1.12.35
export PYTHON=/path/to/qa/venv/bin/python
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --mentions-inbox native'
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --mentions-inbox web'
```

Each run starts a loopback Synapse and two disposable accounts. Element sends
messages to two Megolm rooms. The receiver opens **Mentions** in the native
client on Xvfb or in a Chromium tab running the WASM frontend and its
matrix-sdk worker. The fixture reads the actual rendered entries and clicks each one.

| Check | Required observation |
|---|---|
| Direct mentions with identical bodies | One entry per event, with the correct room name. |
| Ordinary messages, literal Matrix ID, other recipient, and room-wide ping | No entries. |
| Add or remove the recipient through an edit | One entry for the newly addressed base message, none for the removed address. |
| 65 newer non-recipient messages in each room | Synapse shows 65 distinct Megolm events per room. The receiver’s local search counts 70 readable base messages per room. |
| Native process restart or close/reopen Chromium tab on the same origin | Exactly the same three entries. Their reconstruction does not request homeserver event context. |
| Select each entry | The gateway sees precisely `/rooms/{room_id}/context/{event_id}` for that message, including the **base** for the edited hit. Both the highlighted text and the expected next message appear on screen. |
| Leave the second room while viewing the inbox | The visible count drops from three to one; the first room’s entry stays. |

The inbox reads the messages the matrix-sdk event cache stored on this
device (`engine/search.rs`). It checks the decrypted `m.mentions.user_ids` array and the current
valid edit, rather than inferring a mention from a notification or a text
match. It does not store a second plaintext history. Messages the device
has not synced or cannot decrypt are outside its coverage.
