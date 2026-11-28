"""Voice messages (ROADMAP M10), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_voice_messages.py

The test account shares two rooms with Enzo, who is on a gomuks device:
studio (encrypted) and lounge (not), and is a member without the right to
post in Enzo's announcements. The release client runs in a private KWin
session (as qa_kwin.py needs) with its own PulseAudio server, whose only
microphone is a null sink's monitor playing a 440 Hz tone; nothing reaches
the real speakers or records the real microphone. The client must:
- offer the microphone in the empty composer, hide it while text is typed,
  and hide it in announcements, where the account can't post;
- show a timer and live levels while recording; Escape throws a recording
  away and sends nothing;
- send a recording on Enter in studio: Enzo's device decrypts an `m.audio`
  event in Element's shape (MSC3245's voice marker, MSC1767's audio block
  with the duration and 100 waveform bars from 0 to 1024, the text and file
  fallbacks), whose file, decrypted with the event's keys, is Ogg Opus of
  that length with the tone in it;
- send one with the Send button in lounge, read back from Synapse's API and
  its media repository, and drawn in the timeline with its waveform and
  length;
- reach Element Web (ELEMENT_WEB_DIR, signed in as Enzo) as a voice
  message, with its waveform and length, that plays. Without
  ELEMENT_WEB_DIR that part is skipped.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import json
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import struct
import subprocess as sp
import sys
import tempfile
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import Gomuks, http, register  # noqa: E402
from qa_media_previews import SERVER_NAME, homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean, tesseract  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
ENTER, ESCAPE, CTRL, BACKSPACE = "", "", "", ""
TONE = 440
VOICE = "org.matrix.msc3245.voice"
AUDIO = "org.matrix.msc1767.audio"


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-voice-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, room=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))

        def create(h, **body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=h, json=body).json()["room_id"]

        def join(h, room):
            http("POST", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/join", headers=h, json={})

        encryption = [{"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]
        studio = create(hdr, name="studio", preset="private_chat", invite=[enzo], initial_state=encryption)
        lounge = create(hdr, name="lounge", preset="private_chat", invite=[enzo])
        news = create(enzo_hdr, name="announcements", preset="private_chat", invite=[me],
                      power_level_content_override={"events_default": 50})
        for room in (studio, lounge):
            join(enzo_hdr, room)
        join(hdr, news)

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        key = peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        peer.joined(studio)
        print(f"Synapse at {hs}: {me} with {enzo} on gomuks in studio (encrypted) and lounge; "
              "announcements is read-only for the test account", flush=True)
        world = dict(tmp=tmp, hs=hs, hdr=hdr, me=me, enzo=enzo, enzo_pwd=enzo_pwd, key=key, peer=peer,
                     studio=studio, lounge=lounge)
        look(args.shots, world)
        element(world)
        print("PASS: voice messages recorded from a known tone reach Matrix as Element sends them, encrypted "
              "or not, with the right length, waveform and sound; Escape sends nothing; the microphone hides "
              "where it doesn't apply", flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def goertzel(samples, freq, rate=48000):
    """The power of `freq` in `samples`."""
    w = 2 * math.pi * freq / rate
    c = 2 * math.cos(w)
    s1 = s2 = 0.0
    for x in samples:
        s1, s2 = x + c * s1 - s2, s1
    return s1 * s1 + s2 * s2 - c * s1 * s2


def check_content(content, *, encrypted, shown):
    """An event's content against Element Web's voice message, recorded for
    about `shown` seconds (what the timer read when it was sent)."""
    assert content["msgtype"] == "m.audio", content
    assert content["body"] == "Voice message" and content["org.matrix.msc1767.text"] == "Voice message", content
    assert content[VOICE] == {}, content
    info, audio, file = content["info"], content[AUDIO], content["org.matrix.msc1767.file"]
    assert info["mimetype"] == "audio/ogg" == file["mimetype"], content
    assert info["size"] == file["size"] > 0 and file["name"] == "Voice message.ogg", content
    assert audio["duration"] == info["duration"], content
    # The timer shows whole seconds; the file ends when Send is pressed.
    assert shown - 0.2 <= info["duration"] / 1000 <= shown + 1.6, (shown, info)
    waveform = audio["waveform"]
    assert len(waveform) == 100 and all(isinstance(v, int) and 0 <= v <= 1024 for v in waveform), waveform
    # A steady tone: nearly every bar near the top.
    assert sum(v >= 700 for v in waveform) >= 80 and max(waveform) == 1024, waveform
    if encrypted:
        assert "url" not in content and content["file"]["url"].startswith("mxc://"), content
        assert file["file"] == content["file"] and "url" not in file, content
    else:
        assert content["url"].startswith("mxc://") and "file" not in content and file["url"] == content["url"]
    return info


def check_file(path, data, info):
    """The downloaded (and decrypted) file: Ogg Opus of the event's length,
    with the tone audible in it."""
    path.write_bytes(data)
    assert len(data) == info["size"], (len(data), info)
    probe = json.loads(sp.run(["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", str(path)],
                              capture_output=True, text=True, check=True).stdout)
    stream, fmt = probe["streams"][0], probe["format"]
    assert stream["codec_name"] == "opus" and fmt["format_name"] == "ogg", probe
    assert int(stream["channels"]) == 1, stream
    length = float(fmt["duration"])
    assert abs(length * 1000 - info["duration"]) < 300, (length, info)
    pcm = sp.run(["ffmpeg", "-v", "error", "-i", str(path), "-f", "s16le", "-ac", "1", "-ar", "48000", "-"],
                 capture_output=True, check=True).stdout
    samples = struct.unpack(f"<{len(pcm) // 2}h", pcm)
    middle = [s / 32768 for s in samples[len(samples) // 2 - 12000:len(samples) // 2 + 12000]]
    rms = math.sqrt(sum(s * s for s in middle) / len(middle))
    tone, off = goertzel(middle, TONE), max(goertzel(middle, f) for f in (220, 1000, 2500))
    assert rms > 0.1, f"the recording is nearly silent (RMS {rms:.3f})"
    assert tone > 100 * off, f"no {TONE} Hz tone in the recording ({tone:.0f} against {off:.0f})"
    print(f"File: Ogg Opus, mono, {length:.2f} s (event says {info['duration']} ms), "
          f"{TONE} Hz tone at RMS {rms:.2f}, {10 * math.log10(tone / max(off, 1e-9)):.0f} dB above other notes",
          flush=True)


def look(shots, world):
    from qa_kwin import KWin
    from qa_layout import Layout
    from qa_native_paste import child, isolated_env, wait_for
    tmp, hs, hdr, me, peer = world["tmp"], world["hs"], world["hdr"], world["me"], world["peer"]
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        sock = Path(env["XDG_RUNTIME_DIR"]) / "qa-pulse"
        map_file = ui / "controls.json"
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off",
                   UNTETHERED_QA_LAYOUT=str(map_file),
                   PULSE_SERVER="unix:" + str(sock), PULSE_SOURCE="QAMic.monitor", PULSE_SINK="QASpeaker")
        pulse = stack.enter_context(child(["pulseaudio", "-n", "--daemonize=no", "--exit-idle-time=-1",
            "--use-pid-file=no", "--load=module-native-protocol-unix socket=" + str(sock) + " auth-anonymous=1",
            *(f"--load=module-null-sink sink_name={n} sink_properties=device.description={n} rate=48000 channels=1"
              for n in ("QAMic", "QASpeaker"))], env))

        def pactl(*a):
            return sp.run(["pactl", *a], env=env, capture_output=True, text=True, timeout=10)

        def audio_ready():
            assert pulse.poll() is None, "private audio server exited"
            return sock.is_socket() and "QAMic" in pactl("list", "short", "sinks").stdout
        wait_for("private PulseAudio sinks", audio_ready)
        pactl("set-default-source", "QAMic.monitor")
        pactl("set-default-sink", "QASpeaker")
        # The microphone: a steady tone into QAMic, heard on its monitor.
        stack.enter_context(child(["gst-launch-1.0", "-q", "audiotestsrc", "is-live=true", "wave=sine",
                                   f"freq={TONE}", "volume=0.5", "!", "audioconvert", "!",
                                   "audio/x-raw,rate=48000,channels=1", "!", "pulsesink", "device=QAMic"], env))

        def tone_playing():
            rec = sp.Popen(["parec", "--raw", "--format=s16le", "--rate=48000", "--channels=1", "-d",
                            "QAMic.monitor"], env=env, stdout=sp.PIPE)
            time.sleep(0.5)
            rec.terminate()
            data = rec.communicate()[0]
            if len(data) < 4800:
                return False
            tail = data[-4800:]
            peak = max(abs(v) for v in struct.unpack(f"<{len(tail) // 2}h", tail))
            return peak > 8000
        wait_for("the test tone on the private microphone", tone_playing, 30)
        print(f"Audio: a private PulseAudio whose microphone plays a {TONE} Hz tone", flush=True)

        kde = KWin(stack, ui, ROOT, None, env=env)
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)
        controls = Layout(map_file, origin=(0, TITLE_BAR))

        def grab():
            return kde.capture().crop(WINDOW).convert("RGB")

        def shot(name, im=None):
            im = im or grab()
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=MAIN):
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def word(part, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if part in w), None)

        def press(*keys):
            kde.keys(*[("Down", k) for k in keys], *[("Up", k) for k in reversed(keys)])

        def seconds(timer):
            m = re.fullmatch(r"(\d+):(\d\d)", timer.strip())
            return int(m[1]) * 60 + int(m[2]) if m else -1

        def gone(name):
            return controls.get(name) is None

        def open_room(name):
            tap(kde, *wait_for(f"{name} in the sidebar", lambda: word(name, SIDEBAR), 90))
            wait_for(f"{name} open", lambda: word(name, (330, 28, 1180, 80)), 20)

        def live_bar_height(im):
            """The tallest bright bar in the recording bar's levels."""
            c = controls.get("voice-wave")
            left, top, right, bottom = controls.box(c)
            px, best = im.load(), 0
            for x in range(int(left), int(right)):
                run = sum(1 for y in range(int(top), int(bottom))
                          if all(abs(a - b) < 30 for a, b in zip(px[x, y], (0xdb, 0xde, 0xe1))))
                best = max(best, run)
            return best

        def record(seconds_wanted, label):
            """Start recording from the microphone button; once the timer
            reads `seconds_wanted`, what it reads."""
            tap(kde, *controls.wait("voice-record", "on", 20))
            controls.wait("voice-timer", "on", 30)
            wait_for(f"{seconds_wanted} s on the timer", lambda: seconds(controls.text("voice-timer"))
                     >= seconds_wanted, seconds_wanted + 15)
            time.sleep(0.3)
            im = shot(label)
            height = live_bar_height(im)
            assert height >= 14, f"the live levels stay flat ({height} px) while the tone plays"
            return seconds(controls.text("voice-timer")), height

        toolbar = Ui(lambda: words(WINDOW), lambda pos: None, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
        composer = toolbar.composer()

        open_room("studio")
        try:
            controls.wait("voice-record", "on", 30)
        except Exception:
            shot("no-microphone")
            print(controls.missing(), flush=True)
            raise
        shot("composer-empty")
        tap(kde, *composer)
        kde.type("hello")
        wait_for("the microphone hidden while typing", lambda: gone("voice-record"), 10)
        assert controls.find("voice-record") is None
        press(CTRL, "a")
        press(BACKSPACE)
        controls.wait("voice-record", "on", 10)
        print("UI: the empty composer offers the microphone in Send's place; typing hides it", flush=True)

        # Escape throws a recording away.
        record(1, "recording-to-cancel")
        press(ESCAPE)
        wait_for("the recording bar gone", lambda: gone("voice-timer") and controls.find("voice-record", "on"), 10)
        print("UI: recording showed a timer and live levels; Escape threw it away", flush=True)

        # Enter sends one.
        shown, height = record(4, "recording-studio")
        press(ENTER)
        try:
            wait_for("the recording bar gone after sending", lambda: gone("voice-timer"), 90)
        finally:
            shot("sent-studio")

        def voice_events(room):
            return [e for e in peer.events if e.get("room_id") == room and e.get("sender") == me
                    and VOICE in (e.get("decrypted") or e.get("content") or {})]
        event = peer.find("the voice message in studio", lambda e: e.get("room_id") == world["studio"]
                          and e.get("sender") == me and VOICE in (e.get("decrypted") or {}), 90)
        assert event["type"] == "m.room.encrypted", event["type"]
        content = event["decrypted"]
        info = check_content(content, encrypted=True, shown=shown)
        print(f"Matrix: Enzo's device decrypted Element's voice message: {info['duration']} ms (timer read "
              f"{shown} s), {info['size']} bytes, 100 bars, MSC3245 marker, text and file fallbacks", flush=True)
        check_file(tmp / "studio.ogg", peer.media(content["file"]["url"], True), info)
        time.sleep(2)
        assert len({e["event_id"] for e in voice_events(world["studio"])}) == 1, "the cancelled recording was sent"

        # Send, the button, in a room without encryption.
        open_room("lounge")
        shown, _ = record(3, "recording-lounge")
        tap(kde, *controls.wait("voice-send", "on", 5))
        wait_for("the recording bar gone after Send", lambda: gone("voice-timer"), 90)

        def latest():
            chunk = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(world['lounge'], safe='')}/messages"
                         "?dir=b&limit=20", headers=hdr).json()["chunk"]
            return next((e for e in chunk if e["sender"] == me and e["type"] == "m.room.message"
                         and VOICE in e["content"]), None)
        event = wait_for("the voice message on Synapse", latest, 30)
        info = check_content(event["content"], encrypted=False, shown=shown)
        server, media = event["content"]["url"].removeprefix("mxc://").split("/", 1)
        data = http("GET", f"{hs}/_matrix/client/v1/media/download/{quote(server, safe='')}/{quote(media, safe='')}",
                    headers=hdr).content
        print(f"Synapse: lounge has the voice message, {info['duration']} ms (timer read {shown} s)", flush=True)
        check_file(tmp / "lounge.ogg", data, info)
        world["lounge_event"] = event

        # Our own voice message in the timeline: its length before playing,
        # and its waveform.
        length = f"0:{info['duration'] // 1000:02d}"

        def card():
            """The card's title and the time on its play bar, read at four
            times its size (it's small)."""
            title = next(((x, y) for w, x, y in words() if w == "voice"), None)
            if not title:
                return None
            # Just the time: the play button and the bars beside it confuse OCR.
            clock = (title[0] - 18, title[1] + 40, title[0] + 46, title[1] + 64)
            text = " ".join(w for w, _, _ in tesseract(grab().crop(clock), 7, scale=4))
            return (title, (clock[2] + 4, clock[1] - 2, title[0] + 380, clock[3] + 2)) \
                if "0:00" in text and length in text else None
        try:
            title, row = wait_for(f"the card's 0:00 / {length}", card, 30)
        finally:
            im = shot("voice-card")
        px = im.load()
        bars = sum(1 for x in range(row[0], row[2]) for y in range(row[1], row[3])
                   if all(abs(a - b) < 14 for a, b in zip(px[x, y], (0x6d, 0x6f, 0x78))))
        assert bars > 300, f"no waveform beside the card's time ({bars} px of bars)"
        print(f"UI: the sent voice message shows 0:00 / {length} and its waveform ({bars} px of bars)", flush=True)

        # No microphone where the account can't post.
        open_room("announcements")
        wait_for("the read-only composer", lambda: word("read-only"), 20)
        time.sleep(1)
        assert gone("voice-record"), "the microphone shows in a room the account can't post in"
        shot("read-only")
        print("UI: announcements, read-only for the account, has no microphone", flush=True)


