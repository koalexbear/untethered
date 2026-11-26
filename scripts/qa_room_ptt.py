"""Drive Alice's push-to-talk key and measure all three outputs.

Without `mode`, Right Ctrl on her private X display reaches Untethered only
while it has focus (the in-window fallback; global shortcuts are off). "x11"
holds Right Ctrl while an xmessage window has focus: XInput2 raw events.
"wayland" holds the GlobalShortcuts portal's Pause key under a private KWin
while a PySide decoy has focus."""
import json
import os
import re
import subprocess as sp
import time
from pathlib import Path

from qa_native_paste import child, wait_for

PAUSE = ""  # WebDriver's Pause key, for KDE's input synthesizer
QT_PAUSE = 0x1000008


class X11Keys:
    name = "Right Ctrl"

    def __init__(self, stack, alice):
        self.alice = alice
        # Below the 1180×760 client, clear of the call dock's hover tooltips.
        stack.enter_context(child(["xmessage", "-title", "PTT focus target", "-geometry", "220x80+10+790",
                                   "Isolated focus test"], alice.env))
        self.target = wait_for("private focus target", lambda: alice.cmd(
            "xdotool", "search", "--onlyvisible", "--name", "^PTT focus target$", check=False).strip())

    def focused(self):
        return int(self.alice.cmd("xdotool", "getwindowfocus").strip())

    def focus(self, win):
        self.alice.cmd("xdotool", "windowfocus", "--sync", win)
        assert self.focused() == int(win)

    def stays(self):
        # Without a window manager, clicking Untethered's controls moves no focus.
        assert self.focused() == int(self.target), "a click moved focus off the other window"

    def key(self, down):
        self.alice.cmd("xdotool", "keydown" if down else "keyup", "Control_R")


class PortalKeys:
    name = "Pause"

    def __init__(self, stack, alice):
        self.kde = kde = alice.kde
        self.log = kde.tmp / "decoy-keys.log"
        self.log.touch()
        stack.enter_context(child([os.environ.get("PYSIDE_PYTHON", "/usr/bin/python"),
                                   str(Path(__file__).with_name("qa_ptt_decoy.py")), str(self.log)], kde.env))
        self.target = wait_for("PySide decoy window", lambda: kde.window("^PTT decoy$"))
        kde.position(self.target, 1200, 0, 400, 200)
        self.hold = None

        def release():
            self.key(False)
        stack.callback(release)

    def focused(self):
        return self.kde.command("kdotool", "getactivewindow", check=False).stdout.strip()

    def focus(self, win):
        self.kde.command("kdotool", "windowactivate", win)
        wait_for("KWin focus", lambda: self.focused() == win, 5)

    def stays(self):
        # A click activates Untethered under KWin; return to the decoy.
        self.focus(self.target)

    def synth(self, *actions):
        path = self.kde.tmp / f"keys-{time.monotonic_ns()}.json"
        path.write_text(json.dumps({"actions": [{"type": "key", "id": "qa", "actions": list(actions)}]}))
        return path

    def key(self, down):
        if down and not self.hold:
            # KWin drops a fake input device's keys when its client exits, so
            # one helper holds the key until it is terminated.
            script = self.synth({"type": "keyDown", "value": PAUSE}, {"type": "pause", "duration": 600000})
            self.hold = sp.Popen([self.kde.synth, str(script)], env=self.kde.env,
                                 stdout=sp.DEVNULL, stderr=sp.DEVNULL)
        elif not down and self.hold:
            self.hold.terminate()
            self.hold.wait(timeout=5)
            self.hold = None

    def typed(self):
        return [line.split() for line in self.log.read_text().splitlines()]

    def type(self, ch):
        self.kde.command(self.kde.synth, str(self.synth({"type": "keyDown", "value": ch},
                                                        {"type": "keyUp", "value": ch})))


