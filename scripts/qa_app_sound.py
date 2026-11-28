"""Publish one real application's Pulse stream in an already active window share.

The apps playing are listed in Voice Settings → Screen share audio while the
window share is live, each a control `app-audio-N` in the QA control map
(qa_layout.py), with `app-audio-stop` and `devices-refresh` beside them."""
import array
import json
import math
import subprocess as sp
import threading
from qa_native_paste import wait_for

def app_sound(stack,desk,w,tracks,user,baseline):
    kde=desk.kde
    kde.command("pactl","load-module","module-null-sink","sink_name=AppMix","rate=48000","channels=1")
    def play(name,hz):
        state=[hz]
        pcm={h:array.array("h",(round(6500*math.sin(i*math.tau*h/48000)) for i in range(4800))).tobytes() for h in (730,830,1030)}
        p=sp.Popen(["pacat","--playback","--raw","--format=s16le","--rate=48000","--channels=1","--latency-msec=40",
            "--device=AppMix","--client-name="+name,"--stream-name="+name],env=kde.env,stdin=sp.PIPE)
        def stop():
            p.terminate();p.wait(timeout=4)
        stack.callback(stop)
        def feed():
            try:
                while p.poll() is None:p.stdin.write(pcm[state[0]]);p.stdin.flush()
            except (OSError,ValueError):pass
        threading.Thread(target=feed,daemon=True).start()
        return state,stop
    tone,end=play("Prism",730);play("Decoy",1030)
    sinks=lambda:json.loads(kde.command("pactl","-f","json","list","sink-inputs").stdout)
    def both():
        ins={s["properties"].get("application.name"):s["sink"] for s in sinks()}
        return "Prism" in ins and ins["Prism"]==ins.get("Decoy")
    wait_for("both applications playing through the same speaker",both)
    key=(user,"SCREEN_SHARE_AUDIO")
    assert tracks()==baseline
    def recorders():
        return [r for r in json.loads(kde.command("pactl","-f","json","list","source-outputs").stdout)
                if r["properties"].get("application.name")=="Untethered" and
                   r["properties"].get("media.name")=="Shared application audio"]
    assert not recorders()
    layout=desk.layout
    def choose():
        # Voice Settings reads the apps playing as it opens; Refresh devices
        # reads them again.
        desk.settings()
        def option():
            if key in tracks():return True
            name=layout.item("app-audio-","prism")
            if name and layout.state(name)!="selected":
                desk.press(*desk.reveal(name))
                try:return wait_for("selected audio started",lambda:key in tracks(),2)
                except RuntimeError:pass
            elif not name and layout.get("devices-refresh"):
                desk.press(*desk.reveal("devices-refresh"))
            return False
        try:wait_for("Prism publication from Voice Settings",option,18)
        except RuntimeError:
            print("Apps listed:",[c["text"] for _,c in layout.items("app-audio-")],layout.missing(),flush=True);raise
    choose()
    def measured(hz):
        meters=w.evaluate("window.qaAudioMix()")
        assert not any(m["decoy"] for m in meters), "unselected audio reached the receiver"
        return any(m["run410"]>=10 for m in meters) and any(m["clean"+str(hz)]>=10 for m in meters)
    wait_for("Element hears native mic and selected Prism only",lambda:measured(730),20)
    published=tracks()
    assert published[key][1:]==("GCM",False) and {k:v for k,v in published.items() if k!=key}==baseline
    tone[0]=830
    wait_for("fresh Prism frequency on same publication",lambda:measured(830),15)
    assert tracks()==published and len(recorders())==1
    def stop_audio():
        if tracks()==baseline and not recorders():return True
        desk.settings()
        if layout.get("app-audio-stop"):desk.press(*desk.reveal("app-audio-stop"))
        return False
    wait_for("Stop app audio closes only app sound",stop_audio,15)
    choose()
    wait_for("explicit restart of selection",lambda:measured(830),15)
    end()
    wait_for("app exit unpublishes only its audio",lambda:tracks()==baseline and not recorders(),15)
    assert any(s["properties"].get("application.name")=="Decoy" for s in sinks())
    # Explicitly opt into the replacement. Stopping the window with the
    # second stream running must close its recording and publication too.
    play("Prism",730)
    wait_for("replacement playing alongside decoy",both)
    desk.settings()
    desk.press(*desk.reveal("devices-refresh"))
    choose()
    wait_for("fresh Prism selected again",lambda:measured(730),15)
    desk.settings(False)
    print("Element decoded Prism 730→830 Hz alongside 410 Hz voice; Decoy 1030 Hz on the same sink absent. "
          "App exit removed only its GCM sound track; manually selected its replacement.",flush=True)
    return lambda:not recorders()
