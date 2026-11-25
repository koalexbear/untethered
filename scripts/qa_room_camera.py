"""Verify live pixels, GCM and capture lifetime with a private PipeWire camera."""
import json
import re
import subprocess as sp
import threading
import time

from qa_desktop_dm import sfu_participants
from qa_native_paste import wait_for
from qa_window_share import area


def present(kde):
    return kde.command("busctl","--address="+kde.env["DBUS_SESSION_BUS_ADDRESS"],"get-property",
        "org.freedesktop.portal.Desktop","/org/freedesktop/portal/desktop","org.freedesktop.portal.Camera","IsCameraPresent").stdout.strip()=="b true"


def camera_source(stack,kde,palettes,*,name="qa-room-camera",description="TestCamera"):
    frames=[(bytes(a)*160+bytes(b)*160)*180 for a,b in palettes]
    # The producer's own clock drives these frames. Drop stale frames while
    # no reader is connected, including when switching away and back.
    source=sp.Popen(["gst-launch-1.0","-q","fdsrc","do-timestamp=true","!","rawvideoparse","format=rgb","width=320","height=180","framerate=12/1",
        "!","videoconvert","!","video/x-raw,format=I420","!","queue","leaky=downstream","max-size-buffers=1",
        "!","pipewiresink","mode=provide","sync=false","use-bufferpool=false",
        f"stream-properties=props,media.class=Video/Source,media.role=Camera,node.name={name},node.description={description}"],
        env=kde.env,stdin=sp.PIPE,bufsize=0,stdout=sp.DEVNULL)
    def stop():
        source.terminate();source.wait(timeout=5)
    stack.callback(stop)
    current=frames[0]
    def show(index):
        nonlocal current
        current=frames[index]
    def feed():
        try:
            while source.poll() is None:
                source.stdin.write(current);time.sleep(1/12)
        except (OSError,ValueError):pass
    threading.Thread(target=feed,daemon=True).start()
    wait_for("Camera portal recognizes synthetic camera",lambda:present(kde),15)
    return show


def dialog(kde):
    wid=kde.window("Camera|Access Request|Permission|Konsole|xdg-desktop-portal-kde")
    if not wid:return None
    pid=kde.command("kdotool","getwindowpid",wid).stdout.strip()
    owner=kde.command("gdbus","call","--session","--dest","org.freedesktop.DBus","--object-path","/org/freedesktop/DBus",
                      "--method","org.freedesktop.DBus.GetConnectionUnixProcessID","org.freedesktop.impl.portal.desktop.kde").stdout
    return wid if pid.isdigit() and re.search(rf"\b{pid}\b",owner) else None


def start_camera(sender,published):
    sender.click("start")
    grant_camera(sender.kde,published)


def grant_camera(kde,published):
    def publishing():
        if published():return True
        wid=dialog(kde)
        if wid:
            geometry=kde.command("kdotool","getwindowgeometry",wid).stdout
            x,y=map(int,re.search(r"Position: (-?\d+),(-?\d+)",geometry).groups())
            w,h=map(int,re.search(r"Geometry: (\d+)x(\d+)",geometry).groups())
            words=kde.words((x,y,x+w,y+h))
            button=next(((a,b) for s,a,b in reversed(words) if s.strip("|:").lower() in ("ok","grant","allow") and b>y+h/2),None)
            if button:kde.click(*button)
        return False
    wait_for("camera published",publishing,25)


