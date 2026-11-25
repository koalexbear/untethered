"""Exercise the actual camera selector and verify routing at the portal and rendered pixels."""
import json
from contextlib import ExitStack
from qa_native_paste import wait_for
from qa_room_camera import camera_source,dialog,grant_camera


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
        return [(w.strip("|:.,").lower(),x,y) for w,x,y in kde.words((281,380,1179,850))]
    def pos(name):
        return next(((x,y) for w,x,y in words() if w==name),None)
    def click(x,y):
        kde.command("kdotool","windowactivate",desk.win)
        kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":250},
                              {"type":"pointerDown","button":0},{"type":"pause","duration":100},{"type":"pointerUp","button":0}])
    def choose(label):
        # Align the picker click with the Refresh cameras button on its row.
        def picker():
            ws=words()
            return next(((670,y) for w,x,y in ws if w=="refresh" and any(
                w2=="cameras" and x<x2<x+125 and abs(y2-y)<6 for w2,x2,y2 in ws)),None)
        x,y=wait_for("camera picker",picker,12)
        def entry():
            ws=kde.words((388,max(360,y-110),680,y-12)) + (kde.words((388,y+12,680,min(y+124,850))) if y+12<850 else [])
            target=next(((xx,yy) for w,xx,yy in ws if w.strip("|:.,").lower()==label),None)
            if not target and not any(w.lower() in ("amber","cobalt") for w,_,_ in ws):
                click(x,y)
            return target
        try:click(*wait_for(label+" in camera list",entry,10))
        except RuntimeError:print("Picker:",words(),flush=True);raise
    desk.click("devices")
    def listed():
        if dialog(kde):return False
        ws=words()
        if any(w=="refresh" and any(v=="cameras" and x<xx<x+125 and abs(y-yy)<6 for v,xx,yy in ws) for w,x,y in ws):return True
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
    wait_for("missing-camera feedback",lambda:pos("stopped") and (local_user,"CAMERA") not in tracks(),22)
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
