"""Exercise the actual camera selector and verify routing at the portal and rendered pixels."""
import json
from contextlib import ExitStack
from qa_native_paste import wait_for
from qa_room_camera import camera_source,dialog,grant_camera
from qa_ui import CHAT_X,PICK_BELOW,RAIL,SIDEBAR


def verify_switch(stack,desk,remote_box,local_box,fresh_audio,tracks,baseline,amber,first,local_user):
    kde=desk.kde
    second=[((35,205,48),(178,32,96)),((219,89,55),(75,51,144))]
    removable=stack.enter_context(ExitStack())
    cobalt=camera_source(removable,kde,second,name="qa-second-camera",description="Cobalt")
    def graph():
        data=json.loads(kde.command("pw-dump").stdout)
        names={o["id"]:o["info"]["props"].get("node.name") for o in data if o.get("type")=="PipeWire:Interface:Node"}
        return [names.get(o["info"]["output-node-id"]) for o in data if o.get("type")=="PipeWire:Interface:Link"
                and o["info"]["state"]=="active" and names.get(o["info"]["output-node-id"]) in ("qa-room-camera","qa-second-camera")]
    def words():
        # The Devices panel is in the call dock at the foot of the sidebar.
        return [(w.strip("|:.,").lower(),x,y) for w,x,y in kde.words((RAIL,380,CHAT_X,850))]
    def pos(name):
        return next(((x,y) for w,x,y in words() if w==name),None)
    def click(x,y):
        kde.command("kdotool","windowactivate",desk.win)
        kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":250},
                              {"type":"pointerDown","button":0},{"type":"pause","duration":100},{"type":"pointerUp","button":0}])
    def refresh_cameras(ws):
        return any(w=="refresh" and any(v=="cameras" and x<xx<x+125 and abs(y-yy)<6 for v,xx,yy in ws) for w,x,y in ws)
    def choose(label):
        # The camera pick list is on the line below the "Camera" label, with
        # Refresh cameras under it, at the foot of the scrolling Devices panel.
        def picker():
            desk.devices_panel("bottom")
            ws=words()
            # The lowest "camera": a camera status line above can say it too.
            cam=max(((x,y) for w,x,y in ws if w=="camera"),key=lambda p:p[1],default=None)
            return cam and refresh_cameras(ws) and (RAIL+SIDEBAR//2,cam[1]+PICK_BELOW)
        x,y=wait_for("camera picker",picker,12)
        def entry():
            # Its menu opens upwards over the panel.
            ws=kde.words((RAIL,max(380,y-150),CHAT_X,y-10))
            target=next(((xx,yy) for w,xx,yy in ws if w.strip("|:.,").lower()==label),None)
            if not target and not any(w.strip("|:.,").lower() in ("amber","cobalt") for w,_,_ in ws):
                click(x,y)
            return target
        try:click(*wait_for(label+" in camera list",entry,10))
        except RuntimeError:print("Picker:",words(),flush=True);raise
    desk.click("devices")
    def listed():
        if dialog(kde):return False
        desk.devices_panel("bottom")
        ws=words()
        if refresh_cameras(ws):return True
        button=next(((x,y) for w,x,y in ws if w=="list"),None)
        if button:click(*button)
        return False
    try:grant_camera(kde,listed)
    except Exception:
        print("Camera panel:",words(),"Windows:",kde.command("kdotool","search","--title",".","getwindowname","%@").stdout,flush=True);raise
    for label,source,pattern,change in (("cobalt","qa-second-camera",second,cobalt),("amber","qa-room-camera",first,amber)):
        change(0);choose(label)
        grant_camera(kde,lambda:graph()==[source] and remote_box(pattern[0]))
        wait_for("matching native self-preview",lambda:local_box(pattern[0]),10)
        change(1)
        wait_for("fresh remote picture",lambda:remote_box(pattern[1]),12)
        wait_for("fresh local picture",lambda:local_box(pattern[1]),12)
        assert graph()==[source] and tracks()==baseline
        fresh_audio()
        print(f"{label}: remote Element tile and native self-preview both changed; only {source} captured; "
              "same two camera and two mic publication IDs, audio live.",flush=True)
    choose("cobalt")
    grant_camera(kde,lambda:graph()==["qa-second-camera"] and remote_box(second[1]))
    removable.close()
    def stopped():
        # Camera status lines sit at the head of the Devices panel.
        desk.devices_panel("top")
        return pos("stopped")
    wait_for("missing-camera feedback",lambda:stopped() and (local_user,"CAMERA") not in tracks(),22)
    remaining=tracks()
    missing=set(baseline)-set(remaining)
    assert missing=={(local_user,"CAMERA")} and all(remaining[k]==baseline[k] for k in remaining)
    assert not graph()
    fresh_audio()
    choose("amber")
    assert not graph()
    desk.click("start");grant_camera(kde,lambda:graph()==["qa-room-camera"] and remote_box(first[1]))
    restored=tracks()
    assert set(restored)==set(baseline) and all(restored[k]==v for k,v in remaining.items())
    assert all(enc=="GCM" for _,enc,_ in restored.values())
    fresh_audio()
    print("Withdrew Cobalt: no fallback capture, its video unpublished, both microphones and peer video stayed live. "
          "Selected Amber with camera off, then restarted it on the same call.",flush=True)
    desk.click("devices")
