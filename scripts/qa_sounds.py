"""Sounds and shortcuts (ROADMAP N9), live against a disposable Synapse with a
LiveKit call service.

Build the debug client (it seeds the profile) and the release client (it
runs the UI), then from the checkout, with the call fixtures' tools
(LIVEKIT_SERVER, LK_JWT_SERVICE, KWIN_INPUTSYNTH, UNTETHERED_BACKEND):
  $PYTHON scripts/qa_sounds.py [--shots DIR]
It reruns itself in a private network namespace, as run_element.sh does.

The test account has a space, Workshop (alpha, beta, gamma), a room outside
it, a DM with Bob and two encrypted voice rooms, Atrium and Porch. Bob posts
over plain HTTP. The release client runs in a private KWin session with
KDE's tray watcher and a private PulseAudio sink whose monitor the fixture
listens to: every sound has notes of its own (sounds.rs), so each is told
apart by its notes and their order. Driven by the keyboard and the tray's
D-Bus menu, the client must:
- open the next unread channel with Alt+Shift+Down, in sidebar order and on
  through the rail, and stay put with Alt+Shift+Up when nothing is unread;
- show the shortcuts with Ctrl+/ and close them with Escape;
- mark every room of the open space read with Shift+Escape, not others;
- play the message sound for a DM elsewhere, none in the DM being read or
  under Do Not Disturb, and the mention sound for a mention;
- play the join, mute, unmute, deafen, undeafen and leave sounds in a call;
- with all six switches off in User Settings (sounds.json), play none.
UNTETHERED_QA_UI_CLIENT runs another client build.
"""
import argparse
import array
import cmath
import contextlib
import json
import math
import os
from pathlib import Path
import secrets
import shutil
import subprocess as sp
import sys
import tempfile
import threading
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

WIDTH, HEIGHT = 1180, 1000
TITLE_BAR = 28
SHIFT, CONTROL, ALT, ESCAPE, ENTER = "", "", "", "", ""
UP, DOWN = "", ""
MEMBER = "org.matrix.msc3401.call.member"
# The notes of each sound, in order (rust-client/src/sounds.rs).
NOTES = {
    "message": (1568, 2093),
    "mention": (1760, 2349, 2794),
    "join": (1319, 1976),
    "leave": (1865, 1245),
    "mute": (1661, 1397),
    "unmute": (1397, 1661),
    "deafen": (2217, 1480),
    "undeafen": (1480, 2217),
}
FREQS = sorted({f for notes in NOTES.values() for f in notes})
LOUD = 1500  # a note's amplitude while it plays (peaks are 8000)
QUIET = 400
RATE = 12000  # every fourth sample of the 48 kHz monitor
WINDOW = 480  # 40 ms, a new one every 20 ms: 25 Hz bins
HANN = [0.5 - 0.5 * math.cos(math.tau * n / (WINDOW - 1)) for n in range(WINDOW)]
BASIS = {f: [h * cmath.exp(-1j * math.tau * f * n / RATE) for n, h in enumerate(HANN)] for f in FREQS}


def in_namespace():
    links = json.loads(sp.check_output(["ip", "-j", "link", "show"]))
    return {x["ifname"] for x in links} == {"lo", "rtc-test"}


