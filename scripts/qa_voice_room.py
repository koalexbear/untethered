"""Three distinct native clients, each with its own Matrix device, mic and speaker."""
import array
import cmath
import io
import json
import math
import os
import re
import subprocess as sp
import threading
import time
from collections import deque
from contextlib import ExitStack
from pathlib import Path
from urllib.parse import quote

from PIL import Image
from qa_desktop_dm import sfu_participants
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_ui import CHAT_X, RAIL, SIDEBAR, USER_PANEL, Ui, clean, icon_buttons, read

MEMBER = "org.matrix.msc3401.call.member"
TONES = (410, 670, 1030, 530, 830)
PCM = [array.array("h", (round(7000 * math.sin(n * math.tau * hz / 48000))
                        for n in range(4800))).tobytes() for hz in TONES]
SILENCE = len(PCM)
PCM.append(bytes(len(PCM[0])))
# 100 ms, every fourth sample. Bin amplitudes distinguish both superimposed
# tones from one stream, our own echo, comfort noise, and stale queued audio.
BINS = [[cmath.exp(-1j * math.tau * hz * n / 12000) for n in range(1200)] for hz in TONES]


def client_cores(i):
    cores=sorted(os.sched_getaffinity(0))
    return cores[4*i:4*i+4] if len(cores)>=4*i+4 else cores[:4]


