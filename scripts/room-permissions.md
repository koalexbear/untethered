# Verify room posting permissions

Use the [native Iced and Element test environment](replies-element.md):

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --room-permissions
```

The fixture creates an encrypted room on loopback Synapse with two disposable
accounts: the room administrator in Untethered and an ordinary member in
Element. It reads Element’s actual `/send/m.room.encrypted` HTTP responses,
checks the room timeline independently, and reads the delivered text on the
native display.

| State | Observation |
|---|---|
| Initially writable | The member sends an encrypted message; HTTP 200 and visible on both clients. |
| **Make read-only** | Both encrypted and plain member sends return HTTP 403 `M_FORBIDDEN`. Neither enters the room history. |
| Still read-only | The member’s power-level write is also forbidden. An administrator’s encrypted announcement reaches Element. |
| **Restore posting** after a native restart | The member receives HTTP 200 for a new send, and Untethered decrypts it. The denied attempts are absent. |

The preset raises `events_default` and any lower explicit levels for
`m.room.message`, `m.room.encrypted`, `m.sticker` and `m.reaction`. The floor
is 50, or one above `users_default` if that is higher. Other explicit event
rules and user roles retain their values. Before writing `m.room.power_levels`,
Untethered re-reads room state and checks that the power-level event ID still
matches the one used to display the control.

`com.untethered.posting_restore.v1` in the power-level content records only
the changed posting fields and their previous values. Restore removes this
record and restores fields still at the preset level. The test compares the
entire resulting document to the baseline, including an originally absent
`events_default`, except for two later changes made by the administrator’s
other session: a custom audit rule and a stricter reaction rule.

## Verify ban, unban and reinvitation

Using the same Element release and Python environment:

```sh
export ELEMENT_WEB_DIR=/path/to/element-v1.12.35
export PYTHON=/path/to/qa/venv/bin/python
"$PYTHON" scripts/test-media-interop.py --room-ban
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --federated-ban'
```

The second run starts two Synapse homeservers with distinct identities,
databases and signing keys. They federate with one another using temporary
TLS certificates in a loopback-only network namespace (requires OpenSSL,
`unshare` and `ip`). The first run puts both accounts on one Synapse.

The admin selects the member’s row in Iced, enters a reason and clicks
**Ban**. The fixture checks membership and reason on Synapse and captures
the banned user’s actual join response from Element. The room is public at
that point, so invite-only rules cannot explain the refusal. Synapse 1.166
returns `M_BAD_STATE` from its local join handler and `M_FORBIDDEN` on the
federated join path, both with HTTP 403 and an explicit ban explanation.

The admin restarts Untethered and selects the persisted banned member. The
test switches to invite-only before **Unban**: unbanning must yield `leave`,
and joining without an invitation must still fail. **Invite** must yield
`invite`; the same Element account then joins and posts a fresh message.
The fixture checks the send response, Megolm event, decrypted text on both
screens, and the actual membership sequence `ban → leave → invite → join`.

## Verify kick in a public room

Run `"$PYTHON" scripts/test-media-interop.py --room-kick` with the same
native debug build and Element environment. Both disposable accounts use
one loopback Synapse. Public joining is enabled before the member’s first
join; no invite is sent at any point.

The member first posts a message visible in Element and Untethered.
The test holds a second, freshly encrypted Element request before it reaches
the network. The administrator enters a reason and clicks **Kick** in the
People panel. Once Synapse and both clients show `leave`, the test releases
the encrypted request and also submits a new plain-text send as the member.
Both must return HTTP 403 `M_FORBIDDEN` from Synapse, and neither may appear
in room history.

The same Element account must then rejoin directly and post a fresh Megolm
message. The fixture checks both HTTP 200 responses, the event ID and text
on both clients, unchanged join rules and power levels, and a final
moderator-authored `leave` followed by a member-authored `join`, with no ban
or invitation in the member’s history.

## Verify message reports

Build the native debug client and the web frontend (`bash scripts/rust-web.sh`).
With the Element release and QA Python environment above, run:

```sh
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --message-report native'
unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --message-report web'
```

Each run starts a fresh Synapse. The reporter has room power level **0**
and gets HTTP **403** if they try to read the moderation queue. The sender
uses Element; the reporter uses either the native Iced client on Xvfb or a
fresh Chromium tab running the Rust/WASM client and gomuks worker.

1. Element sends two identical messages. Open **Report** on the first and
   cancel after typing a reason: the server still has zero reports.
2. Open **Report** on the second. Before submitting, Element edits that
   message. Once the new text is visible, submit the open form. The queue
   must contain the **second original event ID**, not the look-alike or edit.
3. Open a fresh report on the edited text and press Enter twice. The queue
   must gain exactly one report for the **replacement event ID**.

A separate reviewer account reads both `/_synapse/admin/v1/event_reports`
and the individual report details. The fixture checks the exact room,
event, reporter, sender, and reason; the referenced event still contains
Megolm ciphertext. Room history contains only the original pair and their
one edit. The reviewer’s credentials are never provided to either client.
