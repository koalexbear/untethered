# Verify per-room Linux desktop notifications

Use the native debug build and [Element test environment](replies-element.md),
plus `dbus-daemon`, `dbus-monitor`, `busctl`, `xmessage`, Dunst and its sibling
`dunstctl`. Set `DUNST` if the binary isn’t on `PATH`. From the checkout:

```sh
cargo build --locked --manifest-path rust-client/Cargo.toml
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 DUNST=/path/to/dunst \
  /path/to/qa/venv/bin/python scripts/test-media-interop.py --room-alerts
```

The test runs Dunst on its own Xvfb display and session bus and checks the
bus owner PID. It creates two verified disposable accounts on a loopback
Synapse. Element sends Megolm messages with explicit `m.mentions`; the
receiver must actually decrypt each one. A separate window holds keyboard
focus during the background-delivery checks.

| Receiver setting and event | Expected desktop result |
|---|---|
| **All messages**, ordinary message | One visible popup |
| **Mute ordinary**, ordinary message or mention of another user | No `Notify` call |
| Same muted room, direct mention of the receiver | One visible popup |
| Untouched control room, ordinary message | One visible popup |
| Restarted client, mention in the focused room | No popup or history replay |
| Restarted client, backgrounded muted room | No popup for ordinary chat; one for a direct mention |

The fixture reads each positive popup on screen, checks the app name and
body in Dunst’s history, and counts the desktop service calls, including
the negative controls. It also reads the receiver’s room push rule from
Synapse and verifies **Account default** deletes that override.

## Verify browser tab notifications

Build the [web app](../README.md#browser) and use the same disposable
Synapse/Element and Linux desktop prerequisites above. Use a Playwright
version with `connect_over_cdp(no_defaults=True)`. For a loopback-only run:

```sh
bash scripts/rust-web.sh
ELEMENT_WEB_DIR=/path/to/element-v1.12.35 DUNST=/path/to/dunst \
  PYTHON=/path/to/qa/venv/bin/python \
  unshare -Urn sh -ec 'ip link set lo up; exec "$PYTHON" scripts/test-media-interop.py --browser-alerts'
```

The receiver uses Untethered’s Rust/WASM frontend and its matrix-sdk worker in a
fresh, headed Chromium profile on Xvfb. The sender is a different verified
user in Element. The fixture clicks **Mute ordinary** and reads that room’s
push rule from Synapse. It then drives **Enable browser notifications** and
the real Chromium **Allow** prompt. No notification constructor, permission
result or page visibility is mocked. A second tab is brought forward;
Playwright’s usual forced-focus emulation is disabled.

The receiving tab must be hidden and unfocused, with the worker reporting
decrypted message bodies and mention targets (the debug build's QA event
log, `scripts/qa_events.py`). Only the direct mention after
permission is granted may produce a popup. OCR reads its body on screen;
Dunst’s history must identify Chromium as the sender. The fixture activates
the popup, checks the tab regains focus, reloads, and sends another ordinary
message in the background. Across all controls there must be exactly one
`org.freedesktop.Notifications.Notify` call and one archived notification.

## Verify account-wide Do Not Disturb

With both frontends built and the same dependencies as above, run
`--account-dnd` instead of `--browser-alerts`. Two devices of the same
Matrix account run the native Linux and Chromium clients, each with its
own X display, session bus and Dunst instance. Element uses a separate
sender account. The Chromium tab is hidden and the native window unfocused
when test messages arrive. The test reads the actual decrypted events from
the native cache and browser worker, counts `Notify` calls, and reads the
positive notifications on both screens and in both daemon histories.

| Step | Result on each device |
|---|---|
| Direct mention in a room set to **Mute ordinary** | One baseline popup |
| Linux **User Settings → Pause alerts** | No popup for a direct mention in the muted room or ordinary chat in the other encrypted room |
| Restart the native client and reload Chromium | DND remains on. Another direct mention produces no popup. Both silenced mentions appear in the native inbox. |
| Chromium **User Settings → Resume alerts** | A fresh direct mention and ordinary chat in the other room each produce one popup. Ordinary chat in the muted room still produces none. |

Required total: **three** calls and three archived popups per receiver.
The only change to Synapse’s `m.push_rules` is the `enabled` flag of the
standard suppress-all `.m.rule.master` override. Resuming restores the
entire prior rule set. Untethered leaves a customized master action alone.

**Ringtones:** With the [Linux DM call test environment](../experiments/rtc-audio/README.md#replay-the-linux-desktop-dm-call),
run `sh experiments/rtc-audio/run_element.sh --dnd-ring`. While a real
Element call is ringing, another session enables DND. The private speaker
must fall silent for at least 800 ms, the playback stream must close, and
the **Accept** and **Decline** controls must remain. A new call under DND
must create zero ringtone streams. Accepting it opens one microphone,
delivers 480 Hz to Element and 960 Hz back to Linux, and yields two GCM
microphone tracks at the SFU.

## Verify space alerts

Build the native and web clients. In the [browser notification test environment](#verify-browser-tab-notifications),
run `scripts/test-media-interop.py --space-alerts native` and `--space-alerts web`.
Each test creates two disposable accounts: Element administers two nested
Matrix spaces and sends Megolm messages; the receiving account controls its
own alerts. The web run adds a separately signed-in, background Linux receiver
alongside the hidden Chromium tab, with independent Dunst servers on separate
X displays. Each enabled popup must be visible, counted on D-Bus and archived.

| Configuration or change | Observation |
|---|---|
| **Garden → Space alerts → Mute ordinary** | Ordinary messages in direct and nested channels produce no popup; a direct mention and ordinary chat outside Garden each produce one. |
| Explicit **All messages** on a direct child | One popup, despite Garden’s mute. Restarting Untethered preserves the space setting. |
| **All messages** on nested space Beds | Beds’ nearer preference wins for its child. Enabling account DND still suppresses even a direct mention. |
| Remove Beds’ last child; link it directly into Garden | It first returns to account defaults, then inherits Garden’s mute without restarting. |
| **Inherit** on Garden | The remaining inherited rules are removed. The explicit room override and unrelated user-created rule are unchanged. |

The fixture checks exact rule conditions, actions and enabled flags on Synapse,
and verifies decrypted message bodies and notification decisions at the clients.
Each recipient device must have exactly six `Notify` calls and six archived
popups. Unit tests also cover cycles, equally close conflicting parents,
unjoined spaces and a rejected rule update.

**How this maps to Matrix:** the preference is private room account data on
the space (`com.untethered.space_alerts.v1`). Untethered maintains a distinct
`underride` rule matching each affected room ID, after higher-priority
mention, explicit room and DND rules. It waits for initial sync to finish
before removing stale rules. Existing rules work on the homeserver while
Untethered is closed; changes to the space hierarchy are reconciled when an
Untethered device syncs again.