class Desktop:
    def __init__(self,stack,root,env,i,kde=None,*,prefix=(),room="Atrium",log=None):
        self.env=env
        self.i=i
        self.kde=kde
        self.tone=i
        self.windows=deque(maxlen=12)
        self.audit=None
        self.stats=None  # UNTETHERED_QA_VIDEO_STATS log, when enabled
        pcm=["--raw","--rate=48000","--channels=1","--format=s16le","--latency-msec=40"]
        def process(args,**kw):
            proc=sp.Popen(args,env=env,stderr=sp.DEVNULL,**kw)
            def stop():
                proc.terminate();proc.wait(timeout=5)
            stack.callback(stop)
            return proc
        source=process(["pacat","--playback",f"--device=Mic{i}",*pcm],stdin=sp.PIPE,bufsize=0)
        speaker=process(["pacat","--record",f"--device=Out{i}.monitor",*pcm],stdout=sp.PIPE)
        def feed():
            try:
                while True:source.stdin.write(PCM[self.tone])
            except (BrokenPipeError,ValueError):pass
        def listen():
            while data:=speaker.stdout.read(9600):
                samples=array.array("h");samples.frombytes(data)
                samples=samples[::4]
                amps=[2*abs(sum(v*c for v,c in zip(samples,b)))/len(samples) for b in BINS]
                self.windows.append(amps)
                if (audit:=self.audit) is not None:audit.append((time.monotonic(),amps))
        for fn in (feed,listen):threading.Thread(target=fn,daemon=True).start()
        if kde:
            self.client=kde.native
            self.win=wait_for("Wayland call window",lambda:kde.window("Untethered.*"+re.escape(room)),35)
            kde.position(self.win,0,0,1180,850)
            return
        # Limit per-client rendering/codec parallelism in this three-GUI fixture.
        # Separate four-core sets, when available, stand in for separate machines.
        cores=client_cores(i)
        binary=os.getenv("UNTETHERED_QA_CLIENT") or root/"rust-client/target/debug/untethered-iced"
        self.client=stack.enter_context(child([*prefix,"taskset","-c",",".join(map(str,cores)),binary],env,stderr=log))
        def window():
            assert self.client.poll() is None,f"client {i+1} exited"
            return self.cmd("xdotool","search","--onlyvisible","--name","Untethered",check=False).strip()
        self.win=wait_for(f"client {i+1} window",window,35).splitlines()[0]
        def synced():
            assert self.client.poll() is None,f"client {i+1} exited"
            if room in self.cmd("xdotool","getwindowname",self.win):return True
            # matrix-sdk shows its stored rooms before the first sync and may
            # open another one first; open this room from the sidebar.
            spot=next(((x,y) for w,x,y in self.words() if w==room.lower() and x<CHAT_X),None)
            if spot:self.press(*spot)
            return False
        wait_for(f"client {i+1} synced {room}",synced,35)
        self.cmd("xdotool","windowmove",self.win,"0","0")
        self.cmd("xdotool","windowsize",self.win,"1180","760")

    def cmd(self,*args,check=True):
        return sp.run(args,env=self.env,text=True,capture_output=True,timeout=8,check=check).stdout

    def image(self):
        """A screenshot in the same coordinates as words() and click()."""
        if self.kde:return self.kde.capture()
        shot=sp.check_output(["import","-window",self.win,"png:-"],env=self.env,timeout=8)
        return Image.open(io.BytesIO(shot)).convert("RGB")

    def words(self):
        if self.kde:
            return [(clean(w),x,y) for w,x,y in self.kde.words((0,28,1180,850))]
        # The voice roster's grey names need the sidebar pass.
        return [(clean(w),x,y) for w,x,y in read(self.image(),sidebar=True)]

    @property
    def ui(self):
        # KWin windows are 1180×850 frames below a 28 px title bar.
        point=lambda pos:self.hover(*map(int,pos))
        if self.kde:return Ui(self.words,point,self.image,width=1180,height=822,origin=(0,28))
        return Ui(self.words,point,self.image,width=1180,height=760)

    def press(self,x,y):
        if self.kde:
            self.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":380},
                {"type":"pointerDown","button":0},{"type":"pause","duration":170},{"type":"pointerUp","button":0}])
        else:self.cmd("xdotool","mousemove","--window",self.win,str(x),str(y),"click","1")

    def shot(self):
        if self.kde:return self.kde.capture().crop((0,0,1180,850))
        return Image.open(io.BytesIO(sp.check_output(["import","-window",self.win,"png:-"],env=self.env,timeout=6))).convert("RGB")

    def hover(self,x,y):
        if self.kde:self.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y}])
        else:self.cmd("xdotool","mousemove","--window",self.win,str(x),str(y))

    def scroll(self,x,y,dy):
        """Turn the mouse wheel over (x, y); a positive dy scrolls down."""
        if self.kde:
            self.kde.actions("wheel",[{"type":"scroll","x":x,"y":y,"deltaX":0,"deltaY":dy}])
            # The client only applies the axis event with the next pointer frame.
            self.kde.actions("pointer",[{"type":"pointerMove","x":x+2,"y":y+2}])
        else:self.cmd("xdotool","mousemove","--window",self.win,str(x),str(y),
                      "click","--repeat",str(max(1,abs(dy)//60)),"5" if dy>0 else "4")

    def devices_panel(self,end):
        """Scroll the call dock's Devices panel to its "top" or "bottom".

        The panel is at most 160 px tall, just above the user panel at the
        foot of the sidebar; the camera section is below its fold."""
        x,y=self.ui.at(RAIL+SIDEBAR//2,self.ui.height-USER_PANEL-60)
        self.scroll(x,y,600 if end=="bottom" else -600)

    def tooltip(self,x,y):
        """Hover a control and OCR only the tooltip that appears above it.

        Status text can sit right behind the tooltip, so crop to the pixels
        that changed between the pointer resting below the row and hovering."""
        from PIL import ImageChops
        area=(max(0,x-150),max(0,y-70),x+150,y-19)
        self.hover(x,y+60);time.sleep(.25)
        before=self.shot().crop(area)
        self.hover(x,y);time.sleep(.4)
        after=self.shot().crop(area)
        box=ImageChops.difference(before,after).convert("L").point(lambda v:255 if v>24 else 0).getbbox()
        if not box:return ""
        im=after.crop(box)
        buf=io.BytesIO();im.resize((im.width*3,im.height*3)).save(buf,"PNG")
        return sp.run(["tesseract","stdin","stdout","--psm","7"],input=buf.getvalue(),capture_output=True).stdout.decode().lower()

    def click(self,label):
        if self.kde:self.kde.command("kdotool","windowactivate",self.win)
        if label in ("voice","video"):
            # DM calls start from the phone and camera icons in the channel header.
            tip="start "+label+" call"
            self.press(*wait_for(tip,lambda:self.ui.header(tip),20))
            return
        icons={"mute":[0],"unmute":[0],"start":[1],"camera":[1],"share":[2],"sharing":[2],
               "stop":[2,1],"cancel":[2,1],"leave":[3],"hang":[3]}
        def locate():
            if label in icons:
                buttons=icon_buttons(self.shot())
                # Try the usual slot first; the tooltip text decides.
                order=[i for i in icons[label] if i<len(buttons)]
                for i in order+[i for i in range(len(buttons)) if i not in order]:
                    if re.search(r"\b"+label+r"\b",self.tooltip(*buttons[i])):return buttons[i]
                return None
            return next(((x,y) for word,x,y in self.words() if word==label),None)
        x,y=wait_for(label,locate,12)
        self.press(x,y)

    def open_stage(self):
        """Calls start docked in the sidebar; expand this client to the stage."""
        def opened():
            words={w for w,_,_ in self.words()}
            if "theater" in words:return True
            if "open" in words:self.click("open")
            return False
        wait_for("call stage",opened,20)

    def hears(self,expected):
        windows=list(self.windows)
        return len(windows)==12 and all(all(a>500 if f in expected else a<200 for a,f in zip(amps,TONES))
                                        for amps in windows)


def check_speaking(desks,names):
    def highlights(peer):
        # A speaking participant's avatar gets a green ring in the voice roster.
        # Skip your own name in the user panel: its Online dot is green too.
        # Above that, take each name's topmost row.
        im=peer.image()
        panel=peer.ui.oy+peer.ui.height-USER_PANEL
        rows={}
        for word,x,y in peer.words():
            for i,name in enumerate(names):
                if word==name.lower() and 70<x<CHAT_X and y<min(panel,rows.get(i,panel)):rows[i]=y
        active={i for i,y in rows.items() if peer.ui.speaking(im,y)}
        return active if len(rows)==len(names) else None
    def verify(active):
        wait_for("decoded sound matches selected inputs",lambda:all(
            d.hears({TONES[j] for j in active if j!=i}) for i,d in enumerate(desks)),12)
        expected=", ".join(names[i] for i in sorted(active)) or "silence"
        wait_for(f"all three speaking rosters: {expected}",lambda:all(highlights(d)==active for d in desks),12)
        print(f"Speaking indicators: {expected}; all 3 rendered rosters matched, with decoded audio checked at each client.",flush=True)
    def stage(active):
        for i,d in enumerate(desks):d.tone=i if i in active else SILENCE
        verify(active)
    stage(set())
    for i in range(len(desks)):
        stage({i})
        stage(set())
    stage({0,2})
    stage({2})
    # Clara's input stays loud during mute. An open input or a recent
    # speaker flag alone must not keep a muted name lit.
    desks[2].click("mute")
    verify(set())
    desks[2].click("unmute")
    verify({2})
    print("Mute cleared Clara on all 3 rosters with her input still active; unmute restored her label.",flush=True)
    stage(set())


def test_group(tmp,hs,name,root,backend,seed,http,secret,*,share=False,camera=False,ptt=False,layout=False,reconnect=False,jwt=None):
    dirs=[tmp/f"p{i}" for i in range(3)]
    for p in dirs:p.mkdir()
    credentials=[seed(p,hs) for p in dirs]
    users=["@"+f["username"]+":"+name for f,_ in credentials]
    auth=credentials[0][1]
    assert len(set(users))==3
    names=("Alice","Bruno","Clara")
    for user,(_,hdr),label in zip(users,credentials,names):
        http("PUT",hs+"/_matrix/client/v3/profile/"+quote(user,safe="")+"/displayname",headers=hdr,json={"displayname":label})
    room=http("POST",hs+"/_matrix/client/v3/createRoom",headers=auth,json={
        "name":"Atrium","preset":"private_chat","invite":users[1:],
        "creation_content":{"type":"org.matrix.msc3417.call"},
        "power_level_content_override":{"events":{MEMBER:0,"org.matrix.msc4143.rtc.slot":0}},
        "initial_state":[{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]
    }).json()["room_id"]
    api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
    for _,hdr in credentials[1:]:http("POST",api+"/join",headers=hdr,json={})

    def member_events():
        return [(e["sender"],e["event_id"],e["content"].get("created_ts")) for e in http("GET",api+"/state",headers=auth).json()
                if e["type"]==MEMBER and e["content"].get("application")=="m.call"]
    def members():
        return sorted(sender for sender,_,_ in member_events())
    def no_ring():
        # Prove no timeline event (including an encrypted wrapper hiding a ring)
        # was sent in this fresh room. Examine all pages, not just the last sync.
        args={"dir":"b","limit":100}
        while True:
            batch=http("GET",api+"/messages",headers=auth,params=args).json()
            unexpected=[e["type"] for e in batch["chunk"] if "state_key" not in e or e["type"] in
                        {"org.matrix.msc4075.rtc.notification","org.matrix.msc4075.call.notify","m.call.invite"}]
            assert not unexpected,unexpected
            if not batch.get("end") or not batch["chunk"]:break
            args["from"]=batch["end"]
    def publications(n):
        peers=sfu_participants(secret)
        if len(peers)!=n:return None
        tracks={}
        for p in peers:
            a=[t for t in p.get("tracks",[]) if t.get("type","AUDIO")=="AUDIO"]
            if len(a)!=1:return None
            assert a[0]["encryption"]=="GCM" and a[0]["source"]=="MICROPHONE"
            tracks[p["identity"].rsplit(":",1)[0]]=(p["sid"],a[0]["sid"])
        return tracks if set(tracks)==set(users[:n]) else None
    desks=[]
    with ExitStack() as stack:
        sock=tmp/"pulse"
        link=logs=None
        if reconnect:
            # Bruno's own link to the Matrix and call services, to cut.
            from urllib.parse import urlsplit
            from qa_rtc_link import client_link
            link=client_link(stack,[urlsplit(u).port for u in (hs,"https://"+name,jwt)]+[18820],"room")
            logs=[Path(os.environ.get("KWIN_QA_DEBUG") or d)/f"native-{i}.log" for i,d in enumerate(dirs)]
        for i,d in enumerate(dirs):
            wayland=((share or camera) and i==0) or (layout and i in (1,2))
            env=isolated_env(d,backend,"wayland" if wayland else "x11");env.update(TOKIO_WORKER_THREADS="2",RAYON_NUM_THREADS="2",GOMAXPROCS="2")
            if not wayland:start_display(stack,d,env,"x11")
            env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE=f"Mic{i}.monitor",PULSE_SINK=f"Out{i}")
            log=None
            if reconnect:
                env["UNTETHERED_QA_RECOVERY"]="1"
                log=stack.enter_context(logs[i].open("w"))
            if share or camera:
                # Debug builds print per-track video stats every heartbeat.
                env["UNTETHERED_QA_VIDEO_STATS"]="1"
                # KWIN_QA_DEBUG keeps the logs after the disposable home is removed.
                logs=Path(os.environ.get("KWIN_QA_DEBUG") or d)/f"native-{i}.log"
                log=stack.enter_context(logs.open("w"))
            if i==0:
                stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
                    f"--load=module-native-protocol-unix socket={sock} auth-anonymous=1",
                    *(f"--load=module-null-sink sink_name={k}{j} channels=1 rate=48000" for j in range(3) for k in ("Mic","Out"))],env))
                wait_for("private audio devices",lambda:sock.is_socket() and
                         "Out2" in sp.run(["pactl","list","short","sinks"],env=env,capture_output=True,text=True).stdout)
            kde=None
            if wayland:
                from qa_kwin import KWin
                kde=KWin(stack,d,root,backend,env=env,screencast=True,native_log=log)
            prefix=link.prefix if reconnect and i==1 else ()
            desks.append(Desktop(stack,root,env,i,kde,prefix=prefix,log=log))
            if log:desks[-1].stats=logs
        try:
            def audio_counts():
                def devices(kind):
                    return {v["name"]:int(v["index"]) for v in json.loads(desks[0].cmd("pactl","-f","json","list",kind))}
                def streams(kind,port):
                    return [int(v[port]) for v in json.loads(desks[0].cmd("pactl","-f","json","list",kind))
                            if v["properties"].get("application.name")=="Untethered"]
                sources,sinks=devices("sources"),devices("sinks")
                inputs,outputs=streams("source-outputs","source"),streams("sink-inputs","sink")
                return [(inputs.count(sources[f"Mic{i}.monitor"]),outputs.count(sinks[f"Out{i}"])) for i in range(3)]
            def sidebar(peer,n):
                words={w for w,x,y in peer.words() if x<CHAT_X and y<500}
                return all((name.lower() in words)==(i<n) for i,name in enumerate(names))
            original={}
            if ptt:
                desks[0].click("push-to-talk")
                for d in desks[1:]:d.audit=[]
            for i,d in enumerate(desks):
                d.click("join")
                wait_for("silent join membership",lambda:members()==sorted(users[:i+1]))
                if i==1:
                    wait_for("pair audio before third joins",lambda:desks[0].hears({670}) and desks[1].hears(set() if ptt else {410}),20)
                    original=wait_for("original pair",lambda:publications(2))
                    assert audio_counts()==[(int(not ptt),1),(1,1),(0,0)]
            tracks=wait_for("three encrypted microphone publications",lambda:publications(3))
            expected=[set(TONES[:3])-{hz} for hz in TONES[:3]]
            if ptt:
                for tones in expected:tones.discard(410)
            wait_for("three-client decoded audio",lambda:all(d.hears(h) for d,h in zip(desks,expected)),25)
            assert {u:tracks[u] for u in original}==original
            # One mixed playback stream per client, however many peers it hears.
            assert audio_counts()==[(int(not ptt),1),(1,1),(1,1)]
            wait_for("participant names under the voice channel",lambda:all(sidebar(d,3) for d in desks),8)
            no_ring()
            for i,heard in enumerate(expected):
                print(f"Client {i+1}: input {TONES[i]} Hz; speaker received {sorted(heard)} Hz, no self tone (12 consecutive 100 ms windows).",flush=True)
            print("Three joined devices, three GCM microphones, one mixed playback stream per client; zero ring or other non-state events.",flush=True)
            if ptt:
                from qa_room_ptt import check_ptt
                check_ptt(stack,desks,audio_counts,lambda:publications(3)==tracks)
            elif layout:
                from qa_room_layout import check_layout
                check_layout(stack,desks,users,secret,tracks,publications)
            elif reconnect:
                from qa_room_reconnect import check_reconnect
                tracks=check_reconnect(desks,users,names,link,logs,member_events,members,publications,audio_counts)
                # Clara's departure below must leave Alice's and Bruno's current tracks alone.
                original={u:tracks[u] for u in users[:2]}
            elif not share and not camera:
                from qa_room_volume import check_volume
                check_volume(desks)
                assert publications(3)==tracks and audio_counts()==[(1,1)]*3
                check_speaking(desks,names)
                assert publications(3)==tracks
            if share:
                from qa_window_share import window_share
                window_share(stack,desks,secret,tracks,publications)
            if camera:
                from qa_room_camera import room_camera
                room_camera(stack,desks,secret,tracks,publications)
            if not layout:desks[2].click("leave")
            wait_for("third device left MatrixRTC",lambda:members()==sorted(users[:2]))
            remaining=wait_for("third device left SFU",lambda:publications(2))
            assert remaining==original
            desks[0].tone=3;desks[1].tone=4
            wait_for("fresh two-party audio after leave",lambda:desks[0].hears({830}) and desks[1].hears({530}) and desks[2].hears(set()),20)
            assert publications(2)==original
            assert audio_counts()==[(1,1),(1,1),(0,0)]
            wait_for("remaining pair in sidebar",lambda:all(sidebar(d,2) for d in desks),8)
            print("Client 3 left. Clients 1 and 2 decoded fresh 830/530 Hz with unchanged participant and GCM microphone SIDs; client 3's speaker was silent.",flush=True)
            for d in desks[:2]:d.click("leave")
            wait_for("empty voice roster",lambda:not members())
            wait_for("all desktop audio streams released",lambda:audio_counts()==[(0,0)]*3)
            no_ring()
            print("Final leave: 0 active memberships, 0 Untethered audio streams, 0 ring events.",flush=True)
        except Exception:
            for d in desks:print("Desktop",d.i+1,"labels:",[w for w,_,_ in d.words()],"PCM amplitudes:",[int(n) for n in d.windows[-1]] if d.windows else [])
            print("Voice roster size:",len(members()),flush=True)
            raise
