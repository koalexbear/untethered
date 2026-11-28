"""Voice processing and input sensitivity (calls plan, step A2), measured at
the other voice-room clients' speakers.

Runs inside --voice-room (`--voice-processing`), whose clients all start
from saved settings with every switch off and the gate below the test tones
(`qa_events.RAW_VOICE`). Each switch is clicked in Alice's Voice Settings
popup, and her `voice.json` must follow. The signals are speech-like (syllables) and
the far end is silent or speaking in syllables: a tone that never pauses
looks like noise to noise suppression (held about 13 dB down), and as the
far end it makes the echo canceller suppress the near end (see `voice.rs`'s
tests and `qa_events.RAW_VOICE`).

- Echo: PulseAudio loops Alice's speaker back into her microphone (60 ms,
  -6 dB). Bruno talks in voiced 670 Hz syllables. With processing off he
  hears himself come back; with echo cancellation on, the returned 670 Hz
  drops by at least 25 dB for 12 consecutive windows after 2 s, while
  Alice's 410 Hz still reaches him: at half its level or more in the median
  window, never more than 26 dB down (the canceller ducks the near end while
  the far end talks).
- Noise: pink noise under 410 Hz syllables. Noise suppression lowers the
  noise between syllables at Bruno by at least 10 dB and keeps the
  syllables within 3 dB.
- Gate, manual threshold at -40 dB: quiet noise sends silence to Bruno and
  Clara, no roster shows Alice speaking, her microphone stays unmuted at the
  SFU, and the meter stays below the threshold. A loud onset passes at most
  100 ms later than through an open gate.
- All processing on with the gate open: a steady tone holds within 3 dB.
- Defaults (all on, automatic sensitivity): steady noise is learned and
  gated; syllables over it pass and light Alice's ring on every roster, hers
  from her own gate. Then every switch goes back off from the panel for the
  voice-room fixture's steady tones that follow.
"""
import array
import cmath
import json
import math
import time
from collections import deque
from statistics import median

from qa_desktop_dm import sfu_participants
from qa_native_paste import wait_for
from qa_ui import near
from qa_events import RAW_VOICE as RAW
from qa_voice_room import SILENCE, highlights

RATE = 48000
FS = 12000  # The fixture's speaker analysis rate (qa_voice_room.Desktop.capture).
WINDOW = FS // 10
# The meter's colours (view/voice_settings.rs): below and above the gate.
BELOW = (0xF0, 0xB2, 0x32)
ABOVE = (0x23, 0xA5, 0x5A)


# Signals: lists of 48 kHz samples, whole 100 ms pieces for Desktop.feed.
def pcm(samples):
    return array.array("h", (max(-32768, min(32767, round(v))) for v in samples)).tobytes()


def tone(hz, amplitude, seconds=1.0):
    return [amplitude * math.sin(math.tau * hz * n / RATE) for n in range(int(RATE * seconds))]


def syllables(hz, amplitude, *, on=0.15, period=0.25, harmonics=1, seconds=1.0):
    """`on` seconds of `hz` (and its next harmonics, falling off like a
    voice's) in every `period`, with 5 ms ramps."""
    ramp, out = int(RATE * 0.005), []
    length, sounding = int(RATE * period), int(RATE * on)
    for n in range(int(RATE * seconds)):
        k = n % length
        gain = 0.0 if k >= sounding else min(1.0, k / ramp, (sounding - k) / ramp)
        if gain:
            t = n / RATE
            gain *= sum(math.sin(math.tau * hz * h * t) / h for h in range(1, harmonics + 1))
        out.append(amplitude * gain)
    return out


