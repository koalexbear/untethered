"""Publish one real application's Pulse stream in an already active window share."""
import array
import json
import math
import subprocess as sp
import threading
from qa_native_paste import wait_for
from qa_ui import CHAT_X,PICK_BELOW,RAIL,SIDEBAR

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
    def click(x,y):
        kde.command("kdotool","windowactivate",desk.win)
        kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":300},
                               {"type":"pointerDown","button":0},{"type":"pause","duration":120},{"type":"pointerUp","button":0}])
    def dock(end="top"):
        # The shared window's audio controls head the details of the call
        # dock at the foot of the sidebar, above its Devices panel.
        desk.devices_panel(end)
        return [(s.strip("|:.,").lower(),x,y) for s,x,y in kde.words((RAIL,380,CHAT_X,850))]
    def choose():
        # Use the actual device panel and selection menu; locate rows
        # from the window, since the dock's height varies with call state.
        def picker():
            ws=dock()
            for s,x,y in ws:
                # The pick list is on the line below "Send sound from".
                if s=="send" and any(t=="from" and abs(yy-y)<6 for t,xx,yy in ws):return RAIL+SIDEBAR//2,y+PICK_BELOW
            pt=next(((x,y) for s,x,y in ws if s=="choose" and any(t=="app" and abs(y-yy)<6 for t,xx,yy in ws)),None)
            if pt:click(*pt)
            return None
        x,y=wait_for("application picker",picker,15)
        def option():
            if key in tracks():return True
            # Its menu opens upwards over the dock. An item starts at the
            # list's left edge, unlike "Sharing audio: Prism" in a status line.
            ws=kde.words((RAIL,max(380,y-200),CHAT_X,y-10))
            target=next(((xx,yy) for s,xx,yy in ws if s.lower().rstrip(".,:")=="prism" and xx<RAIL+SIDEBAR//3),None)
            if target:
                click(*target)
                try:return wait_for("selected audio started",lambda:key in tracks(),2)
                except RuntimeError:pass
            else:click(x,y)
            return False
        wait_for("Prism publication from menu",option,18)
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
        ws=dock()
        btn=next(((x,y) for s,x,y in ws if s=="stop" and any(
            s2=="app" and x<x2<x+50 and abs(y2-y)<6 for s2,x2,y2 in ws)),None)
        if btn:click(*btn)
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
    def refresh():
        # Refresh devices is below the fold of the Devices panel. Scroll on
        # every try: a single wheel turn is occasionally lost.
        ws=dock("bottom")
        return max(((x,y) for s,x,y in ws if s=="refresh" and any(t=="devices" and abs(yy-y)<6 for t,_,yy in ws)),
                   key=lambda p:p[1],default=None)
    click(*wait_for("Refresh devices",refresh,12))
    choose()
    wait_for("fresh Prism selected again",lambda:measured(730),15)
    desk.click("devices")
    print("Element decoded Prism 730→830 Hz alongside 410 Hz voice; Decoy 1030 Hz on the same sink absent. "
          "App exit removed only its GCM sound track; manually selected its replacement.",flush=True)
    return lambda:not recorders()
