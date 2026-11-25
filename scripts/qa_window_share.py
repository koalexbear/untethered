"""Verify the portal-selected window on both remote clients' rendered surfaces."""
import io
import json
import os
import re
import subprocess as sp
from pathlib import Path

from PIL import Image
from qa_desktop_dm import sfu_participants
from qa_native_paste import wait_for


def area(peer,palette):
    raw=sp.check_output(["import","-window",peer.win,"png:-"],env=peer.env,timeout=6)
    im=Image.open(io.BytesIO(raw)).convert("RGB")
    # Two adjacent unique fields at rendered video-tile scale, not an icon.
    boxes=[]
    for color in palette:
        pts=[(x,y) for y in range(90,630,5) for x in range(290,1175,5)
             if max(abs(v-w) for v,w in zip(im.getpixel((x,y)),color))<32]
        if len(pts)<800:return False
        boxes.append((min(x for x,_ in pts),max(x for x,_ in pts),min(y for _,y in pts),max(y for _,y in pts)))
    a,b=boxes
    return 0<b[0]-a[1]<16 and abs(a[2]-b[2])<10 and abs(a[3]-b[3])<10


def pattern_window(stack,sender):
    kde=sender.kde
    source=sp.Popen(["/usr/bin/python",str(Path(__file__).with_name("qa_window_pattern.py"))],env=kde.env,stdin=sp.PIPE,stdout=sp.DEVNULL)
    def stop():source.terminate();source.wait(timeout=5)
    stack.callback(stop)
    for title,y in [("Prism",0),("Decoy",340)]:
        wid=wait_for(title,lambda:kde.window("^"+title+"$"))
        kde.command("kdotool","windowmove",wid,"1300",str(y))
    kde.command("kdotool","windowactivate",sender.win)
    return source

def pw_nodes(kde):
    graph=json.loads(kde.command("pw-dump").stdout)
    return {x["id"]: x["info"]["props"].get("node.name","") for x in graph
            if x.get("type")=="PipeWire:Interface:Node" and "Video" in x["info"]["props"].get("media.class","")}
def sessions(kde):
    tree=kde.command("busctl","--address="+kde.env["DBUS_SESSION_BUS_ADDRESS"],"--list",
                     "tree","org.freedesktop.portal.Desktop").stdout
    return {p.strip() for p in tree.splitlines() if re.fullmatch(r"/org/freedesktop/portal/desktop/session/[^/]+/[^/]+",p.strip())}
def dialog_click(kde,wid,label):
    geom=kde.command("kdotool","getwindowgeometry",wid).stdout
    x,y=map(int,re.search(r"Position: (-?\d+),(-?\d+)",geom).groups())
    w,h=map(int,re.search(r"Geometry: (\d+)x(\d+)",geom).groups())
    words=kde.words((x,y+24,x+w,y+h))
    found=next(((x,y) for t,x,y in words if t.strip("|:").lower()==label),None)
    if not found:raise AssertionError(f"Picker label {label!r} not in {words}")
    kde.click(*found)
FIRST=[(235,36,119),(14,204,153)]
SECOND=[(24,79,234),(245,132,33)]

def choose_window(sender,*,cancel=False):
    kde=sender.kde
    def picker():return kde.window("^Screen Sharing Requested")
    def request():
        if wid:=picker():return wid
        sender.click("share")
        try:return wait_for("portal opening",picker,3)
        except RuntimeError:return None
    wid=wait_for("KDE window picker",request,18)
    kde.command("kdotool","windowactivate",wid)
    if not cancel:
        dialog_click(kde,wid,"prism")
        if picker()==wid:dialog_click(kde,wid,"share")
        return
    kde.keys(("Down","\ue00c"),("Up","\ue00c"))  # Escape in the private chooser.
    wait_for("cancelled picker closed",lambda:not picker())
    wait_for("desktop acknowledged cancellation",lambda:any(w in ("cancelled","denied") for w,_,_ in sender.words()))

def window_share(stack,desks,secret,original,publications):
    sender,*peers=desks
    kde=sender.kde
    source=pattern_window(stack,sender)
    def video():
        return [(p["identity"],t) for p in sfu_participants(secret) for t in p.get("tracks",[]) if t.get("type")=="VIDEO"]
    before=pw_nodes(kde);before_sessions=sessions(kde)
    choose_window(sender,cancel=True)
    assert not video()
    wait_for("cancelled capture released",lambda:pw_nodes(kde)==before and sessions(kde)==before_sessions)
    print("Cancelled chooser: no screen publication, portal session or video nodes left.",flush=True)
    choose_window(sender)
    share=wait_for("GCM screen publication",video,25)
    assert len(share)==1 and share[0][1]["source"]=="SCREEN_SHARE" and share[0][1]["encryption"]=="GCM",share
    during=pw_nodes(kde);capture_nodes=set(during)-set(before)
    assert capture_nodes, "expected a new capture source or PipeWire reader"
    active=sessions(kde)-before_sessions
    assert len(active)==1,active
    wait_for("both receivers render Prism",lambda:all(area(p,FIRST) for p in peers),20)
    print("Both peers rendered the selected pink/teal Prism window on one GCM screen track.",flush=True)
    sender.click("media-proof")
    wait_for("sharer is browsing another room",lambda:kde.window("Untethered.*media-proof"))
    desks[0].tone=3;desks[1].tone=4
    fresh=[{830,1030},{530,1030},{530,830}]
    source.stdin.write(b"change\n");source.stdin.flush()
    wait_for("both receivers render new Prism pixels after navigation",lambda:all(area(p,SECOND) for p in peers),18)
    wait_for("new audio tones while sharing from another channel",lambda:all(d.hears(h) for d,h in zip(desks,fresh)))
    current=video()
    assert publications(3)==original and len(current)==1 and current[0][1]["sid"]==share[0][1]["sid"] and current[0][1]["encryption"]=="GCM"
    print("Sharer browsed media-proof; both receivers rendered Prism's new blue/orange pattern. Speakers decoded [830,1030], [530,1030], [530,830] Hz on the original audio tracks.",flush=True)
    sender.click("stop")
    wait_for("screen track unpublished",lambda:not video(),12)
    wait_for("private video graph returned to baseline",lambda:pw_nodes(kde)==before,12)
    wait_for("portal session removed",lambda:sessions(kde)==before_sessions,10)
    wait_for("remote tiles cleared",lambda:all(not area(p,SECOND) and
             not any(w=="shared" for w,_,_ in p.words()) for p in peers),10)
    assert all(d.hears(h) for d,h in zip(desks,fresh)) and publications(3)==original
    desks[0].tone=0;desks[1].tone=1
    print(f"Stop sharing: portal session gone, {len(capture_nodes)} PipeWire video node(s) removed, 0 screen tracks and both remote tiles cleared; original microphones still live.",flush=True)