class Speaker:
    """The private sink's monitor, as 40 ms windows of each note's amplitude."""

    def __init__(self, stack, env):
        self.frames = []  # (time, {Hz: amplitude})
        self.lock = threading.Lock()
        monitor = sp.Popen(["pacat", "--record", "--device=QASpeaker.monitor", "--raw", "--rate=48000",
                            "--channels=1", "--format=s16le", "--latency-msec=20"],
                           stdout=sp.PIPE, env=env, stderr=sp.DEVNULL)

        def stop():
            monitor.terminate()
            monitor.wait(timeout=5)
        stack.callback(stop)
        threading.Thread(target=self.listen, args=(monitor,), daemon=True).start()

    def listen(self, monitor):
        recent = array.array("h")
        while data := monitor.stdout.read(1920):  # 20 ms
            pcm = array.array("h")
            pcm.frombytes(data)
            recent.extend(pcm[::4])
            if len(recent) < WINDOW:
                continue
            recent = recent[-WINDOW:]
            amps = {f: 2 * abs(sum(v * b for v, b in zip(recent, BASIS[f]))) / (WINDOW / 2) for f in FREQS}
            with self.lock:
                self.frames.append((time.monotonic(), amps))

    def since(self, at):
        with self.lock:
            return [(t, a) for t, a in self.frames if t >= at]

    def heard(self, sound, at):
        """When `sound` started after `at`: its notes rising in their order."""
        notes = NOTES[sound]
        frames = self.since(at)
        for i, (t, amps) in enumerate(frames):
            if amps[notes[0]] < LOUD or any(amps[n] >= LOUD for n in notes[1:]):
                continue
            j = i
            for n in notes[1:]:
                j = next((k for k in range(j + 1, min(len(frames), j + 16)) if frames[k][1][n] >= LOUD), None)
                if j is None:
                    break
            else:
                return t
        return None

    def silent(self, at):
        """The loudest note since `at`."""
        return max((max(a.values()) for _, a in self.since(at)), default=0)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    parsed = args.parse_args()
    if not in_namespace():
        setup = ("ip link set lo up; ip link add rtc-test type dummy; ip addr add 192.0.2.2/24 dev rtc-test; "
                 "ip link set rtc-test up; exec \"$@\"")
        os.execvp("unshare", ["unshare", "-Urnm", "sh", "-ec", setup, "qa", sys.executable, *sys.argv])
    for tool in ("LIVEKIT_SERVER", "LK_JWT_SERVICE"):
        assert os.environ.get(tool), f"Set {tool} (see the call fixtures)"
    from qa_media_previews import interop
    from qa_rtc_audio import call_services, configure
    mod = interop()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-sounds-"))
    try:
        name, listen, settings, jwt = configure(tmp, mod.free_port)
        sp.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config", "-H", name,
                "--report-stats", "no"], cwd=tmp, stdout=sp.DEVNULL, check=True)
        port = mod.free_port()
        hs = f"http://127.0.0.1:{port}"
        limit = "{per_second: 1000, burst_count: 1000}"
        (tmp / "local.yaml").write_text(f"""listeners:
{listen}  - port: {port}
    type: http
    tls: false
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [client, media]}}]
public_baseurl: {hs}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: []
max_event_delay_duration: 1h
rc_message: {limit}
rc_room_creation: {limit}
rc_invites: {{per_room: {limit}, per_user: {limit}, per_issuer: {limit}}}
rc_joins: {{local: {limit}, remote: {limit}}}
rc_login: {{address: {limit}, account: {limit}, failed_attempts: {limit}}}
{settings}""")
        with mod.service([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "-c", "local.yaml"],
                         tmp) as synapse:
            for _ in range(300):
                assert synapse.poll() is None, "Disposable homeserver exited"
                try:
                    mod.http("GET", hs + "/_matrix/client/versions")
                    break
                except Exception:
                    time.sleep(.1)
            world = populate(tmp, hs, name, mod)
            with call_services(tmp, name, jwt, mod.service):
                look(tmp, parsed.shots, world)
        print("PASS: sounds play for messages, mentions, joining, leaving, mute and deafen, and stop when switched "
              "off; Alt+Shift+Down, Ctrl+/ and Shift+Escape work", flush=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def populate(tmp, hs, name, mod):
    """The test account (seeded), Bob, and their rooms."""
    from qa_matrix_sdk import register
    http = mod.http
    fixture, hdr = mod.seed(tmp, hs, encrypted=False)
    me = f"@{fixture['username']}:{name}"
    bob, _, bob_hdr = register(hs, "bob" + secrets.token_hex(3))
    http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(bob, safe='')}/displayname", headers=bob_hdr,
         json={"displayname": "Bob"})

    def path(room):
        return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

    def create(**body):
        return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

    def with_bob(room_name, **body):
        room = create(name=room_name, preset="private_chat", invite=[bob], **body)
        http("POST", path(room) + "/join", headers=bob_hdr, json={})
        return room
    space = create(name="Workshop", preset="private_chat", creation_content={"type": "m.space"})
    rooms = {n: with_bob(n) for n in ("alpha", "beta", "gamma", "outside")}
    for n in ("alpha", "beta", "gamma"):
        http("PUT", f"{path(space)}/state/m.space.child/{quote(rooms[n], safe='')}", headers=hdr,
             json={"via": [name]})
    dm = create(preset="trusted_private_chat", is_direct=True, invite=[bob])
    http("POST", path(dm) + "/join", headers=bob_hdr, json={})
    http("PUT", f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}/account_data/m.direct", headers=hdr,
         json={bob: [dm]})
    voice = {}
    for n in ("Atrium", "Porch"):
        voice[n] = create(name=n, preset="private_chat", creation_content={"type": "org.matrix.msc3417.call"},
                          power_level_content_override={"events": {MEMBER: 0, "org.matrix.msc4143.rtc.slot": 0}},
                          initial_state=[{"type": "m.room.encryption", "state_key": "",
                                          "content": {"algorithm": "m.megolm.v1.aes-sha2"}}])

    def post(room, body, mention=False):
        content = {"msgtype": "m.text", "body": body}
        if mention:
            content["m.mentions"] = {"user_ids": [me]}
            content["body"] = f"{fixture['username']}: {body}"
        http("PUT", f"{path(room)}/send/m.room.message/{secrets.token_hex(8)}", headers=bob_hdr, json=content)

    def counts():
        """Each room's unread notifications on the server, as the account syncs them."""
        flt = json.dumps({"room": {"timeline": {"limit": 1}, "state": {"lazy_load_members": True}}})
        joined = http("GET", f"{hs}/_matrix/client/v3/sync?filter={quote(flt)}", headers=hdr).json()["rooms"]["join"]
        return {r: v.get("unread_notifications", {}).get("notification_count", 0) for r, v in joined.items()}
    def in_call(room_name):
        """The account's MatrixRTC membership in a voice room is live."""
        state = http("GET", path(voice[room_name]) + "/state", headers=hdr).json()
        return any(e["type"] == MEMBER and e["sender"] == me and e["content"] for e in state)
    for n in ("beta", "gamma", "outside"):
        post(rooms[n], f"news in {n}")
    print(f"Synapse at {hs}: {me} with Bob, a space with alpha/beta/gamma, outside, a DM and two voice rooms",
          flush=True)
    return dict(me=me, rooms=rooms, dm=dm, voice=voice, post=post, counts=counts, in_call=in_call,
                profile=tmp / "seed")