def check_ptt(stack,desks,audio_counts,same_tracks,mode=None):
    alice,bruno,clara=desks
    # Both peers recorded every speaker window from before the first join.
    for peer in (bruno,clara):
        peak=max(range(len(peer.audit)),key=lambda i:peer.audit[i][1][0],default=None)
        if peak is None or peer.audit[peak][1][0]>=200:
            start=peer.audit[0][0] if peer.audit else 0
            near=[(round(t-start,1),[round(a) for a in f]) for t,f in peer.audit[max(0,(peak or 0)-3):(peak or 0)+4]]
            raise AssertionError(("Alice audible before first press",peer.i,len(peer.audit),near))
        peer.audit=[]

    keys=PortalKeys(stack,alice) if mode=="wayland" else X11Keys(stack,alice)
    name,key,focus=keys.name,keys.key,keys.focus
    # The call dock names the key that works: "Hold Right Ctrl to talk while
    # Untethered is focused." in-window, "Hold <key> to talk." desktop-wide.
    def hint():
        words=[w for w,_,_ in alice.words()]
        return "talk" in words and (("focused" in words)==(mode is None)) and name.split()[-1].lower() in words
    wait_for(f"push-to-talk hint for {name}",hint,15)
    def edge(down,stage,tone):
        # Bound end-to-end response as observed by each speaker monitor,
        # including input delivery, jitter buffer and the 100 ms analysis window.
        before=time.monotonic()
        key(down)
        check(stage,tone if down else None)
        band={410:0,530:3}[tone]
        ms=[]
        for peer in (bruno,clara):
            after=[(t,f[band]) for t,f in peer.audit if t>=before]
            # First window after the last mismatch: a brief hole followed by
            # more speech must not count as a successful release.
            start=1+max((i for i,(_,amp) in enumerate(after) if
                         (amp<500 if down else amp>=200)),default=-1)
            assert len(after)-start>=11,(stage,after)
            ms.append(round(1000*(after[start][0]-before)))
        assert max(ms)<800,(stage,ms)
        print(f"  Key {'down' if down else 'up'} to stable speaker windows: Bruno {ms[0]} ms, Clara {ms[1]} ms.",flush=True)
    def check(stage,tone=None,deaf=False):
        for d in desks:d.windows.clear()
        sent={tone} if tone else set()
        expect=(set() if deaf else {670,1030},{1030}|sent,{670}|sent)
        wait_for(stage,lambda:all(d.hears(hz) for d,hz in zip(desks,expect)),10)
        assert audio_counts()==[(int(tone is not None),1),(1,1),(1,1)] and same_tracks()
        print(f"{stage}: Bruno heard {sorted(expect[1])} Hz; Clara heard {sorted(expect[2])} Hz; "
              f"Alice mic streams: {int(tone is not None)}.",flush=True)
    def click(label):
        alice.click(label)
        if mode:keys.stays()

    try:
        if mode:
            # Focus goes to the other window before the first press and stays
            # there for every hold.
            focus(keys.target)
            check("Joined with PTT armed, another window focused")
            edge(True,f"{name} held in the other window",410)
            alice.tone=3
            check("Fresh input during same hold",530)
            edge(False,f"{name} released with 530 Hz still at input",530)
            edge(True,"Second press",530)
            # Desktop-wide, focus changes neither end a hold nor start one.
            focus(alice.win)
            check("Focus moved to Untethered during the hold",530)
            focus(keys.target)
            check("Focus moved away again during the hold",530)
            edge(False,"Released in the other window",530)
            focus(alice.win)
            check("Focus returned without a press")
            focus(keys.target)
        else:
            focus(alice.win)
            check("Joined with PTT armed")
            edge(True,"Right Ctrl held",410)
            alice.tone=3
            check("Fresh input during same hold",530)
            edge(False,"Right Ctrl released with 530 Hz still at input",530)
            edge(True,"Second press",530)
            focus(keys.target)
            check("Focus lost before key-up")
            focus(alice.win)  # Key repeat on refocus must not reopen capture.
            check("Focus returned with Right Ctrl still held")
            focus(keys.target)
            key(False)  # Delivered to the other window.
            focus(alice.win)
            check("Focus returned without a press")
        click("mute")
        key(True)
        check(f"{name} cannot override manual mute")
        click("unmute")
        check("Unmute during hold still needs a fresh press")
        key(False)
        alice.tone=0
        key(True)
        check("Fresh press after unmute",410)
        click("deafen")
        check("Deafen during hold",deaf=True)
        click("undeafen")
        check("Undeafen still needs a fresh press")
        key(False)
        key(True)
        check("Fresh press after undeafen",410)
        key(False)
        check("Last release")
        if mode=="wayland":
            # The compositor keeps a bound shortcut from the focused window.
            focus(keys.target)
            keys.type("x")
            wait_for("ordinary key in the decoy",lambda:["press","0x58","'x'"] in keys.typed(),5)
            stray=[k for k in keys.typed() if int(k[1],16)==QT_PAUSE]
            assert not stray,stray
            print(f"The decoy kept focus for every hold and received no {name} key events "
                  f"({len(keys.typed())} other key events, including a typed x).",flush=True)
        click("push-to-talk")
        check("Open mic restored",410)
    finally:
        key(False)
    for peer,band,who in ((bruno,2,"Clara → Bruno"),(clara,1,"Bruno → Clara")):
        audit=peer.audit
        peer.audit=None
        minimum=min(f[band] for _,f in audit)
        assert minimum>500,(who,len(audit),minimum)
        print(f"{who}: audible in all {len(audit)} consecutive 100 ms windows across key, "
              f"focus, mute and deafen transitions (minimum tone amplitude {minimum:.0f}).",flush=True)
    return keys


def check_restart(alice,keys,joined,left):
    """KWin keeps the bindings under the client's app ID: a restarted client
    arms push-to-talk without another dialog, and the key still holds."""
    from qa_kwin import APP_ID
    kde=alice.kde
    rc=Path(kde.env["XDG_CONFIG_HOME"])/"kglobalshortcutsrc"
    group=wait_for(f"[{APP_ID}] in kglobalshortcutsrc",lambda:rc.is_file() and
                   (m:=re.search(rf"^\[{re.escape(APP_ID)}\]\n(?:[^\[\n].*\n?)*",rc.read_text(),re.M)) and m[0],10)
    assert re.search(r"^push-to-talk=Pause,",group,re.M),group
    alice.restart()
    alice.click("push-to-talk")
    alice.click("join")
    wait_for("rejoined after restart",joined,20)
    keys.focus(keys.target)
    def hint(held):
        def shown():
            assert not kde.shortcut_dialog(),"the restarted client got a second Global Shortcuts Requested dialog"
            words={w for w,_,_ in alice.words()}
            return {"pause","held"}<=words if held else {"hold","pause","talk"}<=words and "held" not in words
        return shown
    wait_for("Hold Pause to talk after restart, without a dialog",hint(False),25)
    keys.key(True)
    wait_for("Pause held after restart",hint(True),10)
    keys.key(False)
    wait_for("Pause released after restart",hint(False),10)
    alice.click("leave")
    wait_for("left after restart",left,20)
    print(f"Restarted Alice's client on the same profile: kglobalshortcutsrc kept [{APP_ID}] "
          "(push-to-talk=Pause); arming push-to-talk bound without a second dialog, and Pause held and released "
          "the mic with the decoy focused.",flush=True)
