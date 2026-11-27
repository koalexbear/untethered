"""Verify the portal-selected window on both remote clients' rendered surfaces."""
import io
import json
import os
import re
import subprocess as sp
import time
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


STAT=re.compile(r"^RTC video (inbound|outbound|render) sid=(\S+) (?:source=(\w+) )?(?:rid=\S+ )?"
                r"width=(\d+) height=(\d+) fps=([\d.]+)(.*)$",re.M)
def video_stats(desk,direction,source=None):
    """Latest UNTETHERED_QA_VIDEO_STATS sample per track SID in a client's log."""
    latest={}
    for m in STAT.finditer(desk.stats.read_text(errors="replace")):
        if m[1]==direction and (source is None or m[3]==source):
            extra=dict(kv.split("=",1) for kv in m[7].split() if "=" in kv)
            latest[m[2]]={"width":int(m[4]),"height":int(m[5]),"fps":float(m[6]),**extra}
    return latest

def process_cpu(pids,seconds=8):
    """CPU % (100 = one core) per process and per thread name over one window."""
    tick=os.sysconf("SC_CLK_TCK")
    def threads(pid):
        out={}
        for task in Path(f"/proc/{pid}/task").glob("*"):
            try:raw=(task/"stat").read_text()
            except OSError:continue
            f=raw[raw.rindex(")")+2:].split()
            out[task.name]=(raw[raw.index("(")+1:raw.rindex(")")],int(f[11])+int(f[12]))
        return out
    before={p:threads(p) for p in pids};start=time.monotonic()
    time.sleep(seconds)
    after={p:threads(p) for p in pids};span=time.monotonic()-start
    result={}
    for p in pids:
        names={}
        for tid,(name,used) in after[p].items():
            name=re.sub(r"[-_ ]?\d+$","",name)
            names[name]=names.get(name,0)+(used-before[p].get(tid,(name,0))[1])/tick/span*100
        result[p]=(sum(names.values()),sorted(names.items(),key=lambda kv:-kv[1]))
    return result

def children(pid):
    found=[]
    for stat in Path("/proc").glob("[0-9]*/stat"):
        try:raw=stat.read_text()
        except OSError:continue
        if raw[raw.rindex(")")+2:].split()[1]==str(pid):found.append(int(stat.parent.name))
    return found

def comm(pid):
    try:return Path(f"/proc/{pid}/comm").read_text().strip()
    except OSError:return str(pid)

def cpu_line(label,usage):
    total,names=usage
    top=", ".join(f"{n} {v:.0f}%" for n,v in names[:4] if v>=1)
    return f"{label} {total:.0f}% ({top})"

def debug_stats(desk):
    """The client prints UNTETHERED_QA_VIDEO_STATS lines: a debug build.

    Release builds (UNTETHERED_QA_UI_CLIENT) print none; skip the rate checks."""
    if not desk.stats:return False
    try:return bool(wait_for("video stats in the client log",lambda:STAT.search(desk.stats.read_text(errors="replace")),10))
    except RuntimeError:
        print("No video stats in the client log (release build): rate checks skipped.",flush=True)
        desk.stats=None
        return False

