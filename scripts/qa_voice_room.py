"""Three distinct native clients, each with its own Matrix device, mic and speaker."""
import array
import cmath
import io
import json
import math
import os
import subprocess as sp
import threading
import time
from collections import deque
from contextlib import ExitStack
from difflib import SequenceMatcher
from pathlib import Path
from urllib.parse import quote

from PIL import Image
from qa_desktop_dm import sfu_participants
from qa_layout import WORDS, Layout, resolve
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_ui import CHAT_X, USER_PANEL, Ui, clean, read

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
    def __init__(self,stack,root,env,i,kde=None,*,prefix=(),room="Atrium",log=None,pipe=None):
        self.env=env
        self.i=i
        self.kde=kde
        self.room=room
        # A click on a voice channel in the room list opens it and joins its
        # call (14e985b). Leave it unclicked until the fixture joins: it opens
        # by itself as the account's only room (test_group).
        self.voice=room=="Atrium"
        self.tone=i
        self.windows=deque(maxlen=12)
        self.audit=None
        self.stats=None  # UNTETHERED_QA_VIDEO_STATS log, when enabled
        # The call controls' map (qa_layout.py); a KWin window's content
        # starts below its 28 px title bar.
        self.layout=Layout(env["UNTETHERED_QA_LAYOUT"],(0,28) if kde else (0,0))
        # When a list: (monotonic time read, 12 kHz samples) per 100 ms window.
        self.capture=None
        # When the current input signal started feeding (qa_voice_processing).
        self.fed=0.0
        pcm=["--raw","--rate=48000","--channels=1","--format=s16le","--latency-msec=40"]
        def process(args,**kw):
            proc=sp.Popen(args,env=env,stderr=sp.DEVNULL,**kw)
            def stop():
                proc.terminate();proc.wait(timeout=5)
            stack.callback(stop)
            return proc
        source=process(["pacat","--playback",f"--device=Mic{i}",*pcm],stdin=sp.PIPE,bufsize=0)
        if pipe:
            # Less queued input, so a new signal reaches the client sooner.
            import fcntl
            fcntl.fcntl(source.stdin.fileno(),1031,pipe)  # F_SETPIPE_SZ
        speaker=process(["pacat","--record",f"--device=Out{i}.monitor",*pcm],stdout=sp.PIPE)
        def feed():
            # `tone` is an index into PCM or a signal of whole 100 ms pieces,
            # fed 100 ms at a time so a new signal starts within one.
            fed,at=None,0
            try:
                while True:
                    tone=self.tone
                    signal=tone if isinstance(tone,bytes) else PCM[tone]
                    if tone is not fed:fed,at,self.fed=tone,0,time.monotonic()
                    source.stdin.write(signal[at:at+9600])
                    at=(at+9600)%len(signal)
            except (BrokenPipeError,ValueError):pass
        def listen():
            while data:=speaker.stdout.read(9600):
                samples=array.array("h");samples.frombytes(data)
                if (capture:=self.capture) is not None:
                    # A 4-sample average before decimating keeps most of
                    # the band above 6 kHz from folding into the analysis.
                    capture.append((time.monotonic(),[sum(samples[n:n+4])/4 for n in range(0,len(samples),4)]))
                samples=samples[::4]
                amps=[2*abs(sum(v*c for v,c in zip(samples,b)))/len(samples) for b in BINS]
                self.windows.append(amps)
                if (audit:=self.audit) is not None:audit.append((time.monotonic(),amps))
        for fn in (feed,listen):threading.Thread(target=fn,daemon=True).start()
        if kde:
            self.attach(room)
            return
        # Limit per-client rendering/codec parallelism in this three-GUI fixture.
        # Separate four-core sets, when available, stand in for separate machines.
        cores=client_cores(i)
        binary=(os.getenv("UNTETHERED_QA_UI_CLIENT") or os.getenv("UNTETHERED_QA_CLIENT")
                or root/"rust-client/target/debug/untethered-iced")
        self.client=stack.enter_context(child([*prefix,"taskset","-c",",".join(map(str,cores)),binary],env,stderr=log))
        def window():
            assert self.client.poll() is None,f"client {i+1} exited"
            return self.cmd("xdotool","search","--onlyvisible","--name","Untethered",check=False).strip()
        self.win=wait_for(f"client {i+1} window",window,35).splitlines()[0]
        self.show(room,f"client {i+1}")
        self.cmd("xdotool","windowmove",self.win,"0","0")
        self.cmd("xdotool","windowsize",self.win,"1180","760")

    def attach(self,room):
        """Wait for KWin's client window and open `room` in it."""
        kde=self.kde
        self.client=kde.native
        def opened():
            assert self.client.poll() is None,"Wayland client exited"
            return kde.window("^Untethered")
        self.win=wait_for("Wayland call window",opened,35)
        kde.position(self.win,0,0,1180,850)
        self.show(room,"Wayland client")

    def show(self,room,who):
        """Wait until `room` is the open room, after the first sync."""
        def shown():
            assert self.client.poll() is None,who+" exited"
            if room in self.title():return True
            # matrix-sdk shows its stored rooms before the first sync and may
            # open another one first. A voice channel must open by itself, as
            # the account's only room (test_group); open others from the sidebar.
            if self.voice:return False
            spot=next(((x,y) for w,x,y in self.words() if w==room.lower() and x<CHAT_X),None)
            if spot:self.press(*spot)
            return False
        try:wait_for(f"{who} opened {room}",shown,35)
        except RuntimeError:
            if debug:=os.environ.get("KWIN_QA_DEBUG"):self.image().save(Path(debug)/f"open-{self.i}.png")
            print(f"{who} title:",self.title().strip(),"labels:",[w for w,_,_ in self.words()],flush=True)
            raise

    def restart(self,room="Atrium"):
        """KWin desktops: quit the client and start it on the same profile."""
        self.kde.restart_native()
        self.attach(room)

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
        """Scroll the Voice Settings popup to its "top" or "bottom"."""
        x,y=self.layout.wait("voice-settings")
        self.scroll(x,y,600 if end=="bottom" else -600)

    def settings(self,open=True):
        """Open or close Voice Settings from the Voice Connected panel."""
        want="on" if open else "off"
        if self.layout.state("voice-settings-open")!=want:
            self.press(*self.layout.wait("voice-settings-open"))
            wait_for("Voice Settings "+("open" if open else "closed"),
                     lambda:self.layout.state("voice-settings-open")==want,6)

    def close_popups(self,name):
        """A click outside an open popup only closes it: close any that
        `name` isn't in before clicking it. The control that opened one
        closes it anyway."""
        if name in ("voice-settings-open","mute-menu","deafen-menu"):return
        for popup in ("voice-settings","input-menu","output-menu"):
            if self.layout.get(popup) and not self.layout.inside(name,popup):
                self.press(*self.layout.find(name))
                wait_for(popup+" closed",lambda:not self.layout.get(popup),6)

    def reveal(self,name):
        """Scroll Voice Settings until `name` shows whole; its centre."""
        for _ in range(20):
            if self.layout.inside(name,"voice-settings"):return self.layout.find(name)
            c,area=self.layout.get(name),self.layout.get("voice-settings")
            if c and area:
                x,y=self.layout.centre(area)
                self.scroll(x,y,120 if c["y"]>area["y"] else -120)
            time.sleep(.45)
        raise RuntimeError(f"{name} not shown in Voice Settings; {self.layout.missing()}")

    def title(self):
        if self.kde:return self.kde.command("kdotool","getwindowname",self.win,check=False).stdout
        return self.cmd("xdotool","getwindowname",self.win,check=False)

    def click(self,label):
        if self.kde:self.kde.command("kdotool","windowactivate",self.win)
        if label=="join" and self.voice and self.room not in self.title():
            # Join from the room list, which also opens the voice channel.
            self.press(*wait_for(self.room+" in the room list",lambda:next(
                ((x,y) for w,x,y in self.words() if w==self.room.lower() and x<CHAT_X),None),12))
            return
        if label in ("voice","video"):
            # DM calls start from the phone and camera icons in the channel header.
            tip="start "+label+" call"
            self.press(*wait_for(tip,lambda:self.ui.header(tip),20))
            return
        if label=="push-to-talk":
            # Beside Join voice before joining; in Voice Settings during a call.
            if spot:=self.layout.find("ptt-header"):
                self.press(*spot)
                return
            self.settings()
            self.press(*self.reveal("ptt"))
            self.settings(False)
            return
        if label in WORDS or label in ("stop","cancel","sharing"):
            # The call's icon-only controls, through the control map.
            try:name=wait_for(label,lambda:resolve(self.layout,label),12)
            except RuntimeError:
                print(f"Desktop {self.i+1}",self.layout.missing(),flush=True)
                raise
            self.close_popups(name)
            self.press(*self.layout.find(name))
            return
        x,y=wait_for(label,lambda:next(((x,y) for word,x,y in self.words() if word==label),None),12)
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


