"""Animated images, inline players and sending video (ROADMAP P1, P2, P5),
live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_phase4.py

Mira's profile picture is an animated GIF (green and yellow); the test
account's room has an animated GIF (red and blue), Mira's message, chatter,
a video with a magenta poster and an eight-second tone. The release client
in a private KWin session (as qa_kwin.py needs), with its own PulseAudio
null sink, must:
- P1: barely use the CPU while the GIF is scrolled away, then play it once
  it's on screen (both colours show) and use the CPU for it; play Mira's
  profile picture beside her message;
- P2: play the video in place on a click, in its poster's space (the
  viewer stays shut), with its controls over it: shown when the pointer
  moves over it, hidden a moment after it stops; and play the tone in its
  card, whose time moves on and jumps when the seek bar is clicked;
- P5: send a video dropped from Dolphin with its width, height, duration
  and a poster thumbnail (the frame Element shows), which decodes.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import io
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
FEED = (330, 80, 1180, 820)
RED, BLUE, GREEN, YELLOW, MAGENTA = (220, 30, 30), (30, 60, 220), (40, 200, 60), (240, 220, 30), (225, 30, 190)
PAGE_UP, PAGE_DOWN, ENTER = "\ue00e", "\ue00f", "\ue007"


def near(pixel, colour, slack=50):
    return all(abs(a - b) < slack for a, b in zip(pixel[:3], colour))


def gif(colours, size, delay=300):
    frames = [Image.new("RGB", size, c) for c in colours]
    buf = io.BytesIO()
    frames[0].save(buf, "GIF", save_all=True, append_images=frames[1:], duration=delay, loop=0)
    return buf.getvalue()


def gst(*pipeline):
    subprocess.run(["gst-launch-1.0", "-q", *pipeline], check=True, capture_output=True, timeout=60)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-phase4-"))
    synapse = None
    try:
        files = tmp / "DropFiles"
        files.mkdir()
        clip = files / "clip.mp4"
        gst("videotestsrc", "num-buffers=60", "pattern=ball", "!", "video/x-raw,width=320,height=240,framerate=30/1",
            "!", "videoconvert", "!", "openh264enc", "!", "h264parse", "!", "mp4mux", "!", "filesink",
            f"location={clip}")
        # Long enough to watch its controls hide while it plays.
        long = tmp / "long.mp4"
        gst("videotestsrc", "num-buffers=600", "pattern=ball", "!", "video/x-raw,width=320,height=240,framerate=30/1",
            "!", "videoconvert", "!", "openh264enc", "!", "h264parse", "!", "mp4mux", "!", "filesink",
            f"location={long}")
        tone = tmp / "tone.ogg"
        gst("audiotestsrc", "num-buffers=350", "!", "audioconvert", "!", "vorbisenc", "!", "oggmux", "!", "filesink",
            f"location={tone}")
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        room = fixture["room"]
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        mira, _, mira_hdr = register(hs, "mira" + secrets.token_hex(3))

        def upload(headers, data, name, mime):
            return http("POST", f"{hs}/_matrix/media/v3/upload?filename={name}",
                        headers={**headers, "Content-Type": mime}, data=data).json()["content_uri"]

        def say(headers, content):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/send/m.room.message/"
                 f"{secrets.token_hex(8)}", headers=headers, json=content)
        avatar = upload(mira_hdr, gif([GREEN, YELLOW], (64, 64)), "avatar.gif", "image/gif")
        http("PUT", f"{hs}/_matrix/client/v3/profile/{quote(mira, safe='')}/avatar_url", headers=mira_hdr,
             json={"avatar_url": avatar})
        http("POST", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/invite", headers=hdr,
             json={"user_id": mira})
        http("POST", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/join", headers=mira_hdr, json={})
        # 25 frames a second, so playing it has a cost to measure.
        moving = gif([RED, BLUE] * 2, (400, 260), delay=40)
        say(hdr, {"msgtype": "m.image", "body": "moving.gif", "url": upload(hdr, moving, "moving.gif", "image/gif"),
                  "info": {"w": 400, "h": 260, "mimetype": "image/gif", "size": len(moving)}})
        say(mira_hdr, {"msgtype": "m.text", "body": "Mira with a moving picture"})
        for n in range(24):
            say(hdr, {"msgtype": "m.text", "body": f"chatter line {n}"})
        poster = io.BytesIO()
        Image.new("RGB", (320, 240), MAGENTA).save(poster, "JPEG")
        # Not clip.mp4, which P5 looks for among the newest messages.
        say(hdr, {"msgtype": "m.video", "body": "long.mp4", "url": upload(hdr, long.read_bytes(), "long.mp4",
                                                                        "video/mp4"),
                  "info": {"w": 320, "h": 240, "duration": 20000, "mimetype": "video/mp4",
                           "size": long.stat().st_size,
                           "thumbnail_url": upload(hdr, poster.getvalue(), "poster.jpg", "image/jpeg"),
                           "thumbnail_info": {"w": 320, "h": 240, "mimetype": "image/jpeg",
                                              "size": len(poster.getvalue())}}})
        say(hdr, {"msgtype": "m.audio", "body": "tone.ogg", "url": upload(hdr, tone.read_bytes(), "tone.ogg",
                                                                        "audio/ogg"),
                  "info": {"mimetype": "audio/ogg", "size": tone.stat().st_size, "duration": 8000}})

        def sent_video():
            """The newest m.video the test account sent from the client."""
            events = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/messages?dir=b&limit=10",
                          headers=hdr).json()["chunk"]
            return next((e for e in events if e["sender"] == me and e["type"] == "m.room.message"
                         and e["content"].get("msgtype") == "m.video"
                         and e["content"].get("body") == "clip.mp4"
                         and "thumbnail_url" in e["content"].get("info", {})), None)

        def media(mxc):
            server, media_id = mxc.removeprefix("mxc://").split("/", 1)
            return http("GET", f"{hs}/_matrix/client/v1/media/download/{server}/{media_id}", headers=hdr).content
        print(f"Synapse at {hs}", flush=True)
        look(tmp, args.shots, files, sent_video, media)
        print("PASS: animated pictures play on screen only, videos and audio play in the timeline, and a sent "
              "video carries its poster", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def cpu(pid):
    """The process's CPU time so far, in clock ticks."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


