"""The Voice & Video settings page (calls plan U2) through the QA control map.

`Page` drives one client's page with xdotool (the X11 desktop fixtures):
open it from the user panel's gear, choose devices in its dropdowns, move
sliders with the arrow keys, record the push-to-talk key.

`check_persisted` sets one of everything, quits the client the way a window
manager asks it to, starts it again on the same disposable profile (as
`qa_linux_upgrade.py` does across an upgrade) and checks the page and
`voice.json` still have every choice.
"""
import json
import time
from pathlib import Path

from qa_native_paste import wait_for


class Page:
    """`layout` is the client's control map; `press(x, y)` clicks,
    `scroll(x, y, dy)` turns the wheel, `keys(*names)` types with the
    pointer where it is, `hover(x, y)` moves the pointer."""

    def __init__(self, layout, press, scroll, keys, hover):
        self.layout, self.press, self.scroll, self.keys, self.hover = layout, press, scroll, keys, hover

    def open(self):
        layout = self.layout
        if layout.state("settings-voice") == "selected":
            return
        if not layout.get("settings-voice"):
            self.press(*layout.wait("user-settings", timeout=20))
        self.press(*layout.wait("settings-voice"))
        wait_for("Voice & Video page", lambda: layout.state("settings-voice") == "selected"
                 and layout.get("input-mode-voice"), 8)

    def close(self):
        if self.layout.get("settings-close"):
            self.press(*self.layout.find("settings-close"))
            wait_for("User Settings closed", lambda: not self.layout.get("settings-close"), 8)

    def reveal(self, name):
        """Scroll the page until `name` shows whole; its centre."""
        for _ in range(30):
            if self.layout.inside(name, "settings-page"):
                return self.layout.find(name)
            c, area = self.layout.get(name), self.layout.get("settings-page")
            if c and area:
                x, y = self.layout.centre(area)
                self.scroll(x, y, 120 if c["y"] > area["y"] else -120)
            time.sleep(.45)
        raise RuntimeError(f"{name} not shown on the settings page; {self.layout.missing()}")

    def device(self, kind, label):
        """Choose the `kind` ("input" or "output") device whose name has
        `label` in the page's dropdown."""
        field = f"settings-{kind}"
        self.press(*self.reveal(field))
        name = wait_for(f"{label} in the page's {kind} list",
                        lambda: self.layout.item(field + "-", label), 10)
        self.press(*self.layout.find(name))
        wait_for(f"{label} chosen", lambda: not self.layout.get(field + "-menu")
                 and label.lower() in self.layout.text(field).lower(), 8)

    def slider(self, name, target, step, unit, handle=None):
        """Move the slider `handle` (`name`-slider unless given) until
        `name`'s tip shows `target`."""
        def value():
            tip = (self.layout.get(name) or {}).get("tip", "")
            return float(tip.replace(unit, "").strip()) if tip else None
        x, y = self.reveal(handle or name + "-slider")
        self.hover(x, y)
        current = wait_for(f"{name}'s value", lambda: value() is not None and (value(),), 6)[0]
        steps = round((target - current) / step)
        if steps:
            self.keys("--repeat", str(abs(steps)), "--delay", "40", "Up" if steps > 0 else "Down")
        wait_for(f"{name} at {target}{unit}", lambda: value() == target, 8)

    def click(self, name, state=None):
        self.press(*self.reveal(name))
        if state:
            wait_for(f"{name} {state}", lambda: self.layout.state(name) == state, 8)


def check_persisted(page, profile, quit_client, start_client, devices, evidence=lambda name: None):
    """One of everything on the page, then a restart: it's all still there.

    `devices` names an input and an output other than the defaults, as the
    page lists them and as PulseAudio does: ((label, name), (label, name)).
    `quit_client()` closes the window and waits for the process to exit;
    `start_client()` starts it again on the same profile and returns once
    its window shows."""
    (mic_label, mic), (speaker_label, speaker) = devices
    voice = Path(profile) / "voice.json"
    page.open()
    page.device("input", mic_label)
    page.device("output", speaker_label)
    page.slider("input-volume", 150, 5, "%")
    page.slider("output-volume", 60, 5, "%")
    if page.layout.state("voice-auto") == "on":
        page.click("voice-auto", "off")
    page.slider("voice-slider", -42, 1, "dB", handle="voice-slider")
    page.click("voice-echo", "on" if page.layout.state("voice-echo") == "off" else "off")
    echo = page.layout.state("voice-echo")
    page.click("input-mode-ptt", "selected")
    page.click("ptt-key-record", "on")
    wait_for("the binding dialog", lambda: page.layout.get("ptt-key-dialog"), 6)
    evidence("u2-binding-dialog")
    page.keys("F9")
    wait_for("F9 bound", lambda: not page.layout.get("ptt-key-dialog") and page.layout.text("ptt-key") == "F9", 8)
    page.slider("ptt-release", 300, 20, "ms")
    evidence("u2-push-to-talk")
    expected = {"input_device": mic, "output_device": speaker, "input_volume": 150, "output_volume": 60,
                "auto_sensitivity": False, "threshold": -42.0, "echo_cancellation": echo == "on",
                "push_to_talk": True, "talk_key": "F9", "release_delay": 300}

    def saved():
        try:
            data = json.loads(voice.read_text())
        except (FileNotFoundError, ValueError):
            return None
        return data if all(data.get(k) == v for k, v in expected.items()) else None
    wait_for("voice.json with every choice", saved, 8)
    print("Voice & Video page: chose " + mic_label + " and " + speaker_label + ", input 150 %, output 60 %, "
          "threshold -42 dB, echo cancellation " + echo + ", Push to Talk on F9 (binding dialog), "
          "release delay 300 ms; voice.json saved each.", flush=True)
    page.close()
    quit_client()
    assert saved(), voice.read_text()
    start_client()
    page.open()

    def shown():
        layout = page.layout
        return (mic_label.lower() in layout.text("settings-input").lower()
                and speaker_label.lower() in layout.text("settings-output").lower()
                and (layout.get("input-volume") or {}).get("tip") == "150%"
                and (layout.get("output-volume") or {}).get("tip") == "60%"
                and layout.state("input-mode-ptt") == "selected"
                and layout.text("ptt-key") == "F9"
                and (layout.get("ptt-release") or {}).get("tip") == "300 ms"
                and layout.state("voice-echo") == echo)
    try:
        wait_for("every choice on the page after the restart", shown, 15)
    except RuntimeError:
        print("After the restart:", page.layout.missing(), flush=True)
        raise
    # The threshold shows with Voice Activity.
    page.click("input-mode-voice", "selected")
    wait_for("threshold after the restart", lambda: (page.layout.get("voice-slider") or {}).get("tip") == "-42 dB"
             and page.layout.state("voice-auto") == "off", 8)
    page.close()
    print("Quit and started again on the same profile: the page shows every choice "
          "(devices, volumes, Push to Talk on F9 with 300 ms, threshold -42 dB, echo cancellation).", flush=True)
