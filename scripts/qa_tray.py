"""System tray (ROADMAP N6), live: Untethered runs on a private KWin with KDE's
StatusNotifierWatcher (kded6, optionally Plasma's panel), Element Web is the
other person in an encrypted DM. The fixture plays the tray host over D-Bus,
as Plasma's tray does: it reads the item's tooltip and icon and clicks its
menu entries. Run with `scripts/test-media-interop.py --tray` (see
scripts/tray.md)."""
import array
import json
import math
import os
import re
import subprocess as sp
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

from qa_desktop_dm import element_microphone, sfu_audio
from qa_kwin import KWin
from qa_native_paste import child, isolated_env, wait_for
from qa_ui import RAIL, SIDEBAR, SIDE, USER_PANEL, clean, read

RED = (0xF2, 0x3F, 0x43)
GREEN = (0x23, 0xA5, 0x5A)
GREY = (0x80, 0x84, 0x8E)
# Short enough that the user panel stays above Plasma's panel (the screen is 1000 px).
WIDTH, HEIGHT = 1180, 900
TITLE_BAR = 28  # KWin's decoration above the window's content.


class Tray:
    """The client's StatusNotifierItem, read and clicked as a tray host would."""

    def __init__(self, kde):
        self.kde = kde
        self.bus = "--address=" + kde.env["DBUS_SESSION_BUS_ADDRESS"]
        item = wait_for("the tray item registered with KDE's watcher", self.items, 30)
        assert len(item) == 1, item
        self.service, path = item[0].split("/", 1)
        self.path = "/" + path
        assert self.prop("Id") == "io.github.koalexbear.Untethered" and self.prop("Title") == "Untethered"
        self.menu = self.prop("Menu")

    def busctl(self, verb, *args, check=True):
        r = self.kde.command("busctl", self.bus, "--json=short", verb, "--", *args, check=check)
        return json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else None

    def items(self):
        items = self.busctl("get-property", "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher",
                            "org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems", check=False)
        return items and items["data"]

    def registered(self):
        return any(i.startswith(self.service + "/") for i in self.items() or [])

    def prop(self, name):
        return self.busctl("get-property", self.service, self.path, "org.kde.StatusNotifierItem", name)["data"]

    def summary(self):
        """The tooltip's text under the app's name."""
        return self.prop("ToolTip")[3]

    def pixel(self, x, y, size=32):
        """The icon's colour at (x, y) of the unit square, if opaque."""
        for w, h, data in self.prop("IconPixmap"):
            if w == size:
                i = (int(y * h) * w + int(x * w)) * 4
                a, r, g, b = data[i:i + 4]
                return (r, g, b) if a == 255 else None
        raise AssertionError(f"no {size}px pixmap")

    def entries(self):
        layout = self.busctl("call", self.service, self.menu, "com.canonical.dbusmenu",
                             "GetLayout", "iias", "0", "-1", "0")["data"][1]
        found = {}
        for node in layout[2]:
            cid, props, _ = node["data"]
            props = {k: v["data"] for k, v in props.items()}
            if "label" in props:
                found[props["label"]] = (cid, props)
        return found

    def click(self, prefix):
        label, (cid, props) = next((l, e) for l, e in self.entries().items() if l.startswith(prefix))
        assert props.get("enabled", True), f"{label} is disabled"
        self.busctl("call", self.service, self.menu, "com.canonical.dbusmenu", "Event", "isvu", str(cid), "clicked",
                    "s", "", "0")
        return label

    def activate(self):
        self.busctl("call", self.service, self.path, "org.kde.StatusNotifierItem", "Activate", "ii", "0", "0")