def look(tmp, shots, files, sent_video, media):
    from qa_kwin import KWin
    from qa_native_paste import child, isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off")
        sock = ui / "pulse.sock"
        env.update(PULSE_SERVER="unix:" + str(sock), PULSE_SINK="Out")
        pulse = stack.enter_context(child(["pulseaudio", "-n", "--daemonize=no", "--exit-idle-time=-1",
                                           "--use-pid-file=no",
                                           f"--load=module-native-protocol-unix socket={sock} auth-anonymous=1",
                                           "--load=module-null-sink sink_name=Out channels=2 rate=48000"], env))
        wait_for("private audio server", lambda: pulse.poll() is None and sock.is_socket(), 20)
        kde = KWin(stack, ui, ROOT, None, env=env)
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)
        pid = kde.native.pid

        def shot(name):
            im = kde.capture().crop(WINDOW).convert("RGB")
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=FEED):
            return [(w, x, y) for w, x, y in kde.words(rect)]

        def containing(part, rect=FEED):
            return next(((x, y) for w, x, y in words(rect) if part in w.lower()), None)

        def find(colour, rect=FEED, step=6, slack=50):
            im = kde.capture().crop(WINDOW).convert("RGB")
            px = im.load()
            pts = [(x, y) for y in range(rect[1], rect[3], step) for x in range(rect[0], rect[2], step)
                   if near(px[x, y], colour, slack)]
            if len(pts) < 20:
                return None
            xs, ys = [p[0] for p in pts], [p[1] for p in pts]
            return (min(xs), min(ys), max(xs), max(ys))

        def colours_seen(x, y, wanted, seconds=3):
            seen = set()
            end = time.monotonic() + seconds
            while time.monotonic() < end and len(seen) < len(wanted):
                px = kde.capture().crop(WINDOW).convert("RGB").getpixel((x, y))
                seen |= {c for c in wanted if near(px, c)}
                time.sleep(0.08)
            return seen == set(wanted)

        def busy(seconds=4):
            start = cpu(pid)
            time.sleep(seconds)
            return cpu(pid) - start

        wait_for("media-proof", lambda: containing("media-proof", (72, 40, 330, 840)), 90)
        tap(kde, *containing("media-proof", (72, 40, 330, 840)))
        wait_for("the tone's card", lambda: containing("tone.ogg"), 30)
        time.sleep(3)
        assert not (find(RED) or find(BLUE)), "the GIF should start scrolled away"
        away = busy()

        # P1: the GIF on screen plays, and costs CPU while it does.
        for _ in range(12):
            if find(RED) or find(BLUE):
                break
            kde.keys(("Down", PAGE_UP), ("Up", PAGE_UP))
            time.sleep(0.8)
        box = wait_for("the GIF on screen", lambda: find(RED) or find(BLUE), 20)
        gx, gy = (box[0] + box[2]) // 2, (box[1] + box[3]) // 2
        try:
            assert colours_seen(gx, gy, [RED, BLUE]), "the GIF isn't playing"
        finally:
            shot("gif")
        playing = busy()
        print(f"UI: scrolled away the GIF cost {away} CPU ticks in 4 s; on screen it played, costing {playing}",
              flush=True)
        assert away * 3 < playing, (away, playing)

        name = wait_for("Mira's message", lambda: containing("moving"), 10)
        face = (348, name[1] - 10)
        assert colours_seen(*face, [GREEN, YELLOW]), "Mira's profile picture isn't playing"
        print("UI: Mira's animated profile picture played beside her message", flush=True)

        # P2: the video plays where it is, in its poster's space.
        for _ in range(12):
            kde.keys(("Down", PAGE_DOWN), ("Up", PAGE_DOWN))
            time.sleep(0.4)
        poster = wait_for("the video's poster", lambda: find(MAGENTA), 20)
        vx, vy = (poster[0] + poster[2]) // 2, (poster[1] + poster[3]) // 2
        tap(kde, vx, vy)
        black = (0, 0, 0)

        def frame():
            """The playing video's box: its black picture, the poster gone."""
            return not find(MAGENTA) and find(black, slack=25)

        def video_clock():
            """The time in the bar over the video, while it shows."""
            box = find(black, slack=25)
            return box and any(re.match(r"\d:\d\d", w) for w, _, _ in words((box[0], box[3] - 44, box[2], box[3] + 4)))

        def peek(dx):
            kde.actions("pointer", [{"type": "pointerMove", "x": vx + dx, "y": vy}])
        try:
            box = wait_for("the video playing in place", frame, 30)
        finally:
            shot("video-playing")
        assert all(abs(a - b) <= 8 for a, b in zip(box, poster)), (box, poster)
        assert "of" not in {w.lower() for w, _, _ in words((0, 840, 1180, 900))}, "the viewer opened"
        peek(12)
        try:
            wait_for("its controls when the pointer moves", video_clock, 5)
        finally:
            shot("video-controls")
        wait_for("its controls hiding while the pointer rests", lambda: not video_clock(), 8)
        shot("video-bare")
        peek(-12)
        wait_for("its controls back when the pointer moves", video_clock, 5)
        print(f"UI: a click played the video in place in its poster's {box[2] - box[0]}x{box[3] - box[1]} px, the "
              "viewer stayed shut, and its controls showed over it when the pointer moved and hid when it rested",
              flush=True)

        # P2: the tone plays in its card, and the seek bar jumps.
        card = wait_for("the tone's card", lambda: containing("tone.ogg"), 10)
        def times():
            """The tone card's clock ("0:03/0:08" reads as one word), and where it is."""
            for w, x, y in words((card[0] - 60, card[1], 1180, card[1] + 90)):
                if (m := re.match(r"(\d):(\d\d)", w)) and y > card[1] + 20:
                    return int(m[1]) * 60 + int(m[2]), (x, y)
            return None
        start = wait_for("the tone's time", times, 10)[1]
        tap(kde, start[0] - 50, start[1])  # its play button

        def clock():
            found = times()
            return found and found[0]
        try:
            wait_for("the tone's time moving on", lambda: (c := clock()) is not None and c >= 1, 15)
        finally:
            shot("tone-playing")
        tap(kde, start[0] + 240, start[1])  # well along the seek bar
        try:
            wait_for("the tone jumping ahead", lambda: (c := clock()) is not None and c >= 5, 10)
        finally:
            shot("tone-seeked")
        print("UI: the tone played in its card, its time moved on, and a click on the seek bar jumped ahead",
              flush=True)

        # P5: a video dropped from Dolphin goes out with its poster.
        tap(kde, 700, WINDOW[3] - 53)
        dolphin = stack.enter_context(child(["dolphin", "--new-window", str(files)], kde.env))
        dw = wait_for("Dolphin window", lambda: kde.window("DropFiles.*Dolphin"), 30)
        kde.position(dw, 1190, 0, 900, 950)
        assert int(kde.command("kdotool", "getwindowpid", dw).stdout) == dolphin.pid
        kde.command("kdotool", "windowactivate", dw)
        kde.keys(("Down", "\ue009"), ("Down", "3"), ("Up", "3"), ("Up", "\ue009"))  # details view
        sx, sy = wait_for("clip.mp4 in Dolphin", lambda: next(((x, y) for w, x, y in kde.words((1190, 60, 2090, 700))
                                                              if w.lower() == "clip.mp4"), None), 20)
        kde.click(1650, 590)
        kde.select_all()
        kde.drag(sx, sy, 700, 400)
        wait_for("the video staged", lambda: containing("clip.mp4", (330, 500, 1180, 900)), 20)
        kde.keys(("Down", ENTER), ("Up", ENTER))
        event = wait_for("the sent video in the room", sent_video, 60)
        info = event["content"]["info"]
        assert (info.get("w"), info.get("h")) == (320, 240), info
        assert 1500 <= info.get("duration", 0) <= 2500, info
        thumb = Image.open(io.BytesIO(media(info["thumbnail_url"])))
        assert thumb.size == (320, 240) and thumb.format == "JPEG", (thumb.size, thumb.format)
        shot("video-sent")
        print(f"UI: the dropped video went out as m.video {info['w']}×{info['h']}, {info['duration']} ms, with a "
              f"{thumb.size[0]}×{thumb.size[1]} poster frame", flush=True)


if __name__ == "__main__":
    main()