def evidence(desks,name):
    """Save each client's window as <name>-<n>.png under RTC_QA_SCREENSHOTS, if set."""
    if not (dest:=os.getenv("RTC_QA_SCREENSHOTS")):return
    path=Path(dest);path.mkdir(parents=True,exist_ok=True)
    time.sleep(.5)
    for i,d in enumerate(desks):
        try:d.shot().save(path/f"{name}-{i+1}.png")
        except Exception as e:print(f"Screenshot {name}-{i+1} failed: {e!r}",flush=True)


def muted_from_start(secret,user,seconds=3):
    """Poll the SFU from the join click: `user`'s microphone must be muted
    in every sample from the first one that shows it, for `seconds` after."""
    seen=[];until=None
    deadline=time.monotonic()+40
    while time.monotonic()<deadline and (until is None or time.monotonic()<until):
        for p in sfu_participants(secret,allow_empty=True):
            if p["identity"].rsplit(":",1)[0]!=user:continue
            audio=[t for t in p.get("tracks",[]) if t.get("type","AUDIO")=="AUDIO"]
            if audio:
                seen.append(audio[0].get("muted",False))
                until=until or time.monotonic()+seconds
        time.sleep(.05)
    assert seen,f"{user}'s microphone never appeared at the SFU"
    assert all(seen),f"{user}'s microphone was unmuted at the SFU: {seen}"
    return len(seen)