def look(tmp, shots, world):
    from qa_kwin import KWin
    from qa_native_paste import child, isolated_env, wait_for
    from qa_tray import Tray
    from qa_ui import RAIL, SIDEBAR, SIDE, USER_PANEL, clean, read
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    rooms, counts, post = world["rooms"], world["counts"], world["post"]
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        sock = Path(env["XDG_RUNTIME_DIR"]) / "qa-pulse"
        env.update(UNTETHERED_PROFILE=str(world["profile"]), UNTETHERED_KEYRING="off",
                   PULSE_SERVER="unix:" + str(sock), PULSE_SOURCE="QAMic.monitor", PULSE_SINK="QASpeaker")
        pulse = stack.enter_context(child(["pulseaudio", "-n", "--daemonize=no", "--exit-idle-time=-1",
            "--use-pid-file=no", "--load=module-native-protocol-unix socket=" + str(sock) + " auth-anonymous=1",
            *(f"--load=module-null-sink sink_name={n} sink_properties=device.description={n} rate=48000 channels=1"
              for n in ("QAMic", "QASpeaker"))], env))

        def audio_ready():
            assert pulse.poll() is None, "private audio server exited"
            return sock.is_socket() and "QASpeaker" in sp.run(["pactl", "list", "short", "sinks"], env=env,
                                                               capture_output=True, text=True).stdout
        wait_for("private PulseAudio sinks", audio_ready)
        speaker = Speaker(stack, env)
        kde = KWin(stack, ui, ROOT, None, env=env, tray=True)
        tray = Tray(kde)
        win = wait_for("client window", lambda: kde.window("^Untethered"), 60)
        kde.position(win, 0, 0, WIDTH, HEIGHT)

        def title():
            return kde.command("kdotool", "getwindowname", win).stdout.split("·")[-1].strip()

        def shot(name):
            im = kde.capture().crop((0, 0, WIDTH, HEIGHT))
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words():
            return [(clean(w), x, y) for w, x, y in read(kde.capture().crop((0, 0, WIDTH, HEIGHT)),
                                                          origin=(0, TITLE_BAR))]

        def chord(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def go(query, expect):
            """Ctrl+K, a few letters and Enter, until `expect` is open."""
            def opened():
                if title() == expect:
                    return True
                chord(CONTROL, "k")
                time.sleep(.6)
                kde.type(query)
                time.sleep(.8)
                chord(ENTER)
                time.sleep(1.5)
                if title() != expect:
                    chord(ESCAPE)  # the switcher, if the room wasn't listed yet
                    return False
                return True
            wait_for(f"{expect} opened with Ctrl+K", opened, 90)

        def read_on_server(*names):
            wait_for(f"{', '.join(names)} read on the server",
                     lambda: all(counts().get(rooms[n], 0) == 0 for n in names), 30)

        def expect(sound, at, within=8):
            t = wait_for(f"the {sound} sound", lambda: speaker.heard(sound, at), within)
            return t - at

        def quiet(what, at, hold=3.5):
            time.sleep(max(0, at + hold - time.monotonic()))
            loudest = speaker.silent(at)
            assert loudest < QUIET, f"{what}: a sound played (note amplitude {loudest:.0f})"

        # --- Shortcuts ---------------------------------------------------
        go("alpha", "alpha")
        read_on_server("alpha")
        for want in ("beta", "gamma", "outside"):
            chord(ALT, SHIFT, DOWN)
            wait_for(f"Alt+Shift+Down opening {want}", lambda: title() == want, 10)
            read_on_server(want)
        print("Alt+Shift+Down: alpha -> beta -> gamma (down the space) -> outside (around the rail, past the "
              "read DM)", flush=True)
        chord(ALT, SHIFT, UP)
        time.sleep(2)
        assert title() == "outside", f"nothing unread, yet Alt+Shift+Up opened {title()}"
        print("Alt+Shift+Up with nothing unread stayed in outside", flush=True)

        chord(CONTROL, "/")
        try:
            wait_for("Ctrl+/'s shortcuts", lambda: {"keyboard", "shortcuts", "unread"} <= {w for w, _, _ in words()}, 10)
        finally:
            shot("shortcuts")
        chord(ESCAPE)
        wait_for("Escape closing the shortcuts", lambda: "keyboard" not in {w for w, _, _ in words()}, 10)
        assert title() == "outside"
        print("Ctrl+/ listed the shortcuts; Escape closed the list", flush=True)

        at = time.monotonic()
        for n in ("alpha", "beta"):
            post(rooms[n], f"more for {n}")
        wait_for("Bob's new messages unread", lambda: all(counts().get(rooms[n], 0) > 0 for n in ("alpha", "beta")), 15)
        quiet("ordinary group messages", at)
        go("*work", "gamma")  # the space opens on the room last open there
        post(rooms["outside"], "more for outside")
        wait_for("outside unread again", lambda: counts().get(rooms["outside"], 0) > 0, 15)
        chord(SHIFT, ESCAPE)
        read_on_server("alpha", "beta")
        time.sleep(2)
        left = counts().get(rooms["outside"], 0)
        assert left > 0, "Shift+Escape marked a room outside the space read"
        print(f"Shift+Escape in Workshop marked alpha and beta read on the server; outside kept {left} unread; "
              "ordinary group messages played no sound", flush=True)

        # --- Message sounds ----------------------------------------------
        at = time.monotonic()
        post(world["dm"], "hi from Bob")
        print(f"A DM while reading gamma: the message sound, after {expect('message', at):.1f} s", flush=True)
        time.sleep(2)
        at = time.monotonic()
        post(rooms["alpha"], "ping", mention=True)
        print(f"A mention in alpha: the mention sound, after {expect('mention', at):.1f} s", flush=True)
        go("bob", "Bob")
        time.sleep(2)
        at = time.monotonic()
        post(world["dm"], "you're reading this one")
        quiet("a DM in the open, focused DM", at)
        print("A DM in the DM being read: no sound", flush=True)
        go("gamma", "gamma")
        tray.click("Do Not Disturb")
        wait_for("Do Not Disturb on", lambda: "Do Not Disturb" in tray.summary(), 15)
        at = time.monotonic()
        post(world["dm"], "while you're away")
        quiet("a DM under Do Not Disturb", at)
        tray.click("Do Not Disturb")
        wait_for("Do Not Disturb off", lambda: "Do Not Disturb" not in tray.summary(), 15)
        print("A DM under Do Not Disturb: no sound", flush=True)

        # --- Call sounds -------------------------------------------------
        at = time.monotonic()
        go("atrium", "Atrium")  # opening a voice channel joins it
        joined = expect("join", at, 60)
        print(f"Joined Atrium: the join sound, {joined:.1f} s after Enter", flush=True)
        for entry, sound in (("Mute", "mute"), ("Mute", "unmute"), ("Deafen", "deafen"), ("Deafen", "undeafen")):
            time.sleep(1)
            at = time.monotonic()
            tray.click(entry)
            print(f"Tray {entry}: the {sound} sound, after {expect(sound, at):.1f} s", flush=True)
        at = time.monotonic()
        go("porch", "Porch")  # leaves Atrium, then joins Porch
        left_at = expect("leave", at, 30)
        print(f"Porch: the leave sound for Atrium after {left_at:.1f} s, then the join sound after "
              f"{expect('join', at + left_at, 60) + left_at:.1f} s", flush=True)

        # --- Switched off ------------------------------------------------
        saved = world["profile"] / "sounds.json"

        def press(x, y):
            kde.actions("pointer", [{"type": "pointerMove", "x": x, "y": y}, {"type": "pause", "duration": 120},
                                    {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 80},
                                    {"type": "pointerUp", "button": 0}])
        press(RAIL + SIDEBAR - 25, HEIGHT - USER_PANEL // 2)
        wait_for("User Settings", lambda: "privacy" in {w for w, _, _ in words()}, 15)

        def scroll(dy):
            kde.actions("wheel", [{"type": "scroll", "x": 700, "y": 500, "deltaX": 0, "deltaY": dy}])
            kde.actions("pointer", [{"type": "pointerMove", "x": 702, "y": 502}])

        def sounds_shown():
            ws = words()
            found = {w for w, _, _ in ws}
            if "undeafen" in found and any(w == "undeafen" and y < HEIGHT - 80 for w, _, y in ws):
                return ws
            scroll(240)
            return None
        wait_for("the SOUNDS switches", sounds_shown, 40)
        titles = ("Message", "Mention", "Voice join", "Voice leave", "Mute and unmute", "Deafen and undeafen")

        def switch(label, ws):
            """The switch at the right end of the row titled `label`, and whether it's on. A title's
            first word starts its line; its description's line starts otherwise ("A message …")."""
            first, last = label.lower().split()[0], label.lower().split()[-1]

            def line(y):
                return [(v, vx) for v, vx, vy in ws if abs(vy - y) < 6 and vx > RAIL + SIDEBAR]
            y = next(y for w, x, y in sorted(ws, key=lambda w: w[2]) if w == first and x > RAIL + SIDEBAR
                     and min(line(y), key=lambda v: v[1])[0] == first
                     and (first == last or last in [v for v, _ in line(y)]))
            im = kde.capture()
            box = [(x, yy) for yy in range(y - 14, y + 40) for x in range(WIDTH // 2, WIDTH - 70)
                   if sum(abs(a - b) for a, b in zip(im.getpixel((x, yy)), SIDE)) > 60]
            right = max(x for x, _ in box)
            knob = [(x, yy) for x, yy in box if x > right - 50]
            cx, cy = sum(x for x, _ in knob) // len(knob), sum(yy for _, yy in knob) // len(knob)
            on = sum(sum(abs(a - b) for a, b in zip(im.getpixel(p), (0x58, 0x65, 0xF2))) < 40 for p in knob) > 40
            return (cx, cy), on
        keys = {"Message": "messages", "Mention": "mentions", "Voice join": "join", "Voice leave": "leave",
                "Mute and unmute": "mute", "Deafen and undeafen": "deafen"}
        off = set()
        for label in titles:
            (x, y), on = switch(label, words())
            assert on, f"{label} isn't on by default"
            press(x, y)
            off.add(keys[label])
            wait_for(f"{label} off in sounds.json", lambda: saved.exists() and {
                k for k, v in json.loads(saved.read_text()).items() if not v} == off, 10)
        shot("sounds-off")
        assert not any(switch(label, words())[1] for label in titles), "a switch still shows on"
        print(f"User Settings: switched all six sounds off; {saved.name} = {json.loads(saved.read_text())}",
              flush=True)
        chord(ESCAPE)
        wait_for("back from User Settings", lambda: "privacy" not in {w for w, _, _ in words()}, 10)

        go("gamma", "gamma")
        at = time.monotonic()
        post(world["dm"], "can you hear this")
        quiet("a DM with the message sound off", at)
        time.sleep(2)
        at = time.monotonic()
        post(rooms["alpha"], "and this?", mention=True)
        quiet("a mention with the mention sound off", at)
        for entry in ("Mute", "Mute", "Deafen", "Deafen"):
            at = time.monotonic()
            tray.click(entry)
            quiet(f"tray {entry} with its sound off", at, 2.5)
        at = time.monotonic()
        go("atrium", "Atrium")
        quiet("leaving Porch and joining Atrium with their sounds off", at, max(15, 2 * joined))
        assert world["in_call"]("Atrium") and not world["in_call"]("Porch"), "the silent move to Atrium didn't happen"
        print("Switched off: no sound for a DM, a mention, mute, unmute, deafen, undeafen, leaving Porch or "
              "joining Atrium", flush=True)
        shot("done")


if __name__ == "__main__":
    main()
