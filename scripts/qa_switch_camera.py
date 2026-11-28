"""Exercise the actual camera selector and verify routing at the portal and rendered pixels.

The cameras are listed in Voice Settings, opened from the Voice Connected
panel's gear; each is a control `camera-item-N` in the QA control map
(qa_layout.py). The popup sits over the stage, so it's closed before any
pixel check."""
import json
from contextlib import ExitStack
from qa_native_paste import wait_for
from qa_room_camera import camera_source,dialog,grant_camera


def verify_switch(stack,desk,remote_box,local_box,fresh_audio,tracks,baseline,amber,first,local_user):
    kde=desk.kde
    layout=desk.layout
    second=[((35,205,48),(178,32,96)),((219,89,55),(75,51,144))]
    removable=stack.enter_context(ExitStack())
    cobalt=camera_source(removable,kde,second,name="qa-second-camera",description="Cobalt")
    def graph():
        data=json.loads(kde.command("pw-dump").stdout)
        names={o["id"]:o["info"]["props"].get("node.name") for o in data if o.get("type")=="PipeWire:Interface:Node"}
        return [names.get(o["info"]["output-node-id"]) for o in data if o.get("type")=="PipeWire:Interface:Link"
                and o["info"]["state"]=="active" and names.get(o["info"]["output-node-id"]) in ("qa-room-camera","qa-second-camera")]
    def cameras():
        return {n:(c["text"],c["state"]) for n,c in layout.items("camera-item-")}
    def choose(label):
        desk.settings()
        name=wait_for(label+" in the camera list",lambda:layout.item("camera-item-",label),10)
        if layout.state(name)!="selected":
            desk.press(*desk.reveal(name))
            wait_for(label+" chosen",lambda:layout.state(name)=="selected",8)
        # Voice Settings covers part of the stage: close it before looking.
        desk.settings(False)
    def listed():
        if dialog(kde):return False
        desk.settings()
        refresh=layout.text("cameras-refresh").lower()
        if "refresh" in refresh and cameras():return True
        if "list" in refresh:desk.press(*desk.reveal("cameras-refresh"))
        return False
    try:grant_camera(kde,listed)
    except Exception:
        print("Cameras:",cameras(),layout.missing(),"Windows:",kde.command("kdotool","search","--title",".","getwindowname","%@").stdout,flush=True);raise
    print("Voice Settings listed the cameras:",", ".join(t for t,_ in cameras().values()),flush=True)
    desk.settings(False)
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
        # Camera status lines are under the Voice Connected panel's controls.
        return "stopped" in layout.text("call-notes").lower()
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