def element(world):
    """Element Web, signed in as Enzo, shows lounge's voice message as one."""
    if not os.environ.get("ELEMENT_WEB_DIR"):
        print("Element: skipped (set ELEMENT_WEB_DIR to an extracted Element Web release)", flush=True)
        return
    from playwright.sync_api import sync_playwright
    from qa_kwin_element import element_server, login_element
    event = world["lounge_event"]["event_id"]
    peer = dict(username=world["enzo"].split(":")[0][1:], password=world["enzo_pwd"], key=world["key"])
    with element_server(world["hs"], server_name=SERVER_NAME) as web, sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                     args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist",
                                           "--autoplay-policy=no-user-gesture-required"])
        try:
            ctx = browser.new_context(viewport={"width": 1300, "height": 900}, locale="en-US")
            ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"), lambda r: r.abort())
            page = login_element(ctx, web, peer, world["lounge"])
            tile = page.locator(f'[data-event-id="{event}"]')
            body = tile.locator(".mx_MVoiceMessageBody")
            body.wait_for(timeout=30000)
            bars = body.locator(".mx_Waveform_bar")
            page.wait_for_function("(n) => document.querySelectorAll(n).length > 0",
                                   arg=f'[data-event-id="{event}"] .mx_Waveform_bar', timeout=20000)
            clock = body.locator(".mx_Clock").first
            before = clock.inner_text().strip()
            # Element rounds the length to whole seconds.
            ms = world["lounge_event"]["content"]["info"]["duration"]
            assert before.endswith(f":{ms // 1000:02d}") or before.endswith(f":{-(-ms // 1000):02d}"), (before, ms)
            assert "error" not in body.inner_text().lower(), body.inner_text()
            body.locator(".mx_PlayPauseButton").first.click()
            page.wait_for_function("([sel, was]) => { const c = document.querySelector(sel);"
                                   " return c && c.textContent.trim() !== was; }",
                                   arg=[f'[data-event-id="{event}"] .mx_MVoiceMessageBody .mx_Clock', before],
                                   timeout=15000)
            playing = clock.inner_text().strip()
            print(f"Element: a voice message with {bars.count()} waveform bars, {before} long; it plays "
                  f"(clock {before} → {playing})", flush=True)
        finally:
            browser.close()


if __name__ == "__main__":
    main()