def highlights(peer,names):
    """Who `peer`'s voice roster shows speaking (indices into `names`), or
    None until every name is read."""
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


def check_speaking(desks,names):
    def verify(active):
        wait_for("decoded sound matches selected inputs",lambda:all(
            d.hears({TONES[j] for j in active if j!=i}) for i,d in enumerate(desks)),12)
        expected=", ".join(names[i] for i in sorted(active)) or "silence"
        wait_for(f"all three speaking rosters: {expected}",lambda:all(highlights(d,names)==active for d in desks),12)
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


def check_theater(desks):
    """Alice opens the stage from her Voice Connected panel's room link and
    turns on Theater: the panel and her Mute and Deafen stay in the corner
    over the stage, and work there."""
    alice=desks[0]
    for i,d in enumerate(desks):d.tone=i  # check_speaking left everyone silent
    alice.press(*alice.layout.wait("call-room"))
    wait_for("call stage",lambda:"theater" in {w for w,_,_ in alice.words()},20)
    alice.click("theater")
    def cornered():
        # The sidebar is gone; the panel and Mute sit in the bottom-left.
        mute,panel=alice.layout.get("mute"),alice.layout.get("call-status")
        words={w for w,x,_ in alice.words() if x<300}
        return mute and panel and mute["x"]<240 and "mentions" not in words and "exit" in {w for w,_,_ in alice.words()}
    wait_for("Theater with the call panel in its corner",cornered,15)
    evidence(desks,"voice-room-theater")
    alice.click("mute")
    wait_for("Alice muted from Theater",lambda:desks[1].hears({1030}) and desks[2].hears({670}),12)
    alice.click("unmute")
    wait_for("Alice unmuted from Theater",lambda:desks[1].hears({410,1030}) and desks[2].hears({410,670}),12)
    alice.click("exit")
    wait_for("Theater closed",lambda:"theater" in {w for w,_,_ in alice.words()},12)
    alice.click("chat")
    wait_for("back to the docked call",lambda:alice.layout.find("call-room") and "theater" not in {w for w,_,_ in alice.words()},12)
    print("Theater: the call-room link opened the stage; in Theater the Voice Connected panel and Mute stayed in the "
          "corner, and Mute/Unmute from there silenced and restored Alice at both peers.",flush=True)