def tray_call(stack, tmp, page, room, root, backend, alice, bob, secret, host="watcher"):
    from playwright.sync_api import Error as BrowserError
    env = isolated_env(tmp, backend, "wayland")
    # Start at sign-in and close to tray must never touch the real desktop.
    sock = Path(env["XDG_RUNTIME_DIR"]) / "qa-pulse"
    env.update(PULSE_SERVER="unix:" + str(sock), PULSE_SOURCE="QAMic.monitor", PULSE_SINK="QASpeaker")
    def cmd(*args, **kw):
        return sp.run(args, env=env, capture_output=True, timeout=8, **kw)
    pulse = stack.enter_context(child(["pulseaudio", "-n", "--daemonize=no", "--exit-idle-time=-1", "--use-pid-file=no",
        "--load=module-native-protocol-unix socket=" + str(sock) + " auth-anonymous=1",
        *(f"--load=module-null-sink sink_name={name} sink_properties=device.description={label} rate=48000 channels=1"
          for name, label in (("QAMic", "Amber"), ("QASpeaker", "Harbor")))], env))
    def audio_ready():
        assert pulse.poll() is None, "private audio server exited"
        return sock.is_socket() and "QASpeaker" in cmd("pactl", "list", "short", "sinks", text=True).stdout
    wait_for("private PulseAudio sinks", audio_ready)

    # The desktop's microphone plays 480 Hz; its speaker is measured in 100 ms
    # windows: Element's 960 Hz, and the 440/620 Hz ringtone.
    pcm_args = ["--raw", "--rate=48000", "--channels=1", "--format=s16le", "--latency-msec=40"]
    tone = array.array("h", (round(9000 * math.sin(i * math.tau * 480 / 48000)) for i in range(4800))).tobytes()
    speaker = {"windows": 0, "run960": 0}
    chime = threading.Event()
    def inject(feed):
        try:
            while True:
                feed.stdin.write(tone)
        except (OSError, ValueError):
            pass
    def amplitude(pcm, hz):
        w = math.tau * hz / 12000
        return 2 * abs(sum(v * complex(math.cos(k * w), math.sin(k * w)) for k, v in enumerate(pcm[::4]))) / len(pcm[::4])
    def observe(monitor):
        while data := monitor.stdout.read(9600):
            pcm = array.array("h"); pcm.frombytes(data)
            hz = sum(a <= 0 < b for a, b in zip(pcm, pcm[1:])) * 48000 / len(pcm)
            good = abs(hz - 960) < 35 and max(pcm) > 500
            speaker["run960"] = speaker["run960"] + 1 if good else 0
            speaker["windows"] += 1
            if max(pcm) > 500 and all(amplitude(pcm, f) > 600 for f in (440, 620)):
                chime.set()
    def end(p):
        p.terminate(); p.wait(timeout=5)
    feed = sp.Popen(["pacat", "--playback", "--device=QAMic", *pcm_args], stdin=sp.PIPE, bufsize=0, env=env, stderr=sp.DEVNULL)
    stack.callback(end, feed)
    monitor = sp.Popen(["pacat", "--record", "--device=QASpeaker.monitor", *pcm_args], stdout=sp.PIPE, env=env, stderr=sp.DEVNULL)
    stack.callback(end, monitor)
    threading.Thread(target=inject, args=(feed,), daemon=True).start()
    threading.Thread(target=observe, args=(monitor,), daemon=True).start()

    debug = os.environ.get("KWIN_QA_DEBUG")
    # The release build once the debug build has seeded the account: winit
    # panics under synthetic input in debug builds (the settings clicks).
    release = root / "rust-client/target/release/untethered-iced"
    ui_client = os.environ.get("UNTETHERED_QA_UI_CLIENT") or (str(release) if release.is_file() else None)
    if ui_client:
        os.environ["UNTETHERED_QA_CLIENT"] = ui_client
    kde = KWin(stack, tmp, root, backend, env=env, tray="plasma" if host == "plasma" else True)
    tray = Tray(kde)
    def window():
        assert kde.native.poll() is None, "desktop exited"
        return kde.window("Untethered.*media-proof")
    win = wait_for("desktop window on the verified DM", window, 90)
    print(f"Tray item {tray.service}{tray.path} registered with KDE's StatusNotifierWatcher"
          f"{' and shown by Plasma' if host == 'plasma' else ''}.", flush=True)
    def shot(name):
        if debug and host == "plasma":
            im = kde.capture(); im.save(Path(debug) / f"tray-{name}.png")
    def close():
        kde.command("kdotool", "windowclose", win)
        wait_for("window closed", lambda: not window(), 10)
        time.sleep(1)
        assert kde.native.poll() is None and tray.registered(), "closing the window quit Untethered"
    def roster():
        return page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get();
          return c.matrixRTC.getRoomSession(c.getRoom(room)).memberships.map(m=>m.userId)}""", room)
    def streams(kind):
        return [s for s in json.loads(cmd("pactl", "-f", "json", "list", kind, check=True).stdout)
                if s["properties"].get("application.name") == "Untethered"]
    def message(text):
        page.evaluate("([room,text])=>window.mxMatrixClientPeg.get().sendTextMessage(room,text)", [room, text])

    close()
    shot("closed")
    print("Closed the window: still running, the tray item stays registered.", flush=True)

    # Every unread direct message counts as a mention.
    message("Are you around?")
    wait_for("mention badge", lambda: tray.summary() == "1 mention" and tray.pixel(.78, .22) == RED, 30)
    print("A DM from Element while closed: tooltip '1 mention', red dot on the icon.", flush=True)

    def ring_ids():
        return page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room).getLiveTimeline().getEvents()
            .filter(e=>e.getType()==="org.matrix.msc4075.rtc.notification"&&e.getContent().notification_type==="ring")
            .map(e=>e.getId())""", room)
    before = set(ring_ids())
    page.get_by_role("button", name="Voice call", exact=True).click(timeout=15000)
    def ringing():
        for f in page.frames:
            if f == page.main_frame:
                continue
            button = f.get_by_role("button", name=re.compile(r"^(Start call|Join call|Join)$", re.I))
            try:
                if button.count() and button.first.is_visible():
                    button.first.click(timeout=1500)
            except BrowserError:
                if not f.is_detached():
                    raise
        return roster() == [bob] and bool(set(ring_ids()) - before)
    wait_for("Element originated the DM ring", ringing, 22)
    wait_for("ringing in the tray", lambda: tray.summary().startswith("Incoming call from ")
             and tray.prop("Status") == "NeedsAttention" and tray.pixel(.8, .8) == GREEN, 12)
    assert chime.wait(4), "no 440/620 Hz ringtone on the desktop speaker with the window closed"
    assert not window() and not streams("source-outputs")
    entries = tray.entries()
    shot("ringing")
    print(f"Element rang with the window closed: 440/620 Hz ringtone played; tooltip "
          f"{tray.summary().splitlines()[0]!r}; status NeedsAttention; menu offers {next(l for l in entries if l.startswith('Answer'))!r}.",
          flush=True)
    element_microphone(page, bob, secret)
    tray.click("Answer")
    wait_for("Element and Untethered joined the answered call", lambda: set(roster()) == {alice, bob}, 20)
    win = wait_for("window opened by Answer", window, 10)
    widget = next(f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/"))
    def stats():
        return widget.evaluate("window.qaAudioStats()")
    def flowing(windows=12):
        # Fresh windows only: both directions, measured after this point.
        first, start = stats()["spectra"], speaker["windows"]
        def match():
            s = stats()
            return min(s["spectra"] - first, speaker["windows"] - start, s["run480"], speaker["run960"]) >= windows
        try:
            wait_for("two-way call audio", match, 30)
        except RuntimeError:
            print("Element's PCM stats:", stats(), "desktop speaker:", speaker, "SFU:", sfu_audio(secret), flush=True)
            raise
    flowing()
    tracks = sfu_audio(secret)
    assert len(tracks) == 2 and all(v == ["GCM"] for v in tracks.values()), tracks
    print("Answer in the tray menu: call joined, the window opened; Element decodes the desktop's 480 Hz, "
          "the desktop plays Element's 960 Hz.", flush=True)

    close()
    flowing()
    message("Still there?")
    def call_and_badge():
        s = tray.summary()
        return ("In a call: " in s and "1 mention" in s and tray.pixel(.78, .22) == RED
                and tray.pixel(.8, .8) == GREEN and s)
    summary = wait_for("call and mention in the tray", call_and_badge, 30)
    shot("call")
    print(f"Closed during the call: both tones carry on for 12 fresh windows; tooltip {summary!r}, "
          "red mention dot and green call dot.", flush=True)

    tray.click("Mute")
    first = stats()["spectra"]
    wait_for("Element hears silence", lambda: (s := stats())["spectra"] - first >= 12 and s["quiet"] >= 12, 15)
    wait_for("muted in the tray", lambda: "(muted)" in tray.summary() and tray.pixel(.8, .8) == GREY
             and tray.entries()["Mute"][1].get("toggle-state") == 1, 8)
    assert len(streams("source-outputs")) == 0 and set(roster()) == {alice, bob}
    print("Mute in the tray menu: Element measures silence for 12 fresh windows, capture closed, "
          "Mute checked, grey call dot; still in the call.", flush=True)

    started = time.monotonic()
    tray.click("Quit")
    code = kde.native.wait(timeout=20)
    took = time.monotonic() - started
    assert code == 0, f"exit status {code}"
    wait_for("Untethered left the call", lambda: alice not in roster(), 20)
    assert took < 10, f"exited only at the {took:.1f} s deadline: the call didn't end"
    # Element ends a DM call once the other side has left.
    wait_for("Element ended the call", lambda: roster() == [], 20)
    print(f"Quit in the tray menu: left the call and exited with status 0 after {took:.1f} s.", flush=True)
    settings_persist(kde)


def settings_persist(kde):
    """Switch Close to tray off and Start at sign-in on in User Settings; both
    take effect, survive a restart, and the autostart entry starts Untethered
    in the tray."""
    env = kde.env
    def window():
        # Any page: the title names the room, or Account in User Settings.
        return kde.window("^Untethered")
    saved = Path(env["UNTETHERED_PROFILE"]) / "desktop.json"
    entry = Path(env["XDG_CONFIG_HOME"]) / "autostart/io.github.koalexbear.Untethered.desktop"
    assert not saved.exists() and not entry.exists()
    def start(*args):
        binary, child_env, stderr = kde.native_args
        kde.native = kde.stack.enter_context(child([*binary, *args], child_env, stderr=stderr))
    def opened():
        win = wait_for("client window", window, 45)
        kde.position(win, 0, 0, WIDTH, HEIGHT)
        return win
    def words():
        im = kde.capture().crop((0, 0, WIDTH, HEIGHT))
        return [(clean(w), x, y) for w, x, y in read(im, origin=(0, TITLE_BAR))]
    def press(x, y):
        kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 120},
                                {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 80},
                                {"type": "pointerUp", "button": 0}])
    def open_settings():
        # The gear at the right of the user panel, at the foot of the sidebar
        # (KWin sizes the frame: HEIGHT includes the title bar).
        press(RAIL + SIDEBAR - 25, HEIGHT - USER_PANEL // 2)
        wait_for("User Settings", lambda: {"system", "close", "sign-in"} <= {w for w, _, _ in words()}, 15)
    def switch(title):
        """The switch at the right end of the setting row titled `title`:
        its centre, and whether it's on (the theme's blurple track)."""
        first, last = title.lower().split()[0], title.lower().split()[-1]
        ws = words()
        y = next(y for w, x, y in ws if w == last and any(v == first and abs(vy - y) < 6 and vx < x for v, vx, vy in ws))
        im = kde.capture()
        box = [(x, yy) for yy in range(y - 14, y + 40) for x in range(WIDTH // 2, WIDTH - 70)
               if sum(abs(a - b) for a, b in zip(im.getpixel((x, yy)), SIDE)) > 60]
        right = max(x for x, _ in box)
        knob = [(x, yy) for x, yy in box if x > right - 50]
        cx, cy = sum(x for x, _ in knob) // len(knob), sum(yy for _, yy in knob) // len(knob)
        on = sum(sum(abs(a - b) for a, b in zip(im.getpixel(p), (0x58, 0x65, 0xF2))) < 40 for p in knob) > 40
        return (cx, cy), on

    start()
    opened()
    open_settings()
    (close_at, close_on), (_, login_on) = switch("Close to tray"), switch("Start at sign-in")
    assert close_on and not login_on, "defaults: close to tray on, start at sign-in off"
    press(*close_at)
    wait_for("desktop.json written", lambda: saved.exists() and json.loads(saved.read_text()) == {"close_to_tray": False}, 10)
    # Close to tray's description got shorter, so the next row moved up.
    press(*switch("Start at sign-in")[0])
    wait_for("autostart entry written", entry.exists, 10)
    text = entry.read_text()
    exec_line = next(l for l in text.splitlines() if l.startswith("Exec="))
    assert exec_line.endswith(" --start-minimized") and "Hidden=" not in text, text
    assert not switch("Close to tray")[1] and switch("Start at sign-in")[1], "switches show the new state"
    print(f"User Settings: Close to tray off wrote {saved.name} {json.loads(saved.read_text())}; "
          f"Start at sign-in on wrote {entry.name} with {exec_line!r}.", flush=True)

    # Close to tray off: closing quits, now and after a restart.
    for when in ("at once", "after a restart"):
        if when != "at once":
            start()
            opened()
        kde.command("kdotool", "windowclose", window())
        assert kde.native.wait(timeout=15) == 0
        print(f"Close to tray off ({when}): closing the window quit with status 0.", flush=True)

    # The autostart entry's command starts Untethered in the tray, no window.
    start(*exec_line.removeprefix("Exec=").split()[1:])
    tray = Tray(kde)
    time.sleep(4)
    assert not window() and kde.native.poll() is None, "the start at sign-in opened a window"
    tray.activate()
    opened()
    open_settings()
    assert not switch("Close to tray")[1] and switch("Start at sign-in")[1], "settings lost across the restart"
    press(*switch("Start at sign-in")[0])
    wait_for("autostart entry removed", lambda: not entry.exists(), 10)
    print("Autostart command: started in the tray with no window; Activate opened it; both switches kept their state; "
          "switching Start at sign-in off removed the entry.", flush=True)
    tray.click("Quit")
    assert kde.native.wait(timeout=15) == 0


def edge_cases(root):
    """No homeserver needed: the signed-out preview on a private KWin.
    Without a tray host closing quits and a start at sign-in falls back to
    the window; with one, a start at sign-in shows no window until Activate,
    and Activate brings back a minimised window. Launching Untethered again
    shows the running one's window instead of starting a second copy."""
    import tempfile
    from contextlib import ExitStack
    release = root / "rust-client/target/release/untethered-iced"
    if release.is_file():
        os.environ.setdefault("UNTETHERED_QA_CLIENT", str(release))
    def session(stack, tray, *args):
        tmp = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="untethered-tray-")))
        (tmp / "seed").mkdir()
        env = isolated_env(tmp, None, "wayland")
        env["UNTETHERED_KEYRING"] = "off"
        kde = KWin(stack, tmp, root, None, env=env, tray=tray)
        if args:
            # KWin starts the client plainly; start it again as autostart does.
            os.killpg(kde.native.pid, 15); kde.native.wait(timeout=10)
            binary, child_env, stderr = kde.native_args
            kde.native = stack.enter_context(child([*binary, *args], child_env, stderr=stderr))
        return kde
    def window(kde):
        return kde.window("^Untethered")
    def focused(kde):
        win = window(kde)
        return win if win and kde.command("kdotool", "getactivewindow", check=False).stdout.strip() == win else None
    def windows(kde):
        return kde.command("kdotool", "search", "--title", "^Untethered", check=False).stdout.split()
    with ExitStack() as stack:
        kde = session(stack, False)
        win = wait_for("window", lambda: window(kde), 30)
        time.sleep(2)
        kde.command("kdotool", "windowclose", win)
        assert kde.native.wait(timeout=15) == 0
        print("No tray host: closing the window quit with status 0.", flush=True)
    with ExitStack() as stack:
        kde = session(stack, False, "--start-minimized")
        started = time.monotonic()
        time.sleep(3)
        assert not window(kde), "a start at sign-in showed its window at once"
        wait_for("the window, with no tray to start in", lambda: window(kde), 40)
        print(f"No tray host, start at sign-in: the window opened after {time.monotonic() - started:.0f} s.", flush=True)
    with ExitStack() as stack:
        kde = session(stack, True, "--start-minimized")
        tray = Tray(kde)
        time.sleep(3)
        assert not window(kde), "a start at sign-in showed its window"
        tray.activate()
        win = wait_for("window opened and focused by Activate", lambda: focused(kde), 15)
        kde.command("kdotool", "windowminimize", win)
        time.sleep(1)
        tray.activate()
        # Wayland can't restore a minimised window, so it comes back as a new one.
        wait_for("window back in front", lambda: focused(kde) not in (None, win), 10)
        assert len(windows(kde)) == 1
        print("Tray host, start at sign-in: no window until Activate; Activate also brings back a minimised window, focused.",
              flush=True)
        tray.click("Quit")
        assert kde.native.wait(timeout=15) == 0
    with ExitStack() as stack:
        kde = session(stack, True)
        tray = Tray(kde)
        win = wait_for("window", lambda: window(kde), 30)
        binary, child_env, _ = kde.native_args
        def launch(*args):
            """Start Untethered again on the same profile: it must hand over and exit."""
            started = time.monotonic()
            r = sp.run([*binary, *args], env=child_env, capture_output=True, text=True, timeout=30)
            assert r.returncode == 0 and "running already" in r.stderr, (r.returncode, r.stderr[-2000:])
            assert kde.native.poll() is None and len(tray.items()) == 1, "the running Untethered must stay the only one"
            return time.monotonic() - started
        time.sleep(2)
        kde.command("kdotool", "windowclose", win)
        wait_for("window closed to the tray", lambda: not window(kde), 10)
        took = launch()
        win = wait_for("window back and focused after launching again", lambda: focused(kde), 15)
        assert len(windows(kde)) == 1
        launch("--start-minimized")
        time.sleep(2)
        assert windows(kde) == [win], "a start at sign-in must leave the running window alone"
        kde.command("kdotool", "windowminimize", win)
        time.sleep(1)
        launch()
        wait_for("minimised window back in front after launching again", lambda: focused(kde) not in (None, win), 10)
        assert len(windows(kde)) == 1
        print(f"Launching again: the running Untethered showed its window (the launch exited 0 after {took:.1f} s); "
              "a start at sign-in left it alone; one process, one window, one tray item.", flush=True)
        tray.click("Quit")
        assert kde.native.wait(timeout=15) == 0


if __name__ == "__main__":
    edge_cases(Path(__file__).resolve().parent.parent)
