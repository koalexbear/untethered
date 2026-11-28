"""The QA control map: Untethered's icon-only call controls by name.

A client started with `UNTETHERED_QA_LAYOUT=<file>` (any build) writes its
tagged controls to that file every 300 ms when they change
(rust-client/src/qa_layout.rs): each control's bounds in window coordinates,
a state ("on", "off", "live", "selected"…), its tooltip and the text inside
it. The call fixtures click through it instead of hovering icons and reading
their tooltips with OCR.

The call's controls (calls plan U1, U2):
  user panel        mute (off/on/deafened), mute-menu, deafen (off/on), deafen-menu,
                    user-settings (the gear: User Settings)
  device menus      input-menu, input-device-N (text: the device, state
                    "selected"), input-refresh, input-settings (opens the
                    Voice & Video page); output-… alike
  Voice Connected   call-status (text; state "connected"), call-room,
                    voice-settings-open, disconnect, camera and screen
                    (off/starting/live), call-notes (text), share-sound-panel
                    (Play shared audio, while someone shares)
  Voice Settings    voice-settings, voice-settings-page (opens the page),
                    app-audio-N (text: the app; state "selected"),
                    app-audio-stop, devices-refresh, share-sound, volume-N
                    (text: name and percent), volume-slider-N, volume-reset-N
  voice header      ptt-header (Push-to-talk beside Join voice)
  User Settings     settings-account and settings-voice (the sidebar; state
                    "selected"), settings-page (the scrolling page),
                    settings-close
  Voice & Video     settings-input (text: the device; state on while its list
                    is open), settings-input-menu, settings-input-N (state
                    "selected"), settings-input-refresh; settings-output-… alike;
                    input-volume and output-volume (tip "100%"), with
                    …-volume-slider; mic-test (on/off), mic-meter (state
                    "voice"; tip the level, "-42 dB"); input-mode-voice and
                    input-mode-ptt (state "selected"); voice-auto, voice-slider
                    (tip the threshold), voice-meter; ptt-key (text: the key),
                    ptt-key-record, ptt-key-default, ptt-key-dialog,
                    ptt-key-cancel, ptt-release (tip "100 ms") and
                    ptt-release-slider; voice-echo, voice-noise, voice-gain
                    (on/off); camera-item-N (text: the camera; state
                    "selected"), cameras-refresh, camera-preview
                    (off/starting/live), camera-test (on/off)
"""
import json
import time
from pathlib import Path

from qa_native_paste import wait_for

# Old tooltip and button words, as the fixtures name controls, and the
# control each means now, with the state it must be in to mean it.
WORDS = {
    "mute": ("mute", "off"), "unmute": ("mute", "on"),
    "deafen": ("deafen", "off"), "undeafen": ("deafen", "on"),
    "leave": ("disconnect", None), "hang": ("disconnect", None), "disconnect": ("disconnect", None),
    "start": ("camera", "off"), "camera": ("camera", None),
    "share": ("screen", "off"), "screen": ("screen", None),
    "settings": ("voice-settings-open", None), "mixer": ("voice-settings-open", None),
    "devices": ("voice-settings-open", None),
    "play": ("share-sound-panel", None),
}


class Layout:
    """The map one client writes. `origin` is where its window content
    starts in the caller's coordinates (below a KWin title bar, say)."""

    def __init__(self, path, origin=(0, 0)):
        self.path = Path(path)
        self.ox, self.oy = origin

    def controls(self):
        try:
            return json.loads(self.path.read_text())["controls"]
        except (FileNotFoundError, ValueError, KeyError):
            return {}

    def get(self, name):
        return self.controls().get(name)

    def state(self, name):
        c = self.get(name)
        return c and c["state"]

    def text(self, name):
        c = self.get(name)
        return (c or {}).get("text", "")

    def box(self, c):
        """A control's (left, top, right, bottom) in the caller's coordinates."""
        return (self.ox + c["x"], self.oy + c["y"], self.ox + c["x"] + c["w"], self.oy + c["y"] + c["h"])

    def centre(self, c):
        left, top, right, bottom = self.box(c)
        return int((left + right) / 2), int((top + bottom) / 2)

    def find(self, name, state=None):
        """The centre of `name`, if it's shown (in `state`, if given)."""
        c = self.get(name)
        if not c or (state is not None and c["state"] != state):
            return None
        return self.centre(c)

    def wait(self, name, state=None, timeout=12):
        return wait_for(f"{name} in the control map" + (f" ({state})" if state else ""),
                        lambda: self.find(name, state), timeout)

    def item(self, prefix, word):
        """A menu item `prefix`-N whose text has `word`: its name."""
        word = word.lower()
        for name, c in self.controls().items():
            if name.startswith(prefix) and name[len(prefix):].isdecimal() and (
                    word == c["text"].lower() or word in c["text"].lower().split()):
                return name
        return None

    def items(self, prefix):
        """Every `prefix`-N, in order, as (name, control)."""
        found = [(n, c) for n, c in self.controls().items() if n.startswith(prefix) and n[len(prefix):].isdecimal()]
        return sorted(found, key=lambda nc: int(nc[0][len(prefix):]))

    def inside(self, name, area):
        """Whether `name` lies wholly inside the control `area` (a popup
        that may scroll it out of view)."""
        controls = self.controls()
        c, a = controls.get(name), controls.get(area)
        if not c or not a:
            return False
        l, t, r, b = self.box(c)
        al, at, ar, ab = self.box(a)
        return al <= l and at <= t and r <= ar and b <= ab

    def updated(self, since):
        """Whether the map was written after `since` (time.time())."""
        try:
            return self.path.stat().st_mtime > since
        except FileNotFoundError:
            return False

    def missing(self):
        """Why there's no map, for a failing fixture's report."""
        if not self.path.exists():
            return f"no control map at {self.path}: is the client older than the map (calls plan U1)?"
        return "control map: " + ", ".join(f"{n}={c['state'] or '-'}" for n, c in sorted(self.controls().items()))


def resolve(layout, label):
    """The control an old label means, now: its name, or None."""
    label = label.lower()
    if label in ("stop", "cancel", "sharing"):
        # Whichever of Screen and Camera is on or starting, Screen first.
        for name in ("screen", "camera"):
            if layout.state(name) in ("live", "starting"):
                return name
        return None
    if label in WORDS:
        name, state = WORDS[label]
        return name if layout.find(name, state) else None
    return None


def settle():
    """Long enough for the client to write the map again (every 300 ms)."""
    time.sleep(0.4)