def pink(dbfs, seconds=4.0, seed=7):
    """Pink noise at an RMS of `dbfs` (Paul Kellet's economy filter)."""
    state, b0, b1, b2, out = seed, 0.0, 0.0, 0.0, []
    for _ in range(int(RATE * seconds)):
        state ^= state << 13 & (1 << 64) - 1
        state ^= state >> 7
        state ^= state << 17 & (1 << 64) - 1
        w = (state >> 11) / (1 << 53) * 2 - 1
        b0 = 0.99765 * b0 + w * 0.0990460
        b1 = 0.96300 * b1 + w * 0.2965164
        b2 = 0.57000 * b2 + w * 1.0526913
        out.append(b0 + b1 + b2 + w * 0.1848)
    scale = 32767 * 10 ** (dbfs / 20) / math.sqrt(sum(v * v for v in out) / len(out))
    return [v * scale for v in out]


def mix(*signals):
    length = max(map(len, signals))
    return [sum(s[n % len(s)] for s in signals) for n in range(length)]


# Analysis of Desktop.capture windows: (time read, 12 kHz samples).
BASES = {}


def amplitude(samples, hz):
    basis = BASES.get((hz, len(samples)))
    if basis is None:
        basis = BASES[hz, len(samples)] = [cmath.exp(-1j * math.tau * hz * n / FS) for n in range(len(samples))]
    return 2 * abs(sum(s * c for s, c in zip(samples, basis))) / len(samples)


def rms(samples):
    return math.sqrt(sum(s * s for s in samples) / len(samples))


def db(ratio):
    return 20 * math.log10(max(ratio, 1e-9))


def percentile(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int(p * len(values)))]


def record(desk, seconds):
    desk.capture = []
    time.sleep(seconds)
    windows, desk.capture = desk.capture, None
    return [samples for _, samples in windows if len(samples) == WINDOW]


def listen(*desks):
    for d in desks:
        d.capture = deque(maxlen=40)


def latest(desk, n):
    windows = [s for _, s in list(desk.capture or ()) if len(s) == WINDOW]
    return windows[-n:] if len(windows) >= n else None


class Panel:
    """Alice's voice settings: in the Voice Settings popup from her Voice
    Connected panel until the Voice & Video page (U2). Its switches are
    found through the control map (qa_layout.py)."""

    # The words the steps name a switch by, and its control.
    LINES = {
        ("input", "sensitivity"): "voice-auto",
        ("automatically", "determine"): "voice-auto",
        ("echo", "cancellation"): "voice-echo",
        ("noise", "suppression"): "voice-noise",
        ("automatic", "gain", "control"): "voice-gain",
        ("threshold",): "voice-slider",
    }

    def __init__(self, desk, profile):
        self.desk = desk
        self.file = profile / "voice.json"

    def saved(self):
        try:
            return json.loads(self.file.read_text())
        except (FileNotFoundError, ValueError):
            return None

    def wait_saved(self, **expected):
        def matches():
            saved = self.saved()
            return saved if saved and all(saved.get(k) == v for k, v in expected.items()) else None
        return wait_for(f"voice.json with {expected}", matches, 8)

    def find(self, *line):
        """The centre of the control for `line`, scrolled into view."""
        return self.desk.reveal(self.LINES[line])

    def toggle(self):
        """Open or close Voice Settings with the panel's gear."""
        layout = self.desk.layout
        before = layout.state("voice-settings-open")
        self.desk.press(*layout.wait("voice-settings-open"))
        wait_for("Voice Settings " + ("closed" if before == "on" else "open"),
                 lambda: layout.state("voice-settings-open") not in (before, None), 6)

    def switch(self, line, key, on):
        x, y = self.find(*line)
        self.desk.press(x, y)
        self.wait_saved(**{key: on})
        print(f"Alice's Voice Settings: {' '.join(line).capitalize()} {'on' if on else 'off'}; voice.json saved it.", flush=True)

    def threshold(self, target):
        """Move the manual threshold with the arrow keys over its slider."""
        current = self.saved()["threshold"]
        x, y = self.find("threshold")
        d = self.desk
        d.cmd("xdotool", "windowfocus", d.win, "mousemove", "--window", d.win, str(x), str(y))
        steps = round(target - current)
        d.cmd("xdotool", "key", "--repeat", str(abs(steps)), "--delay", "40", "Up" if steps > 0 else "Down")
        self.wait_saved(threshold=float(target))
        print(f"Alice's threshold slider: {current:.0f} dB to {target:.0f} dB with the arrow keys; voice.json saved it.", flush=True)

    def meter(self):
        """Pixels of the manual meter's bright colours: (below, above) the
        threshold."""
        self.desk.reveal("voice-meter")
        left, top, right, bottom = self.desk.layout.box(self.desk.layout.get("voice-meter"))
        im = self.desk.image()
        below = above = 0
        for row in range(int(top), int(bottom)):
            for x in range(int(left), int(right)):
                c = im.getpixel((x, row))
                below += near(c, BELOW, 24)
                above += near(c, ABOVE, 24)
        return below, above