def room_camera(stack,desks,secret,original,publications):
    sender,*peers=desks
    kde=sender.kde
    def graph():return json.loads(kde.command("pw-dump").stdout)
    def video_nodes():
        return {x["id"]:x["info"]["props"] for x in graph() if x.get("type")=="PipeWire:Interface:Node"
                and "Video" in x["info"]["props"].get("media.class","")}
    def video():
        return [(p["identity"],t) for p in sfu_participants(secret) for t in p.get("tracks",[]) if t.get("type")=="VIDEO"]
    assert not present(kde) and not video_nodes(), "test needs a private camera-free PipeWire server"
    for p in peers:p.open_stage()
    sender.click("start")
    wait_for("no-camera feedback",lambda:{"no","camera","available"}.issubset({v for v,_,_ in sender.words()}),10)
    assert not video()
    palettes=[((32,215,129),(216,47,177)),((240,120,26),(39,89,239))]
    show=camera_source(stack,kde,palettes)
    base=video_nodes()
    cameras=[i for i,p in base.items() if p.get("media.class")=="Video/Source" and p.get("media.role")=="Camera"]
    assert len(cameras)==1 and base[cameras[0]]["node.name"]=="qa-room-camera" and not video()
    def capture_links():
        return [x for x in graph() if x.get("type")=="PipeWire:Interface:Link" and x["info"]["output-node-id"]==cameras[0]]
    def reader_pids():
        # Descendants of the actual camera publisher, not other host processes.
        return kde.command("pgrep","-P",str(kde.native.pid),"-x","gst-launch-1.0",check=False).stdout.splitlines()
    sender.click("start")
    wait_for("permission dialog before capture",lambda:dialog(kde),12)
    assert not capture_links() and not video() and not reader_pids()
    sender.click("cancel")
    wait_for("desktop Cancel camera dismisses permission request",lambda:not dialog(kde),8)
    assert not capture_links() and not video() and not reader_pids()
    print("Cancelled camera permission before grant: no capture process, link or publication.",flush=True)
    # KDE can remember closing the prompt as a refusal. Reset only this
    # fixture's disposable permission store to exercise a fresh positive grant.
    store=["busctl","--address="+kde.env["DBUS_SESSION_BUS_ADDRESS"],"--json=short","call",
           "org.freedesktop.impl.portal.PermissionStore","/org/freedesktop/impl/portal/PermissionStore",
           "org.freedesktop.impl.portal.PermissionStore"]
    lookup=kde.command(*store,"Lookup","ss","devices","camera",check=False)
    if lookup.returncode==0:
        for app,permissions in json.loads(lookup.stdout)["data"][0].items():
            if permissions==["no"]:
                kde.command(*store,"DeletePermission","sss","devices","camera",app)
    try:
        start_camera(sender,video)
    except Exception:
        print("Private windows:",kde.command("kdotool","search","--title",".","getwindowname","%@").stdout,flush=True)
        print("Private camera graph:",[(i,p.get("node.name")) for i,p in video_nodes().items()],flush=True)
        raise
    def camera_sid():
        tracks=video()
        assert len(tracks)==1 and tracks[0][1]["source"]=="CAMERA" and tracks[0][1]["encryption"]=="GCM",tracks
        return tracks[0][1]["sid"]
    sid=camera_sid()
    links=capture_links()
    assert links and all(x["info"]["state"]=="active" for x in links) and len(reader_pids())==1
    wait_for("both camera tiles decode the first pattern",lambda:all(area(p,palettes[0]) for p in peers),18)
    print("Camera on: private Camera portal authorized the capture; both peers rendered the green/magenta pattern on one GCM CAMERA track.",flush=True)
    if sender.stats:
        from qa_window_share import video_stats
        def sent():
            out=list(video_stats(sender,"outbound","camera").values())
            return out[0] if len(out)==1 and (out[0]["width"],out[0]["height"])==(1280,720) else None
        try:out=wait_for("sender encodes the camera at 1280x720",sent,30)
        except RuntimeError:
            print("Sender outbound:",video_stats(sender,"outbound"),flush=True);raise
        inbound=[next(iter(video_stats(p,"inbound","camera").values()),None) for p in peers]
        print(f"Camera video: sender encodes {out['width']}x{out['height']} at {out['fps']:.1f} fps from the 12 fps "
              "synthetic source; receivers decode "+" / ".join(f"{i['width']}x{i['height']} at {i['fps']:.1f} fps"
              if i else "n/a" for i in inbound)+".",flush=True)
    show(1)
    desks[0].tone=3;desks[1].tone=4
    fresh=({830,1030},{530,1030},{530,830})
    wait_for("both camera tiles render changed frames",lambda:all(area(p,palettes[1]) for p in peers),18)
    wait_for("six live audio paths while camera is on",lambda:all(d.hears(h) for d,h in zip(desks,fresh)),12)
    assert camera_sid()==sid and publications(3)==original
    print("Camera changed to orange/blue: fresh decoded pixels at both receivers on the same video SID; all six audio paths live.",flush=True)
    sender.click("stop")
    wait_for("camera capture released",lambda:not capture_links() and set(video_nodes())==set(base) and not reader_pids(),12)
    wait_for("camera unpublished",lambda:not video(),12)
    def tile_gone(peer):
        # "Start camera" stays in the call controls; the incoming tile's
        # caption contains "Camera" and the publisher's display name.
        words=peer.words()
        return not any(word=="camera" and any(w=="alice" and 312<xx<x and abs(yy-y)<6 for w,xx,yy in words)
                       for word,x,y in words) and not area(peer,palettes[1])
    wait_for("camera tiles cleared",lambda:all(tile_gone(p) for p in peers),10)
    desks[0].tone=0;desks[1].tone=1
    wait_for("fresh audio after camera off",lambda:all(d.hears(h) for d,h in zip(desks,({670,1030},{410,1030},{410,670}))),12)
    assert publications(3)==original and present(kde)
    print("Camera off: capture links gone, PipeWire video graph at baseline, zero video publications, both tiles cleared. Original three GCM microphones still carry fresh tones.",flush=True)
