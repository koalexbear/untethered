# System tray (ROADMAP N6)

A Linux runtime test of the tray icon, closing to the tray, the tray menu and
the two settings. Untethered runs on a private KWin virtual output with its
own session bus, where KDE's `kded6` provides the StatusNotifierWatcher the
icon registers with; `--tray plasma` also starts Plasma's panel, which draws
the icon. The fixture then acts as the tray host over D-Bus, as Plasma's tray
does: it reads the item's tooltip and icon pixmaps and clicks its menu
entries through `com.canonical.dbusmenu`. Element Web, a different Matrix
account, messages and rings Untethered in an encrypted DM. Nothing touches
the real desktop: the autostart entry goes to the test's own
`XDG_CONFIG_HOME`.

With the [Linux DM call test environment](../experiments/rtc-audio/README.md#replay-the-linux-desktop-dm-call)
and the KWin tools from [kwin-element.md](kwin-element.md) (`kwin_wayland`,
`kdotool`, `spectacle`, KDE inputsynth as `KWIN_INPUTSYNTH`), plus `kded6`
and, for `plasma`, `plasmashell`. Build both clients first: the debug build
seeds the account, the release build runs the UI (winit panics under
synthetic input in debug builds).

```sh
cargo build --manifest-path rust-client/Cargo.toml
cargo build --release --manifest-path rust-client/Cargo.toml
sh experiments/rtc-audio/run_element.sh --tray plasma
```

The fixture runs the release build when there is one (`UNTETHERED_QA_UI_CLIENT`
picks another).

`KWIN_QA_DEBUG=<dir>` keeps KWin screenshots (`tray-closed.png`,
`tray-ringing.png`, `tray-call.png` with Plasma) and the client's, kded's and
Plasma's logs.

| Step | Required observation |
|---|---|
| Start | One item registered with the watcher: Id `io.github.koalexbear.Untethered`, Title `Untethered`, a `/MenuBar` menu. |
| Close the window (`kdotool windowclose`) | The window goes; the process and its item stay. |
| Element sends a DM | Tooltip `1 mention`; the 32 px icon has the red dot at the top right. |
| Element rings | With no window: the 440/620 Hz ringtone on the private speaker, tooltip `Incoming call from …`, status `NeedsAttention`, a green dot, and an **Answer …** menu entry. |
| **Answer …** | Both join; the window opens; Element decodes the desktop's 480 Hz and the desktop plays Element's 960 Hz. |
| Close during the call, Element sends another DM | Both tones carry on for 12 fresh 100 ms windows; tooltip `In a call: media-proof` and `1 mention`; red and green dots. |
| **Mute** | Element measures silence for 12 fresh windows; capture closes; Mute is checked; the call dot turns grey; both stay in the call. |
| **Quit Untethered** | Exit status 0 well inside the 10 s deadline; Element's roster loses the desktop and Element ends the call. |
| User Settings → System tray | Defaults: Close to tray on, Start at sign-in off. Switching both writes `desktop.json` (`{"close_to_tray": false}`) and the autostart entry, whose `Exec` ends in `--start-minimized`. |
| Close with Close to tray off | Quits with status 0, at once and again after a restart. |
| Run the entry's `Exec` | No window; the item registers; **Activate** opens the window; both switches kept their state; switching Start at sign-in off removes the entry. |

## Edge cases without a homeserver

`python scripts/qa_tray.py` runs the signed-out preview on a private KWin
(release build, `KWIN_INPUTSYNTH` as above). Without a StatusNotifierWatcher
on the bus, closing the window must quit with status 0, and a
`--start-minimized` start must show no window for 3 s and then open it once
the tray retries give up (about 10 s). With kded6's watcher, the same start
shows no window until **Activate**, which opens it focused; after
`kdotool windowminimize`, **Activate** must bring it back focused, as a new
window (Wayland can't restore or raise a window without an activation
token), with only one Untethered window left. Finally, with the window closed
to the tray, starting the client again on the same profile must exit 0 and
bring the running window back focused; a `--start-minimized` start must leave
it alone; after `kdotool windowminimize`, starting it again must bring it back
as a new window. One process, one window and one tray item throughout.

## Recorded result

2026-11-26 on KWin 6.8.3 and Plasma 6.8 with Element Web 1.12.35, release
build: every step in both tables passed. Quit exited 0.2 to 0.4 s after the
click across runs; a hidden start without a tray opened its window after
10 to 16 s at load averages of 60 to 300. One earlier run under that load
missed the two-way audio window after Answer (then 15 s; now 30 s with the
measurements printed on failure). Not covered: GNOME's AppIndicator
extension and Windows' notification area (the Windows build is only
cross-compiled).

Later on 2026-11-26, with single-instance launching: the edge cases passed
again, and each second start handed over and exited 0 in under 0.1 s. Not
covered live: a start while the running one quits (unit tests in
`rust-client/src/instance.rs`), and Windows, where the second start lets the
running one take the foreground (`AllowSetForegroundWindow`).