def test_group(tmp,hs,name,root,backend,seed,http,secret,*,share=False,camera=False,ptt=False,layout=False,reconnect=False,processing=False,jwt=None):
    dirs=[tmp/f"p{i}" for i in range(3)]
    for p in dirs:p.mkdir()
    # Atrium is each account's only room, so the client opens it by itself.
    # Push-to-talk is armed in the open voice channel's header, and a click on
    # Atrium in the room list would join its call (14e985b). A seeded
    # "media-proof" room stays in the client's store and opens first.
    credentials=[seed(p,hs,room=False) for p in dirs]
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
            wayland=((share or camera or ptt=="wayland") and i==0) or (layout and i in (1,2))
            env=isolated_env(d,backend,"wayland" if wayland else "x11");env.update(TOKIO_WORKER_THREADS="2",RAYON_NUM_THREADS="2",GOMAXPROCS="2")
            # Plain --room-ptt checks the in-window key on its own.
            if ptt=="window":env["UNTETHERED_GLOBAL_SHORTCUTS"]="0"
            if not wayland:start_display(stack,d,env,"x11")
            env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE=f"Mic{i}.monitor",PULSE_SINK=f"Out{i}")
            log=None
            if reconnect:
                env["UNTETHERED_QA_RECOVERY"]="1"
                log=stack.enter_context(logs[i].open("w"))
            if share or camera:
                # Debug builds print per-track video stats every second.
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
                kde=KWin(stack,d,root,backend,env=env,screencast=ptt!="wayland",native_log=log,
                         global_shortcuts=ptt=="wayland")
            prefix=link.prefix if reconnect and i==1 else ()
            desks.append(Desktop(stack,root,env,i,kde,prefix=prefix,log=log,pipe=16384 if processing and i==0 else None))
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
                # OCR reads the grey roster names loosely ("aiice" for Alice).
                def seen(name):
                    return any(SequenceMatcher(None,name.lower(),w).ratio()>=.8 for w in words)
                return all(seen(name)==(i<n) for i,name in enumerate(names))
            original={}
            if ptt:
                desks[0].click("push-to-talk")
                if ptt=="wayland":
                    # Arming push-to-talk binds the portal shortcuts. The
                    # portal names the app from its registered app ID's
                    # desktop file, not from the launcher's scope.
                    offered=desks[0].kde.accept_shortcuts({"untethered","talk","pause","mute","deafen"})
                    print("KDE's Global Shortcuts Requested dialog named Untethered and offered Push to talk (Pause), "
                          f"Toggle mute and Toggle deafen; accepted ({len(offered)} words read).",flush=True)
                for d in desks[1:]:d.audit=[]
            # Plain --voice-room: Clara mutes in her user panel before she
            # joins (calls plan U1), and unmutes once all three are in.
            join_muted=not (share or camera or ptt or layout or reconnect or processing)
            for i,d in enumerate(desks):
                if i==2 and join_muted:
                    d.click("mute")
                    wait_for("Clara muted before joining",lambda:d.layout.state("mute")=="on",8)
                d.click("join")
                if i==2 and join_muted:samples=muted_from_start(secret,users[2])
                wait_for("silent join membership",lambda:members()==sorted(users[:i+1]))
                if i==1:
                    wait_for("pair audio before third joins",lambda:desks[0].hears({670}) and desks[1].hears(set() if ptt else {410}),20)
                    original=wait_for("original pair",lambda:publications(2))
                    assert audio_counts()==[(int(not ptt),1),(1,1),(0,0)]
                    # The same moment before and after the call controls change.
                    evidence(desks,"voice-room-pair")
            tracks=wait_for("three encrypted microphone publications",lambda:publications(3))
            evidence(desks,"voice-room-joined")
            if join_muted:
                wait_for("Alice and Bruno hear only each other",lambda:desks[0].hears({670}) and desks[1].hears({410})
                         and desks[2].hears({410,670}),20)
                assert audio_counts()==[(1,1),(1,1),(0,1)]
                print(f"Clara joined muted: the SFU reported her microphone muted in all {samples} samples from its "
                      "first appearance; Alice and Bruno heard no 1030 Hz; her client opened no capture stream.",flush=True)
                desks[2].click("unmute")
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
                keys=check_ptt(stack,desks,audio_counts,lambda:publications(3)==tracks,None if ptt=="window" else ptt)
            elif layout:
                from qa_room_layout import check_layout
                check_layout(stack,desks,users,secret,tracks,publications)
            elif processing:
                from qa_voice_processing import check_processing
                check_processing(desks,names,secret,users,dirs)
                assert publications(3)==tracks and audio_counts()==[(1,1)]*3
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
                check_theater(desks)
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
            if ptt=="wayland":
                from qa_room_ptt import check_restart
                check_restart(desks[0],keys,lambda:members()==users[:1],lambda:not members())
        except Exception:
            for d in desks:
                print("Desktop",d.i+1,"labels:",[w for w,_,_ in d.words()],"PCM amplitudes:",[int(n) for n in d.windows[-1]] if d.windows else [])
                print("Desktop",d.i+1,d.layout.missing(),"· call status:",d.layout.text("call-status"),"· notes:",d.layout.text("call-notes"),flush=True)
            print("Voice roster size:",len(members()),flush=True)
            raise