def dm_share(stack,sender,user,tracks,readers,picture,audio):
    """The two cameras and two microphones are already live in this DM."""
    kde=sender.kde
    source=pattern_window(stack,sender)
    baseline=tracks();key=(user,"SCREEN_SHARE")
    procs=set(readers());nodes=pw_nodes(kde);leases=sessions(kde)
    assert len(procs)==1 and len(baseline)==4
    choose_window(sender,cancel=True)
    wait_for("cancel released screen grant only",lambda:pw_nodes(kde)==nodes and sessions(kde)==leases)
    assert tracks()==baseline and set(readers())==procs
    choose_window(sender)
    wait_for("new screen publication",lambda:key in tracks())
    live=tracks()
    assert len(live)==5 and live[key][1]=="GCM" and all(live[k]==v for k,v in baseline.items())
    assert len(sessions(kde)-leases)==1 and len(set(readers())-procs)==1
    a=wait_for("receiver rendered chosen window",lambda:picture(FIRST),20)
    from qa_room_layout import pattern_box
    def own_stage(pair):
        return pattern_box(kde.capture().crop((288,270,1178,430)),pair)
    first=wait_for("sharer sees their own window on stage",lambda:own_stage(FIRST))
    def thumb(label):
        def target():
            # Filmstrip labels are separate from the stage's title/caption.
            if label=="your":
                im=kde.capture()
                for y in range(570,775,8):
                    for x in range(320,1140,4):
                        if all(max(abs(v-w) for v,w in zip(im.getpixel((x+offset,y)),color))<25
                               for offset,color in [(-24,SECOND[0]),(24,SECOND[1])]):
                            return x,y+20
            return next(((x,y) for w,x,y in kde.words((282,540,1180,796))
                         if w.lower().strip("|:.,")==label or label=="remote" and w.lower().startswith("media")),None)
        x,y=wait_for("filmstrip "+label,target,12)
        kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y-20},{"type":"pause","duration":380},
                    {"type":"pointerDown","button":0},{"type":"pause","duration":160},{"type":"pointerUp","button":0}])
    thumb("remote")
    wait_for("own share demoted to thumbnail",lambda:not own_stage(FIRST))
    source.stdin.write(b"change\n");source.stdin.flush()
    b=wait_for("receiver rendered changed window",lambda:picture(SECOND),18)
    thumb("your");r=wait_for("own share reselected with new frames",lambda:own_stage(SECOND))
    assert first[2]-first[0]>700 and r[2]-r[0]>700
    sender.click("theater")
    def wide():
        big=pattern_box(kde.capture().crop((0,275,1179,435)),SECOND)
        return big and big[2]-big[0]>r[2]-r[0]+80
    wait_for("theater shows larger own share",wide)
    if dest:=os.getenv("RTC_QA_SCREENSHOTS"):
        source.stdin.write(b"board\n");source.stdin.flush()
        wait_for("readable shared board",lambda:any(w.lower()=="playtest" for w,_,_ in kde.words((0,90,1179,570))))
        path=Path(dest);path.mkdir(parents=True,exist_ok=True)
        kde.capture().crop((0,0,1180,850)).save(path/"own-share-theater.png")
    def sidebar_back():
        if kde.capture().getpixel((80,300))==(43,45,49):return True
        sender.click("exit")
        return False
    wait_for("exited theater",sidebar_back)
    if dest:
        kde.capture().crop((0,0,1180,850)).save(path/"own-share-stage.png")
        source.stdin.write(b"change\n");source.stdin.flush()
        wait_for("share still visible",lambda:picture(SECOND))
    audio()
    assert tracks()==live
    sender.click("sharing")
    wait_for("window stopped but cameras and microphones remain",lambda:tracks()==baseline)
    wait_for("window grant and capture closed",lambda:sessions(kde)==leases and pw_nodes(kde)==nodes and set(readers())==procs)
    wait_for("no stale screen tile",lambda:not picture(SECOND))
    audio()
    print(f"DM window: remote rendered Prism at {a[2]-a[0]} then {b[2]-b[0]} px wide. "
          f"Sender refocused Your screen at {r[2]-r[0]} px and enlarged it in theater. "
          "Cancel/stop released the grant; both cameras and both mics kept their GCM SIDs.",flush=True)