def check_processing(desks, names, secret, users, dirs):
    alice, bruno, clara = desks

    def rosters(what, peers, active):
        try:
            wait_for(what, lambda: all(highlights(d, names) == active for d in peers), 12)
        except RuntimeError:
            print("Rosters read:", {names[d.i]: highlights(d, names) for d in peers}, flush=True)
            raise
    panel = Panel(alice, dirs[0] / "seed")
    assert panel.saved() == RAW, panel.saved()
    panel.toggle()
    for line in (("input", "sensitivity"), ("echo", "cancellation"), ("noise", "suppression"),
                 ("automatic", "gain", "control")):
        panel.find(*line)
    print("Alice's Voice Settings show Input sensitivity, Echo cancellation, Noise suppression and "
          "Automatic gain control; all off as her saved voice.json says.", flush=True)

    # Echo: Alice's speaker plays into her microphone.
    voiced = pcm(syllables(670, 4000, harmonics=4))
    alice.tone, bruno.tone, clara.tone = 0, voiced, SILENCE
    module = alice.cmd("pactl", "load-module", "module-loopback", "source=Out0.monitor", "sink=Mic0",
                       "latency_msec=60", "source_dont_move=true", "sink_dont_move=true").strip()
    try:
        loop = next(v["index"] for v in json.loads(alice.cmd("pactl", "-f", "json", "list", "sink-inputs"))
                    if str(v.get("owner_module")) == module)
        # -6 dB, as a linear factor: a leading minus reads as relative.
        alice.cmd("pactl", "set-sink-input-volume", str(loop), "0.501")
        time.sleep(3)
        windows = record(bruno, 4)
        echo = percentile([amplitude(w, 670) for w in windows], 0.9)
        talker = median(amplitude(w, 410) for w in windows)
        print(f"Processing off: Bruno hears his own 670 Hz back at {echo:.0f} (90th percentile) "
              f"and Alice's 410 Hz at {talker:.0f}.", flush=True)
        assert echo > 600, f"the loopback returned only {echo:.0f}: no echo to cancel"
        panel.switch(("echo", "cancellation"), "echo_cancellation", True)
        time.sleep(2)
        listen(bruno)
        limit = echo * 10 ** (-25 / 20)

        def cancelled():
            last = latest(bruno, 12)
            if not last:
                return None
            returned = [amplitude(w, 670) for w in last]
            kept = [amplitude(w, 410) for w in last]
            # The canceller ducks the near end while the far end talks (here
            # all the time): Alice must stay audible, mostly at full level.
            heard = median(kept) >= talker / 2 and min(kept) >= talker / 20
            return (max(returned), median(kept), min(kept)) if max(returned) <= limit and heard else None
        try:
            returned, kept, lowest = wait_for("returned 670 Hz down 25 dB, Alice's 410 Hz kept", cancelled, 20)
        except RuntimeError:
            last = latest(bruno, 12) or []
            print("Last windows at Bruno, 670/410 Hz:", [(round(amplitude(w, 670)), round(amplitude(w, 410))) for w in last])
            raise
        bruno.capture = None
        print(f"Echo cancellation on: the returned 670 Hz stayed at or below {returned:.0f} for 12 consecutive "
              f"100 ms windows ({db(echo / max(returned, 1)):.1f} dB down); Alice's 410 Hz reached him at a median of "
              f"{kept:.0f}, lowest {lowest:.0f} ({db(lowest / talker):+.1f} dB, ducked under Bruno's syllables).",
              flush=True)
    finally:
        alice.cmd("pactl", "unload-module", module)

    # Noise: pink noise under syllables, no far end.
    bruno.tone = SILENCE
    alice.tone = pcm(mix(pink(-30), syllables(410, 5000, on=0.4, period=0.8, seconds=4.0)))

    def noise_and_speech():
        time.sleep(3)
        windows = record(bruno, 8)
        tones = [amplitude(w, 410) for w in windows]
        loud = percentile(tones, 0.75)
        floor = median(rms(w) for w, a in zip(windows, tones) if a < loud / 10)
        speech = median(a for a in tones if a > loud / 2)
        return floor, speech
    floor, speech = noise_and_speech()
    panel.switch(("noise", "suppression"), "noise_suppression", True)
    quiet, kept = noise_and_speech()
    print(f"Noise suppression: noise between syllables at Bruno {floor:.0f} to {quiet:.0f} RMS "
          f"({db(floor / quiet):.1f} dB down); syllables {speech:.0f} to {kept:.0f} ({db(kept / speech):+.1f} dB).",
          flush=True)
    assert db(floor / quiet) >= 10, "noise suppression lowered the noise less than 10 dB"
    assert abs(db(kept / speech)) <= 3, "noise suppression changed the syllables by more than 3 dB"

    # The gate, with a manual threshold.
    panel.find("automatically", "determine")
    panel.threshold(-40)
    quiet_noise, loud = pcm(pink(-48, 2.0)), pcm(tone(410, 7000, 1.0))
    alice.tone = quiet_noise
    listen(bruno, clara)

    def silent():
        heard = [latest(d, 12) for d in (bruno, clara)]
        if not all(heard):
            return None
        worst = max(max(rms(w), *(amplitude(w, hz) for hz in (410, 670, 1030))) for last in heard for w in last)
        # A tuple: perfect silence (0.0) would read as "not yet".
        return (worst,) if worst < 30 else None
    worst, = wait_for("silence from Alice's gate at Bruno and Clara", silent, 10)
    # With her Voice Settings open Alice's own roster isn't read; Bruno's and
    # Clara's show what the SFU reports. Her own ring is checked below.
    rosters("nobody speaking at Bruno and Clara", (bruno, clara), set())
    alice_id = users[0]
    mic = next(t for p in sfu_participants(secret) if p["identity"].rsplit(":", 1)[0] == alice_id
               for t in p.get("tracks", []) if t.get("type", "AUDIO") == "AUDIO")
    assert not mic.get("muted"), mic
    below, above = panel.meter()
    assert below > 20 and above < 10, (below, above)
    print(f"Gate at -40 dB, quiet noise: Bruno and Clara decoded silence (12 windows, worst {worst:.0f}); "
          f"their rosters don't show Alice speaking; her microphone stays unmuted at the SFU; "
          f"her meter shows the level below the threshold ({below} px below, {above} px above).", flush=True)
    alice.tone = loud
    wait_for("Alice's loud tone through the gate", lambda: (last := latest(bruno, 6)) and
             min(amplitude(w, 410) for w in last) > 500, 10)
    rosters("Alice speaking at Bruno and Clara", (bruno, clara), {0})
    below, above = panel.meter()
    assert above > 20, (below, above)
    print(f"Loud tone: it reaches Bruno, Bruno's and Clara's rosters show Alice speaking, her meter goes past the "
          f"threshold ({above} px above).", flush=True)
    for d in (bruno, clara):
        d.capture = None

    def onsets():
        delays = []
        for _ in range(5):
            alice.tone = quiet_noise
            time.sleep(1.5)
            bruno.capture = []
            alice.tone = loud
            time.sleep(1.2)
            windows, bruno.capture = bruno.capture, None
            started = alice.fed
            full = median(amplitude(s, 410) for _, s in windows[-4:])
            onset = next((t - (WINDOW - k * 120) / FS for t, s in windows if len(s) == WINDOW
                          for k in range(10) if amplitude(s[k * 120:(k + 1) * 120], 410) >= full / 2), None)
            assert onset is not None, "the loud tone never arrived"
            delays.append(onset - started)
        return median(delays), delays
    gated, gated_all = onsets()
    panel.threshold(-100)
    opened, opened_all = onsets()
    print(f"Loud onset to Bruno: {gated * 1000:.0f} ms through the closed gate, {opened * 1000:.0f} ms with it "
          f"always open (medians of {[round(d * 1000) for d in gated_all]} and "
          f"{[round(d * 1000) for d in opened_all]} ms).", flush=True)
    assert gated - opened <= 0.1, "the gate held a loud onset back more than 100 ms"

    # All processing on, the gate still open: gain control on a steady tone.
    panel.switch(("automatic", "gain", "control"), "gain_control", True)
    alice.tone = pcm(tone(410, 7000, 1.0))
    time.sleep(4)
    windows = record(bruno, 10)
    seconds = [median(amplitude(w, 410) for w in windows[n:n + 10]) for n in range(0, len(windows) - 9, 10)]
    middle = median(seconds)
    drift = max(abs(db(s / middle)) for s in seconds)
    print(f"All processing on, steady 410 Hz: {middle:.0f} at Bruno ({db(middle / talker):+.1f} dB from unprocessed: "
          f"noise suppression holds a tone that never pauses down), within {drift:.1f} dB over {len(seconds)} s.",
          flush=True)
    assert drift <= 3, "the steady tone drifted more than 3 dB"

    # The defaults: the automatic gate as well.
    panel.switch(("automatically", "determine"), "auto_sensitivity", True)
    saved = panel.saved()
    assert all(saved[k] for k in ("echo_cancellation", "noise_suppression", "gain_control", "auto_sensitivity")), saved
    # Close the panel: Alice's own roster shows her ring from the gate.
    panel.toggle()
    alice.tone = pcm(pink(-35))
    started = time.monotonic()
    listen(bruno, clara)
    try:
        worst, = wait_for("steady noise learned and gated", silent, 20)
    except RuntimeError:
        for d in (bruno, clara):
            print(f"{names[d.i]}'s last windows, RMS:", [round(rms(w)) for w in latest(d, 30) or []], flush=True)
        raise
    learned = time.monotonic() - started
    rosters("nobody speaking on any roster", desks, set())
    print(f"Defaults, steady noise at -35 dBFS: the automatic gate learned it; Bruno and Clara decoded silence "
          f"{learned:.1f} s after it started (12 windows, worst {worst:.0f}); no roster shows Alice speaking, "
          "her own included.", flush=True)
    alice.tone = pcm(mix(pink(-35), syllables(410, 6000, harmonics=3, seconds=4.0)))

    def speech():
        last = latest(bruno, 12)
        return last and percentile([amplitude(w, 410) for w in last], 0.75) > 500
    wait_for("syllables over the noise at Bruno", speech, 10)
    rosters("Alice speaking on every roster", desks, {0})
    print("Defaults, syllables over that noise: they reach Bruno and every roster shows Alice speaking, "
          "her own from the gate.", flush=True)
    for d in (bruno, clara):
        d.capture = None

    # Back to the fixtures' settings for the steady tones that follow.
    alice.tone, bruno.tone, clara.tone = 0, 1, 2
    panel.toggle()
    panel.switch(("echo", "cancellation"), "echo_cancellation", False)
    panel.switch(("noise", "suppression"), "noise_suppression", False)
    panel.switch(("automatic", "gain", "control"), "gain_control", False)
    panel.switch(("automatically", "determine"), "auto_sensitivity", False)
    panel.threshold(RAW["threshold"])
    assert panel.saved() == RAW, panel.saved()
    panel.toggle()
    print("Every switch back off from the panel; voice.json is the fixtures' unprocessed settings again.", flush=True)