def check_video_rate(sender,peers,others=()):
    """Sender encodes the share at >=1280 px and >=24 fps; each receiver decodes >=24 fps.

    CPU is sampled first so a slow run still reports where the time went.
    `others` adds (label, pid) pairs such as the compositor and source window."""
    def sample():
        out=list(video_stats(sender,"outbound","screenshare").values())
        got=[list(video_stats(p,"inbound","screenshare").values()) for p in peers]
        return (out[0],[g[0] for g in got]) if len(out)==1 and all(len(g)==1 for g in got) else None
    wait_for("video stats from the sender and both receivers",sample,30)
    pids=[sender.client.pid,*children(sender.client.pid),*(p.client.pid for p in peers),*(pid for _,pid in others)]
    usage=process_cpu(pids)
    out,inbound=sample()
    capture=re.findall(r"^RTC video capture .* fps=([\d.]+)$",sender.stats.read_text(errors="replace"),re.M)
    rendered=[list(video_stats(p,"render").values()) for p in peers]
    print(f"Video: sender captures {capture[-1] if capture else '?'} fps and encodes {out['width']}x{out['height']} "
          f"at {out['fps']:.1f} fps ({out.get('encoder','?')}, limit={out.get('limit','?')}); receivers decode "
          +" / ".join(f"{i['width']}x{i['height']} at {i['fps']:.1f} fps" for i in inbound)
          +"; converted for display at "
          +" / ".join(f"{r[0]['fps']:.1f} fps, {r[0]['convert_ms']} ms per frame" if r else "n/a" for r in rendered),flush=True)
    n=len(peers)
    children_of=pids[1:len(pids)-n-len(others)]
    print("CPU over 8 s (100% = one core): "+"; ".join([cpu_line("sender client",usage[pids[0]]),
          *(cpu_line("sender child "+comm(c),usage[c]) for c in children_of if usage[c][0]>=1),
          *(cpu_line(f"receiver {i+1}",usage[p.client.pid]) for i,p in enumerate(peers)),
          *(cpu_line(label,usage[pid]) for label,pid in others)]),flush=True)
    def sent():
        out=list(video_stats(sender,"outbound","screenshare").values())
        return out[0] if len(out)==1 and out[0]["width"]>=1280 and out[0]["fps"]>=24 else None
    def received():
        got=[list(video_stats(p,"inbound","screenshare").values()) for p in peers]
        return [g[0] for g in got] if all(len(g)==1 and g[0]["fps"]>=24 for g in got) else None
    try:
        out=wait_for("sender outbound screen share >=1280 px wide at >=24 fps",sent,30)
        inbound=wait_for("both receivers decode the share at >=24 fps",received,30)
    except RuntimeError:
        print("Sender outbound:",video_stats(sender,"outbound"),
              "Receivers inbound:",[video_stats(p,"inbound") for p in peers],flush=True)
        raise
    print(f"Video rate: sender outbound {out['width']}x{out['height']} at {out['fps']:.1f} fps; receivers inbound "
          +" / ".join(f"{i['fps']:.1f} fps" for i in inbound)+".",flush=True)

def evidence(desks,name):
    """Save each client's screen as <name>-<n>.png under RTC_QA_SCREENSHOTS, if set."""
    if not (dest:=os.getenv("RTC_QA_SCREENSHOTS")):return
    path=Path(dest);path.mkdir(parents=True,exist_ok=True)
    for i,d in enumerate(desks):
        try:d.shot().save(path/f"{name}-{i+1}.png")
        except Exception as e:print(f"Screenshot {name}-{i+1} failed: {e!r}",flush=True)

def own_share(desks,sender):
    """The sharer's own stage shows clean, upright fields at the capture rate.

    Prism's width isn't a multiple of 8, so GStreamer pads its I420 rows; a
    reader that assumes packed rows shears the picture and drifts through the
    planes (colour errors and tearing) on the stage and at every receiver."""
    from qa_room_layout import pattern_box
    kde=sender.kde
    sender.open_stage()
    def stage():
        box=pattern_box(kde.capture().crop((282,60,1180,700)),FIRST)
        return box if box and box[2]-box[0]>700 else None
    try:own=wait_for("sharer sees their own window upright on stage",stage,20)
    except RuntimeError:
        evidence(desks,"own-stage-failed");raise
    evidence(desks,"own-stage")
    if not debug_stats(sender):
        print(f"Sharer's own stage shows Prism {own[2]-own[0]} px wide.",flush=True);return
    def smooth():
        got=video_stats(sender,"render").get("local-window")
        return got if got and got["fps"]>=24 else None
    try:rate=wait_for("sharer's own preview redrawn at >=24 fps",smooth,20)
    except RuntimeError:
        print("Sharer's own preview:",video_stats(sender,"render"),flush=True);raise
    print(f"Sharer's own stage shows Prism {own[2]-own[0]} px wide; its preview is converted at "
          f"{rate['fps']:.1f} fps from {rate['width']}x{rate['height']}, {rate['convert_ms']} ms per frame.",flush=True)

def steady(desks,n=10):
    """Every capture of the sharer's stage and both receivers' tiles shows the share.

    iced 0.14 uploads images of 2 MiB or more on a worker thread and draws
    nothing until the upload is done. A tile handed a new frame that way is
    presented empty once per video frame: it flickers."""
    from qa_room_layout import pattern_box
    sender,*peers=desks
    def own():
        box=pattern_box(sender.kde.capture().crop((282,60,1180,700)),FIRST)
        return bool(box and box[2]-box[0]>700)
    seen={}
    for i,(label,check) in enumerate([("sender's own stage",own),*((f"receiver {j+1}",lambda p=p:area(p,FIRST)) for j,p in enumerate(peers))]):
        hits=0
        for k in range(n):
            if check():hits+=1
            elif k==0 or hits==k:evidence([desks[i]],f"blank-{label.split()[0]}{i+1}-{k}")
        seen[label]=hits
    print("Steady video: "+", ".join(f"{label} {hits}/{n}" for label,hits in seen.items())+" captures showed Prism.",flush=True)
    assert all(hits==n for hits in seen.values()),seen

