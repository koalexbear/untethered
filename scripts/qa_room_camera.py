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


def check_layers(sender,peers,secret,palettes,show):
    """Simulcast (V2): each tile gets the layer its size needs, hidden video
    stops flowing, and it comes back fresh when shown again.

    Layer and byte checks need the debug build's video stats; the release
    build still checks the SFU's layers and the pixels after reopening."""
    from qa_room_layout import pattern_box
    from qa_ui import CHAT_X
    from qa_window_share import debug_stats,video_stats
    tracks=[t for p in sfu_participants(secret) for t in p.get("tracks",[]) if t.get("type")=="VIDEO"]
    assert len(tracks)==1,tracks
    layers=sorted((l.get("width",0),l.get("height",0)) for l in tracks[0].get("layers",[]))
    assert tracks[0].get("simulcast") and layers==[(320,180),(640,360),(1280,720)],tracks[0]
    print("Simulcast: the SFU lists the GCM camera track with 320x180, 640x360 and 1280x720 layers.",flush=True)
    bruno,clara=peers
    stats=debug_stats(sender) and all([debug_stats(p) for p in peers])
    def inbound(peer):
        got=list(video_stats(peer,"inbound","camera").values())
        return got[0] if len(got)==1 else None
    def receiving(peer,fits):
        got=inbound(peer)
        return got if got and got["fps"]>0 and fits(got["width"]) else None
    def sending():
        return {k.rsplit("/",1)[-1]:v for k,v in video_stats(sender,"outbound","camera").items() if v["fps"]>0}
    def expect(label,check,timeout=20):
        try:return wait_for(label,check,timeout)
        except RuntimeError:
            print("Sender outbound:",video_stats(sender,"outbound","camera"),
                  "Receivers inbound:",[inbound(p) for p in peers],flush=True)
            raise
    def stage_words(peer):
        return {w for w,x,_ in peer.words() if x>CHAT_X}
    if stats:
        expect("both peers' grid tiles take the 640x360 layer",lambda:all(receiving(p,lambda w:w==640) for p in peers))
        # Dynacast: the SFU has the sender stop the layer nobody watches,
        # some seconds after its last viewer left it.
        expect("the sender stops encoding the unwatched 1280x720 layer",lambda:"f" not in sending() and "h" in sending(),40)
        print("Grid of three: both peers receive the camera's 640x360 layer; the sender encodes only "
              +", ".join(f"{rid} {v['width']}x{v['height']}" for rid,v in sorted(sending().items()))+".",flush=True)
    box=wait_for("Alice's camera tile on Bruno's stage",lambda:pattern_box(bruno.image(),palettes[0]),10)
    bruno.press((box[0]+box[2])//2,(box[1]+box[3])//2)
    wait_for("Bruno focuses Alice's camera",lambda:"focus" in stage_words(bruno),10)
    if stats:
        expect("Bruno's focused tile takes the 1280x720 layer while Clara's grid tile keeps 640x360",
               lambda:receiving(bruno,lambda w:w>=1280) and receiving(clara,lambda w:w==640) and "f" in sending())
        print(f"Focus: Bruno receives {inbound(bruno)['width']}x{inbound(bruno)['height']}, Clara "
              f"{inbound(clara)['width']}x{inbound(clara)['height']}; the sender resumed its top layer for Bruno alone.",flush=True)
    bruno.click("grid")
    if stats:
        expect("back in the grid, Bruno's tile takes 640x360 again",lambda:receiving(bruno,lambda w:w==640))
    # Open chat collapses the stage: the dock previews the camera, small.
    bruno.click("chat")
    wait_for("Bruno's stage collapsed to the dock",lambda:"theater" not in {w for w,_,_ in bruno.words()},10)
    if stats:
        expect("Bruno's dock preview takes the 320x180 layer",lambda:receiving(bruno,lambda w:w<=320))
        print("Dock: with the stage collapsed, Bruno's preview receives the 320x180 layer.",flush=True)
    bruno.click("hide")
    hidden=time.monotonic()
    before=None
    if stats:
        # The first sample printed 2 s after hiding, and no bytes after it.
        samples=lambda:len(re.findall(r"^RTC video inbound .*source=camera",bruno.stats.read_text(errors="replace"),re.M))
        time.sleep(max(0,hidden+2-time.monotonic()))
        n=samples()
        wait_for("a stats sample 2 s after hiding",lambda:samples()>n,5)
        before=inbound(bruno)["bytes"]
        time.sleep(3)
        assert inbound(bruno)["bytes"]==before,("bytes after hiding",before,inbound(bruno))
        assert receiving(clara,lambda w:w==640),inbound(clara)
        print(f"Hidden: Bruno's camera bytes stopped within 2 s of hiding the preview ({before} bytes, unchanged "
              "3 s later) while Clara's grid tile kept flowing.",flush=True)
    show(1)
    wait_for("Clara renders the new pattern while Bruno's camera is paused",lambda:area(clara,palettes[1]),18)
    assert before is None or inbound(bruno)["bytes"]==before
    spot=wait_for("Open call view in Bruno's dock",lambda:next(((x,y) for w,x,y in bruno.words() if w=="open" and x<CHAT_X),None),10)
    # Timed from before the click: xdotool may return well after it landed.
    opened=time.monotonic()
    bruno.press(*spot)
    def restored():
        taken=time.monotonic()
        return (taken-opened,time.monotonic()-opened) if area(bruno,palettes[1]) else None
    first,seen=wait_for("Bruno's reopened stage shows the new pattern",restored,10)
    assert first<=1,f"fresh pixels {first:.2f} s after reopening"
    print(f"Reopened: Bruno's stage showed the new pattern in a screenshot started {first:.2f} s after the click "
          f"began (confirmed {seen:.2f} s).",flush=True)


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
    # Ends with the second pattern on both peers, Bruno's after a pause.
    check_layers(sender,peers,secret,palettes,show)
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
    return show,palettes


def camera_preview(desk,secret,show,palettes):
    """The Voice & Video page's camera preview (calls plan U2), after
    everyone left the call: the private camera's pattern shows on the page
    and changes live, through one reader of its own, while LiveKit's Twirp
    ListParticipants stays empty."""
    def near(pixel,colour,slack):
        return max(abs(v-w) for v,w in zip(pixel,colour))<slack
    kde,layout=desk.kde,desk.layout
    samples=[]
    def nobody():
        samples.append(sfu_participants(secret,allow_empty=True))
        return not samples[-1]
    def readers():
        return kde.command("pgrep","-P",str(kde.native.pid),"-x","gst-launch-1.0",check=False).stdout.splitlines()
    assert nobody() and not readers()
    show(0)
    desk.voice_page()
    desk.press(*desk.reveal_page("camera-test"))
    grant_camera(kde,lambda:layout.state("camera-preview")=="live")
    def shows(palette):
        if not nobody():return False
        desk.reveal_page("camera-preview")
        if not (c:=layout.get("camera-preview")):return False
        left,top,right,bottom=map(int,layout.box(c))
        im=desk.image()
        counts=[sum(near(im.getpixel((x,y)),color,32) for y in range(top+2,bottom-2,3) for x in range(left+2,right-2,3))
                for color in palette]
        # Each half of the 320×180 pattern fills about half the preview.
        return all(n>800 for n in counts) and counts
    first=wait_for("the page's preview shows the first pattern",lambda:shows(palettes[0]),15)
    show(1)
    second=wait_for("the page's preview changes to the second pattern",lambda:shows(palettes[1]),12)
    assert len(readers())==1,readers()
    assert all(not s for s in samples),[s for s in samples if s]
    desk.press(*desk.reveal_page("camera-test"))
    wait_for("preview stopped and its reader gone",lambda:layout.state("camera-preview")=="off" and not readers(),10)
    assert nobody()
    desk.voice_page(False)
    print(f"Voice & Video page: Test Video showed the private camera's green/magenta pattern ({first[0]}/{first[1]} "
          f"samples), then orange/blue ({second[0]}/{second[1]}), through one capture process; LiveKit's "
          f"ListParticipants stayed empty in all {len(samples)} samples; Stop Video ended the capture.",flush=True)