def pattern_window(stack,sender):
    kde=sender.kde
    source=sp.Popen(["/usr/bin/python",str(Path(__file__).with_name("qa_window_pattern.py"))],env=kde.env,stdin=sp.PIPE,stdout=sp.DEVNULL)
    def stop():source.terminate();source.wait(timeout=5)
    stack.callback(stop)
    for title,x in [("Prism",1190),("Decoy",2800)]:
        wid=wait_for(title,lambda:kde.window("^"+title+"$"))
        kde.command("kdotool","windowmove",wid,str(x),"0")
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
        # The picker lists screens ("Share “Virtual-0”") above the windows, and
        # thumbnails load after it opens. Click the Prism card until the dialog
        # closes. Older pickers select first and need the bottom Share button;
        # never click a screen card's "Share" title.
        def chosen():
            if picker()!=wid:return True
            geom=kde.command("kdotool","getwindowgeometry",wid).stdout
            x,y=map(int,re.search(r"Position: (-?\d+),(-?\d+)",geom).groups())
            w,h=map(int,re.search(r"Geometry: (\d+)x(\d+)",geom).groups())
            words=kde.words((x,y+24,x+w,y+h))
            if hit:=next(((a,b) for t,a,b in words if t.strip("|:").lower()=="prism"),None):
                kde.actions("pointer",[{"type":"pointerMove","x":hit[0],"y":hit[1]},{"type":"pause","duration":300},
                    {"type":"pointerDown","button":0},{"type":"pause","duration":120},{"type":"pointerUp","button":0}])
                time.sleep(1.5)
            if picker()==wid and (button:=next(((a,b) for t,a,b in kde.words((x,y+h-70,x+w,y+h))
                                                if t.strip("|:").lower()=="share"),None)):
                kde.click(*button);time.sleep(1)
            return picker()!=wid
        wait_for("Prism chosen in the KDE picker",chosen,30)
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
    for p in peers:p.open_stage()
    if sender.stats:
        idle=process_cpu([p.client.pid for p in peers],4)
        print("Receiver CPU on the stage before any video (100% = one core): "
              +"; ".join(cpu_line(f"receiver {i+1}",idle[p.client.pid]) for i,p in enumerate(peers)),flush=True)
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
    try:wait_for("both receivers render Prism",lambda:all(area(p,FIRST) for p in peers),20)
    except RuntimeError:
        if sender.stats:print("Video stats:",[(video_stats(d,"outbound"),video_stats(d,"inbound"),video_stats(d,"render")) for d in desks],flush=True)
        try:sender.open_stage();time.sleep(2)
        finally:evidence(desks,"receivers-failed")
        raise
    print("Both peers rendered the selected pink/teal Prism window on one GCM screen track.",flush=True)
    own_share(desks,sender)
    steady(desks)
    if sender.stats:check_video_rate(sender,peers,[("KWin",kde.kw.pid),("pattern window",source.pid)])
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

def dm_share(stack,sender,user,tracks,readers,picture,audio,share_sound=None):
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
            if label=="remote" and share_sound:
                from qa_element_video import BROWSER
                # The filmstrip sits under the stage, down to the window's foot.
                im=kde.capture().crop((282,640,1180,848))
                for pair in BROWSER:
                    if hit:=pattern_box(im,pair):return 282+(hit[0]+hit[2])//2,640+hit[1]+42
            if label=="your":
                im=kde.capture()
                for y in range(570,840,8):
                    for x in range(320,1140,4):
                        if all(max(abs(v-w) for v,w in zip(im.getpixel((x+offset,y)),color))<25
                               for offset,color in [(-24,SECOND[0]),(24,SECOND[1])]):
                            return x,y+20
            return next(((x,y) for w,x,y in kde.words((282,540,1180,848))
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
        return big and big[2]-big[0]>r[2]-r[0]+40
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
    audio_closed=share_sound(live) if share_sound else None
    sender.click("sharing")
    wait_for("window stopped but cameras and microphones remain",lambda:tracks()==baseline)
    wait_for("window grant and capture closed",lambda:sessions(kde)==leases and pw_nodes(kde)==nodes and set(readers())==procs)
    if audio_closed:wait_for("window stop also released selected app recorder",audio_closed)
    wait_for("no stale screen tile",lambda:not picture(SECOND))
    audio()
    print(f"DM window: remote rendered Prism at {a[2]-a[0]} then {b[2]-b[0]} px wide. "
          f"Sender refocused Your screen at {r[2]-r[0]} px and enlarged it in theater. "
          "Cancel/stop released the grant; both cameras and both mics kept their GCM SIDs.",flush=True)
