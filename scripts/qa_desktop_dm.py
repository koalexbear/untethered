"""Click the desktop and Element call controls; measure PCM at the private audio server."""
import array
import base64
import hashlib
import hmac
import io
import json
import math
import os
import re
import subprocess as sp
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

from PIL import Image
import requests
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_ui import CHAT_X, PICK_BELOW, RAIL, SIDEBAR, Ui, clean, icon_buttons, read

# Since 76d6e43 a call's controls, status and Devices panel are in a dock at the
# foot of the sidebar. These controls are icons, named only by their tooltips:
DOCK_ICONS=("mute","unmute","hang")
# and these words are dock text:
DOCK_TEXT=("deafen","undeafen","devices","refresh","reconnecting","connected")


def sfu_participants(secret,*,allow_empty=False):
    def query(method,params,grant):
        def b64(v):
            return base64.urlsafe_b64encode(v).rstrip(b"=")
        claims={"iss":"testkey","exp":int(time.time())+30,"video":grant}
        body=b64(b'{"alg":"HS256","typ":"JWT"}')+b"."+b64(json.dumps(claims).encode())
        token=body+b"."+b64(hmac.digest(secret.encode(),body,hashlib.sha256))
        r=requests.post("http://127.0.0.1:18820/twirp/livekit.RoomService/"+method,
                        headers={"Authorization":"Bearer "+token.decode()},json=params,timeout=5)
        r.raise_for_status()
        return r.json()
    rooms=query("ListRooms",{},{"roomList":True}).get("rooms",[])
    if not rooms and allow_empty:return []
    assert len(rooms)==1
    name=rooms[0]["name"]
    peers=query("ListParticipants",{"room":name},{"roomAdmin":True,"room":name})["participants"]
    return peers


def sfu_publications(secret):
    return [(p["identity"],p["sid"],t) for p in sfu_participants(secret) for t in p.get("tracks",[]) if t.get("type","AUDIO")=="AUDIO"]


def sfu_audio(secret):
    result={}
    for who,_,track in sfu_publications(secret):
        result.setdefault(who,[]).append(track.get("encryption","NONE"))
    return result


def desktop_call(stack,tmp,page,room,root,backend,alice,bob,secret,*,incoming=False,controls=False,devices=False,network_ports=None,outage_seconds=None,relay_links=None,sibling=None,set_dnd=None):
    env=isolated_env(tmp,backend,"x11")
    start_display(stack,tmp,env,"x11",1280,900)
    sock=Path(env["XDG_RUNTIME_DIR"])/"qa-pulse"
    env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE="QAMic.monitor",PULSE_SINK="QASpeaker")
    def cmd(*args,**kw):
        return sp.run(args,env=env,capture_output=True,timeout=8,**kw)
    ports=[("QAMic","Amber"),("QASpeaker","Harbor")]
    if devices:ports.extend([("QAMicB","Cobalt"),("QASpeakerB","Grove")])
    pulse=stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
        "--load=module-native-protocol-unix socket="+str(sock)+" auth-anonymous=1",
        *(f"--load=module-null-sink sink_name={name} sink_properties=device.description={label} rate=48000 channels=1" for name,label in ports)],env))
    def ready():
        assert pulse.poll() is None,"private audio server exited"
        return sock.is_socket() and "QASpeaker" in cmd("pactl","list","short","sinks",text=True).stdout
    wait_for("private PulseAudio sinks",ready)
    if devices:
        # A removable source backed by mic B, independently of its tone generator.
        spare=cmd("pactl","load-module","module-remap-source","master=QAMicB.monitor","source_name=QASpare",
                  "source_properties=device.description=Spare",text=True,check=True).stdout.strip()
        # PulseAudio prefers a non-monitor source by default; removing Spare
        # must not itself move the fixture's default during the assertion.
        cmd("pactl","set-default-source","QAMic.monitor",check=True)
        cmd("pactl","set-default-sink","QASpeaker",check=True)
    # Play only into the null mic sink; monitor only the separate speaker sink.
    args=["--raw","--rate=48000","--channels=1","--format=s16le","--latency-msec=40"]
    new_tone=threading.Event()
    def inject(feed,hz):
        def pcm(freq):return array.array("h",(round(9000*math.sin(i*math.tau*freq/48000)) for i in range(4800))).tobytes()
        tone=pcm(hz)
        changed=pcm(730) if network_ports or relay_links else tone
        try:
            while True:feed.stdin.write(changed if new_tone.is_set() else tone)
        except (OSError,ValueError):pass
    received=threading.Event()
    chime=threading.Event()
    speakers=[(0,0,0),(0,0,0)]  # completed 100ms windows, consecutive 960 Hz windows, consecutive silent windows
    def observe(monitor,idx):
        good=quiet=0
        while data:=monitor.stdout.read(9600):
            pcm=array.array("h");pcm.frombytes(data)
            hz=sum(a<=0<b for a,b in zip(pcm,pcm[1:]))*48000/len(pcm)
            good=good+1 if abs(hz-960)<35 and max(pcm)>500 else 0
            quiet=quiet+1 if max(abs(min(pcm)),abs(max(pcm)))<32 else 0
            speakers[idx]=(speakers[idx][0]+1,good,quiet)
            if good>=6:received.set()
            if incoming and max(pcm)>500:
                # Both ringtone partials must actually be playing on the private sink.
                def amplitude(freq):
                    return 2*abs(sum(v*complex(math.cos(k*freq),math.sin(k*freq))
                        for k,v in enumerate(pcm[::4])))/len(pcm[::4])
                if all(amplitude(math.tau*hz/12000)>600 for hz in (440,620)):chime.set()
    def end(p):
        p.terminate();p.wait(timeout=5)
    for idx,suffix in enumerate(["","B"] if devices else [""]):
        feed=sp.Popen(["pacat","--playback","--device=QAMic"+suffix,*args],stdin=sp.PIPE,bufsize=0,env=env,stderr=sp.DEVNULL)
        stack.callback(end,feed)
        monitor=sp.Popen(["pacat","--record","--device=QASpeaker"+suffix+".monitor",*args],stdout=sp.PIPE,env=env,stderr=sp.DEVNULL)
        stack.callback(end,monitor)
        threading.Thread(target=inject,args=(feed,(480,730)[idx]),daemon=True).start()
        threading.Thread(target=observe,args=(monitor,idx),daemon=True).start()
    prefix=[]
    if network_ports:
        from qa_rtc_link import desktop_network
        prefix,outage=desktop_network(stack,network_ports)
    native_log=tmp/"native-rtc.log"
    logs={}
    if relay_links:
        prefix=relay_links["desktop"].prefix
        env["UNTETHERED_QA_ICE_STATS"]="1"
        logs["stderr"]=stack.enter_context(native_log.open("w"))
    client=stack.enter_context(child([*prefix,root/"rust-client/target/debug/untethered-iced"],env,**logs))
    def window():
        assert client.poll() is None,"desktop exited"
        return cmd("xdotool","search","--onlyvisible","--name","Untethered.*media-proof",text=True).stdout.strip()
    win=wait_for("desktop verified DM",window,40).splitlines()[0]
    cmd("xdotool","windowmove",win,"0","0",check=True)
    cmd("xdotool","windowsize",win,"1180","760",check=True)
    def shot():
        return Image.open(io.BytesIO(cmd("import","-window",win,"png:-",check=True).stdout)).convert("RGB")
    def words():
        # The header and the incoming-call banner.
        im=shot().crop((0,0,1180,190)).resize((2360,380))
        buf=io.BytesIO();im.save(buf,"PNG")
        data=sp.run(["tesseract","stdin","stdout","--psm","11","tsv"],input=buf.getvalue(),capture_output=True,check=True).stdout.decode()
        return [(s[11],(int(s[6])+int(s[8])//2)//2,(int(s[7])+int(s[9])//2)//2) for line in data.splitlines()
                if len(s:=line.split("\t"))==12 and s[0]=="5" and s[11].strip()]
    def dock_words():
        # The sidebar column, whose foot holds the call dock. Its grey labels
        # need the shared reader's sidebar pass.
        im=shot()
        return [(clean(w),x,y) for w,x,y in read(im.crop((0,0,CHAT_X,im.height)),dialogs=False,sidebar=True) if x>RAIL]
    def sidebar_room(name):
        # A channel or DM row: the topmost match, since the call dock below the
        # room list names the call's room too. The DM row sits below 190 px.
        return min(((x,y) for t,x,y in dock_words() if name in t),key=lambda p:p[1],default=None)
    def find(label):
        if label=="Voice":return call_ready()
        key=label.lower()
        if key in DOCK_ICONS:
            buttons=icon_buttons(shot())  # microphone first, hang up last
            return dock.pick(buttons[::-1] if key=="hang" else buttons,label,"above")
        if key in DOCK_TEXT:return next(((x,y) for t,x,y in dock_words() if t==key),None)
        return next(((x,y) for t,x,y in words() if clean(t)==key),None)
    def hover(pos):cmd("xdotool","mousemove","--window",win,*map(str,pos),check=True)
    ui=Ui(words,hover,width=1180,height=760)
    dock=Ui(dock_words,hover,width=1180,height=760)
    def call_ready():
        # The DM header's phone icon (tooltip "Start Voice Call") starts a call.
        return ui.header("start voice call")
    def click(label):
        x,y=wait_for("desktop "+label+" control",lambda:find(label),20)
        cmd("xdotool","mousemove","--window",win,str(x),str(y),"click","1",check=True)
    def roster():
        return page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get();
          return c.matrixRTC.getRoomSession(c.getRoom(room)).memberships.map(m=>m.userId)}""",room)
    def streams(kind):
        return [s for s in json.loads(cmd("pactl","-f","json","list",kind,check=True).stdout)
                if s["properties"].get("application.name")=="Untethered"]
    assert roster()==[] and not streams("source-outputs")
    def check_relay(widget):
        if relay_links:
            from qa_rtc_turn import verify_turn
            verify_turn(relay_links,widget,lambda:speakers[0],new_tone,native_log,roster,lambda:sfu_audio(secret),(alice,bob))
    live=True
    try:
        if incoming:
            reverse_call(stack,page,room,alice,bob,secret,find,click,roster,streams,chime,received,env,cmd,check_relay,sibling,set_dnd,lambda:speakers[0])
            return
        ring_started=threading.Event()
        page.on("console",lambda m:ring_started.set() if "LegacyCallHandler.play(ringAudio): playing audio successfully" in m.text else None)
        sockets=[]
        if controls or devices:
            page.on("websocket",lambda ws:sockets.append(ws) if urlsplit(ws.url).netloc=="127.0.0.1:18820" else None)
        click("Voice")
        toast=page.locator(".mx_IncomingCallToast_content")
        toast.get_by_text("Incoming voice call",exact=True).wait_for(timeout=25000)
        assert alice in toast.inner_text() and roster()==[alice]
        wait_for("Element ringtone playing",lambda:ring_started.is_set() or page.wait_for_timeout(100),8)
        def lease():
            return page.evaluate("""([room,user])=>{const c=window.mxMatrixClientPeg.get();const m=c.matrixRTC
                .getRoomSession(c.getRoom(room)).memberships.find(m=>m.userId===user);
                return m && {event:m.eventId, expiry:m.getAbsoluteExpiry()}}""",[room,alice])
        first=lease()
        assert not streams("source-outputs"),"microphone opened before answer"
        print("Desktop Voice call → Element displayed the incoming DM and started its ringtone; microphone still closed.",flush=True)
        toast.get_by_role("button",name="Join",exact=True).click()
        wait_for("both accepted members",lambda:set(roster())=={alice,bob},25)
        widget=next(f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/"))
        wait_for("Element decodes desktop mic's 480 Hz",lambda:widget.evaluate("window.qaAudioStats?.().then(s=>s.got480) ?? false"),30)
        assert received.wait(10),"desktop speaker did not render Element's 960 Hz"
        assert len(streams("source-outputs"))==len(streams("sink-inputs"))==1
        tracks=sfu_audio(secret)
        assert len(tracks)==2 and all(v==["GCM"] for v in tracks.values()) and {p.rsplit(":",1)[0] for p in tracks}=={alice,bob}, tracks
        print("Element accepted; Element measured 480 Hz on received WebRTC audio, desktop speaker monitor measured 960 Hz for 600 ms.",flush=True)
        print("LiveKit independently reports GCM on both microphone publications.",flush=True)
        if relay_links:
            check_relay(widget)
        elif network_ports:
            if outage_seconds<LEASE:
                reconnect_call(page,widget,room,lambda:speakers[0],new_tone,outage,find,streams,secret,alice,bob,outage_seconds)
            else:
                live=rejoin_call(page,widget,room,lambda:speakers[0],new_tone,outage,find,streams,alice,bob,outage_seconds)
        elif devices:
            def select(port,option):
                # Click the actual pick list on the line below the label (the
                # lowest match: a device error above the pickers can also say
                # "Microphone"). Its overlay opens upwards from the dock at the
                # foot of the window, so click a matching item above the list,
                # not a word in the other, closed pick list below it.
                _,y=wait_for(port+" picker",lambda:max(((x,y) for t,x,y in dock_words() if t==port.lower()),
                                                       key=lambda p:p[1],default=None))
                y+=PICK_BELOW
                cmd("xdotool","mousemove","--window",win,str(RAIL+SIDEBAR//2),str(y),"click","1",check=True)
                def item():
                    return next(((px,py) for t,px,py in dock_words() if t==option.lower() and y-200<py<y-10),None)
                px,py=wait_for(option+" in "+port+" menu",item,8)
                cmd("xdotool","mousemove","--window",win,str(px),str(py),"click","1",check=True)
            click("Devices")
            device_switch(widget,select,click,speakers,streams,cmd,secret,sockets,alice,bob,spare)
        elif controls:
            def change_room():
                x,y=wait_for("Sidechat sidebar entry",lambda:sidebar_room("sidechat"))
                cmd("xdotool","mousemove","--window",win,str(x),str(y),"click","1",check=True)
                wait_for("browsing another room",lambda:"Sidechat" in cmd("xdotool","getwindowname",win,text=True).stdout)
                assert set(roster())=={alice,bob}
                print("Switched the desktop to Sidechat during mute; DM call bar remained active.",flush=True)
            audio_controls(widget,click,lambda:speakers[0],streams,secret,sockets,alice,bob,change_room)
            # Return to the DM so the final assertion checks its Voice call button.
            x,y=wait_for("original DM sidebar entry",lambda:sidebar_room("media-proof"))
            cmd("xdotool","mousemove","--window",win,str(x),str(y),"click","1",check=True)
            wait_for("original DM selected",lambda:"media-proof" in cmd("xdotool","getwindowname",win,text=True).stdout)
        else:
            while time.time()*1000 < first["expiry"]+1200:
                assert set(roster())=={alice,bob}, f"lost a member {int((time.time()*1000-first['expiry'])/1000+60)}s after join"
                page.wait_for_timeout(400)
            updated=lease()
            assert updated["event"]!=first["event"] and updated["expiry"]>time.time()*1000
            sample=widget.evaluate("window.qaAudioStats()")
            assert sample["got480"] and sample["spectra"]-sample["last480"]<5
            received.clear()
            assert received.wait(3)
            print("Both tones still present past the original 60-second membership expiry.",flush=True)
        if not live:pass  # Element already hung up.
        elif devices or network_ports or relay_links:widget.get_by_role("button",name="End call",exact=True).click(timeout=8000)
        else:click("Hang")
        wait_for("desktop and Element left DM call",lambda:roster()==[],22)
        wait_for("desktop released both audio devices",lambda:not streams("source-outputs") and not streams("sink-inputs"))
        wait_for("desktop ready to call again",lambda:find("Voice"),12)
        print("Hangup → empty Element call roster, 0 desktop capture/playback streams; call control available again.",flush=True)
    except Exception:
        if relay_links:
            from qa_rtc_turn import counters
            print("Relay-only counters:",{k:counters(v) for k,v in relay_links.items()},flush=True)
            print("Native ICE:",[l for l in native_log.read_text(errors="replace").splitlines() if l.startswith("RTC selected audio")][-8:],flush=True)
        print("Desktop visible labels:", " ".join(t for t,_,_ in words()),flush=True)
        print("Desktop sidebar and call dock:", " ".join(t for t,_,_ in dock_words()),flush=True)
        print("Element roster:",roster(),"incoming toast:",page.locator(".mx_IncomingCallToast_content").all_text_contents(),flush=True)
        print("Membership timeline:",page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room)
            .getLiveTimeline().getEvents().filter(e=>e.getType()==="org.matrix.msc3401.call.member")
            .map(e=>[e.getTs(),e.getSender(),e.getContent().expires??"leave"])""",room),flush=True)
        if incoming:
            print("Call notification summary:",page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room)
                .getLiveTimeline().getEvents().filter(e=>e.getType().includes("notification")||e.getType().includes("decline"))
                .map(e=>[e.getType(),e.getContent().notification_type,e.getContent()["m.call.intent"],e.getWireType()])""",room),flush=True)
        raise


# The desktop's MSC4140 delayed leave fires this long after its last heartbeat.
LEASE=16


def reconnect_call(page,widget,room,speaker,new_tone,outage,find,streams,secret,alice,bob,seconds=6.5):
    def matrix():
        return page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get(), r=c.getRoom(room);
          return [c.matrixRTC.getRoomSession(r).memberships.map(m=>[m.userId,m.deviceId,m.memberId]).sort(),
            r.getLiveTimeline().getEvents().filter(e=>e.getType()==='org.matrix.msc4075.rtc.notification' &&
              e.getContent().notification_type==='ring').map(e=>e.getId())]}""",room)
    members,rings=matrix()
    assert len(rings)==1 and len(members)==2 and {m[0] for m in members}=={alice,bob}
    expected={m[0]+":"+m[1] for m in members}
    before=sfu_participants(secret)
    assert len(before)==2 and {p["identity"] for p in before}==expected
    assert all([t.get("encryption") for t in p.get("tracks",[])]==["GCM"] for p in before)
    page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get(),s=c.matrixRTC.getRoomSession(c.getRoom(room));
        const snapshot=()=>s.memberships.map(m=>[m.userId,m.deviceId,m.memberId]).sort();
        const before=JSON.stringify(snapshot());window.qaRosterChanged=false;
        s.on("memberships_changed",()=>{if(JSON.stringify(snapshot())!==before)window.qaRosterChanged=true;});
    }""",room)
    def check_rosters():
        assert not page.evaluate("window.qaRosterChanged"),"MatrixRTC roster changed during the outage"
        assert matrix()==[members,rings],"Matrix participant or ring count changed"
        peers=sfu_participants(secret)
        ids=[p["identity"] for p in peers]
        assert set(ids)<=expected and len(set(ids))==len(ids),"unexpected or duplicate SFU participant"
        return peers
    def stats():return widget.evaluate("window.qaAudioStats()")
    initial=stats()
    assert initial["run480"] and speaker()[1]>0
    start=time.monotonic()
    with outage():
        def silent():
            check_rosters()
            return stats()["quiet"]>=12 and speaker()[2]>=12
        wait_for("both clients lost audio after cable cut",silent,10)
        wait_for("Linux reconnect indication",lambda:find("Reconnecting"),9)
        frozen=stats()["packets"]
        new_tone.set()
        while time.monotonic()-start<seconds:
            check_rosters()
            assert stats()["packets"]==frozen and stats()["hits730"]==0
            page.wait_for_timeout(200)
        assert stats()["packets"]==frozen,"inbound audio packets continued during outage"
        assert not streams("source-outputs") and not streams("sink-inputs")
        print("Link down: both outputs silent, RTP stopped, desktop showed Reconnecting and released audio devices; switched test mic to 730 Hz.",flush=True)
    restored=time.monotonic()
    print(f"Restored desktop link after {restored-start:.1f} s; no user action on either client.",flush=True)
    first=speaker()[0]
    def recovered():
        peers=check_rosters()
        s=stats()
        if len(peers)!=2 or s["run730"]<12 or min(speaker()[0]-first,speaker()[1])<12:return False
        return all([t.get("encryption") for t in p.get("tracks",[])]==["GCM"] for p in peers)
    wait_for("two encrypted tones recovered without redial",recovered,42)
    wait_for("Linux call bar connected again",lambda:find("Connected"),5)
    assert matrix()==[members,rings]
    assert len(streams("source-outputs"))==len(streams("sink-inputs"))==1
    print(f"Recovered in {time.monotonic()-restored:.1f} s: new 730 Hz at Element, fresh 960 Hz at desktop for 1.2 s; "
          "two GCM microphones, two original Matrix devices, exactly one ring.",flush=True)


def rejoin_call(page,widget,room,speaker,new_tone,outage,find,streams,alice,bob,seconds):
    """An outage past the delayed leave. The desktop may rejoin only the same
    call: with a new membership, no new ring, and only while the peer's
    original session is active. Element's DM calls leave when everyone else
    has (autoLeaveWhenOthersLeft), so this also checks the refusal."""
    def memberships():
        # The homeserver's state, not Element's view of it.
        return {s:(e,c) for s,e,c in page.evaluate("""async room=>(await window.mxMatrixClientPeg.get().roomState(room))
            .filter(e=>e.type==='org.matrix.msc3401.call.member' && e.content.application==='m.call')
            .map(e=>[e.sender,e.event_id,e.content.created_ts])""",room)}
    def rings():
        return page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room).getLiveTimeline().getEvents()
            .filter(e=>e.getType()==='org.matrix.msc4075.rtc.notification' && e.getContent().notification_type==='ring')
            .map(e=>e.getId())""",room)
    before=memberships()
    ring=rings()
    assert set(before)=={alice,bob} and len(ring)==1,(before,ring)
    start=time.monotonic()
    with outage():
        wait_for("both clients lost audio after cable cut",
                 lambda:widget.evaluate("window.qaAudioStats()")["quiet"]>=12 and speaker()[2]>=12,10)
        wait_for("Linux reconnect indication",lambda:find("Reconnecting"),9)
        new_tone.set()
        wait_for("desktop's delayed leave fired",lambda:alice not in memberships(),seconds)
        expired=time.monotonic()-start
        wait_for(f"{seconds:.0f}-second outage",lambda:time.monotonic()-start>=seconds,seconds)
        stayed=bob in memberships()
        assert stayed or memberships()=={},memberships()
    restored=time.monotonic()
    print(f"Desktop link down for {restored-start:.1f} s; its membership expired after {expired:.1f} s. "
          f"Element {'kept its session' if stayed else 'left the call when the desktop expired'}.",flush=True)
    if not stayed:
        # The peer's session is gone: the desktop ends the call instead of
        # starting a new one without a ring.
        wait_for("desktop ended the call",lambda:find("Voice"),30)
        for _ in range(10):
            assert memberships()=={},"the desktop published a membership into an ended call"
            page.wait_for_timeout(500)
        assert not streams("source-outputs") and not streams("sink-inputs")
        assert rings()==ring
        print(f"Ended {time.monotonic()-restored:.1f} s after the link returned: no new membership or ring, "
              "both audio devices released, call control available again.",flush=True)
        return False
    first=speaker()[0]
    def rejoined():
        now=memberships()
        if alice not in now or now[alice][1]<=before[alice][1]:return False
        s=widget.evaluate("window.qaAudioStats()")
        return s["run730"]>=12 and min(speaker()[0]-first,speaker()[1])>=12
    wait_for("desktop rejoined with a fresh membership and two-way audio",rejoined,60)
    after=memberships()
    assert after[bob][1]==before[bob][1] and rings()==ring
    print(f"Rejoined {time.monotonic()-restored:.1f} s after the link returned: new membership {after[alice][0]} "
          f"(created {after[alice][1]-before[alice][1]} ms later), Element's original session, exactly one ring.",flush=True)
    return True


def continuity(widget,secret,sockets,alice,bob):
    initial=widget.evaluate("window.qaAudioStats()")
    original_sockets=tuple(sockets)
    assert initial["ssrc"] and initial["state"]=="connected" and original_sockets
    def publications():
        tracks=sfu_publications(secret)
        ids={who.rsplit(":",1)[0]:(participant,track["sid"],track.get("encryption")) for who,participant,track in tracks}
        mutes={who.rsplit(":",1)[0]:track.get("muted",False) for who,_,track in tracks}
        assert len(tracks)==2 and set(ids)=={alice,bob} and all(v[-1]=="GCM" for v in ids.values())
        return ids,mutes
    original,mutes=publications()
    assert mutes=={alice:False,bob:False}
    def stats():
        s=widget.evaluate("window.qaAudioStats()")
        assert s["state"]=="connected" and all(s[k]==initial[k] for k in ("receiverId","trackId","ssrc","peerConnections","losses"))
        assert tuple(sockets)==original_sockets and all(not ws.is_closed() for ws in sockets)
        return s
    def check_tracks(mic_on):
        ids,mutes=publications()
        assert ids==original and mutes=={alice:not mic_on,bob:False}, "encrypted track, participant or mute state changed"
    return stats,check_tracks


def device_switch(widget,select,click,speakers,streams,cmd,secret,sockets,alice,bob,spare):
    stats,check_tracks=continuity(widget,secret,sockets,alice,bob)
    def server_defaults():
        return tuple(cmd("env","-u","PULSE_SOURCE","-u","PULSE_SINK","pactl","get-default-"+kind,check=True).stdout
                     for kind in ("source","sink"))
    defaults=server_defaults()
    assert tuple(x.strip() for x in defaults)==(b"QAMic.monitor",b"QASpeaker")
    def indices(kind):
        return {v["name"]:int(v["index"]) for v in json.loads(cmd("pactl","-f","json","list",kind,check=True).stdout)}
    sources,sinks=indices("sources"),indices("sinks")
    def check(mic_b,speaker_b):
        spec=stats()["spectra"];start=[m[0] for m in speakers]
        def match():
            s=stats()
            return min(s["spectra"]-spec,s["run730" if mic_b else "run480"],
                       *(min(m[0]-start[i],m[1 if i==speaker_b else 2]) for i,m in enumerate(speakers)))>=12
        wait_for("fresh microphone and both sink measurements",match,12)
        src,=streams("source-outputs");out,=streams("sink-inputs")
        assert int(src["source"])==sources["QAMic"+("B" if mic_b else "")+".monitor"]
        assert int(out["sink"])==sinks["QASpeaker"+("B" if speaker_b else "")]
        check_tracks(True)
        print(f"Device route: Element received {730 if mic_b else 480} Hz; "
              f"{'Grove' if speaker_b else 'Harbor'} played 960 Hz, {'Harbor' if speaker_b else 'Grove'} silent "
              "(12 fresh 100 ms windows at all three measurement points).",flush=True)
    check(False,False)
    select("Microphone","Cobalt");check(True,False)
    select("Speaker","Grove");check(True,True)
    select("Microphone","Amber");check(False,True)
    select("Speaker","default");check(False,False)
    assert server_defaults()==defaults,"switching live call routes changed server defaults"
    def mic_closed():
        first=stats()["spectra"];start=speakers[0][0]
        def silent():
            s=stats();n,tone,_=speakers[0]
            return min(s["quiet"],s["spectra"]-first,tone,n-start)>=12
        wait_for("unavailable or muted mic stays silent while playback continues",silent,12)
        assert not streams("source-outputs")
        check_tracks(False)
    click("Mute")
    select("Microphone","Spare");mic_closed()
    cmd("pactl","unload-module",spare,check=True)
    click("Refresh")
    click("Unmute");mic_closed()
    print("Removed the selected mic while muted: no fallback capture after Unmute; Element silent, speaker still at 960 Hz.",flush=True)
    select("Microphone","default");check(False,False)
    assert server_defaults()==defaults,"call selection changed system audio defaults"
    print("Device switches kept both GCM track SIDs, participant SIDs, browser receiver, SSRC and SFU signaling sockets.",flush=True)


def audio_controls(widget,click,speaker,streams,secret,sockets,alice,bob,change_room):
    """Use fresh PCM after every click; an old 'tone seen' latch can't pass."""
    stats,check_tracks=continuity(widget,secret,sockets,alice,bob)
    def phase(button,mic_on,output_on):
        click(button)
        first=stats()["spectra"]
        start=speaker()[0]
        def match():
            s=stats(); n,tone,quiet=speaker()
            return min(s["spectra"]-first,n-start,s["run480"] if mic_on else s["quiet"],tone if output_on else quiet)>=12
        wait_for(button+" measured at both clients",match,12)
        check_tracks(mic_on)
        assert len(streams("source-outputs"))==int(mic_on),"capture did not follow effective mute state"
        print(f"{button}: Element {'480 Hz' if mic_on else 'silent (< -65 dB FFT peak)'}; "
              f"desktop {'960 Hz' if output_on else 'silent (< 32 PCM peak)'} for 12 fresh 100 ms windows.",flush=True)
    phase("Mute",False,True)
    change_room()
    phase("Deafen",False,False)
    phase("Undeafen",False,True)  # explicit mute survives
    phase("Unmute",True,True)
    phase("Deafen",False,False)   # open mic is automatically muted
    phase("Undeafen",True,True)
    print("No reconnect: same Element receiver, SSRC and signaling sockets; both original GCM track and participant SIDs retained.",flush=True)


def reverse_call(stack,page,room,alice,bob,secret,find,click,roster,streams,chime,received,env,cmd,check_relay,sibling,set_dnd,speaker):
    from playwright.sync_api import Error as BrowserError
    # Count stream creations too: polling an empty list after Decline alone
    # would miss a microphone that briefly opened and was then closed.
    watcher=sp.Popen(["stdbuf","-oL","pactl","subscribe"],env={**env,"LC_ALL":"C"},stdout=sp.PIPE,text=True)
    events=[]
    reader=threading.Thread(target=lambda:events.extend(line.strip() for line in watcher.stdout),daemon=True)
    def stop():
        watcher.terminate();watcher.wait(timeout=5);reader.join(timeout=2)
    stack.callback(stop);reader.start()
    def subscribed():
        cmd("pactl","info",check=True)
        return bool(events)
    wait_for("PulseAudio subscription",subscribed)
    def capture_starts():return sum("'new' on source-output" in line for line in events)
    def ring_ids():
        return page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room).getLiveTimeline().getEvents()
            .filter(e=>e.getType()==="org.matrix.msc4075.rtc.notification"&&e.getContent().notification_type==="ring")
            .map(e=>e.getId())""",room)
    def silent():
        first=speaker()[0]
        wait_for("silent speaker and no remaining playback stream",lambda:min(speaker()[0]-first,speaker()[2])>=8
                 and not streams("sink-inputs"),8)
    def playback_starts():return sum("'new' on sink-input" in line for line in events)
    def dial(quiet=False):
        before=set(ring_ids());chime.clear()
        played=playback_starts()
        if sibling:ring_frames=sibling.ring_frames
        page.get_by_role("button",name="Voice call",exact=True).click(timeout=15000)
        def ringing():
            for f in page.frames:
                if f==page.main_frame:continue
                button=f.get_by_role("button",name=re.compile(r"^(Start call|Join call|Join)$",re.I))
                try:
                    if button.count() and button.first.is_visible():button.first.click(timeout=1500)
                except BrowserError:
                    if not f.is_detached():raise
            return roster()==[bob] and bool(set(ring_ids())-before)
        wait_for("Element originated the DM ring",ringing,22)
        ids=set(ring_ids())-before;assert len(ids)==1
        wait_for("Untethered incoming voice banner",lambda:find("Accept") and find("Decline"),12)
        if quiet:
            silent()
            assert not chime.is_set() and playback_starts()==played,"DND started ringtone playback"
        else:assert chime.wait(4),"no 440/620 Hz ringtone on the desktop speaker"
        assert not streams("source-outputs") and capture_starts()==0
        ring=ids.pop()
        if sibling:sibling.ringing(ring,room,bob,ring_frames)
        return ring
    ring=dial()
    if set_dnd:
        set_dnd(True);silent()
        assert find("Accept") and find("Decline") and roster()==[bob] and not capture_starts()
        print("Enabling DND from another session stopped an audible 440/620 Hz ring; controls remained active with 800 ms silence.",flush=True)
    if sibling:sibling.tap("decline")
    else:click("Decline")
    def declined():
        return page.evaluate("""([room,id,sender])=>window.mxMatrixClientPeg.get().getRoom(room).getLiveTimeline().getEvents()
            .filter(e=>e.getType()==="org.matrix.msc4310.rtc.decline"&&e.getSender()===sender&&e.getRelation()?.event_id===id)
            .map(e=>e.getWireType())""",[room,ring,alice])
    wait_for("Element received correlated encrypted decline",lambda:declined()==["m.room.encrypted"],12)
    wait_for("caller stopped after decline",lambda:roster()==[],15)
    assert not find("Accept") and not streams("source-outputs") and not streams("sink-inputs") and capture_starts()==0
    if sibling:
        wait_for("first desktop saw sibling's decline",lambda:find("declined") and find("another"))
        sibling.quiet()
        print(f"{sibling.other_id} declined {ring}; {sibling.main_id} showed 'declined on another device'.",flush=True)
    print("First Element call: audible 440/620 Hz desktop ring; Decline received for that invite; 0 microphone streams created.",flush=True)
    page.goto(page.url.split("#")[0]+"#/room/"+room)
    second=dial(quiet=bool(set_dnd))
    assert second!=ring
    started=time.monotonic()
    click("Accept")
    wait_for("Element and Untethered joined the answered call",lambda:set(roster())=={alice,bob},20)
    if sibling:sibling.answered(started,peer=page,room=room,ring=second)
    widget=next(f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/"))
    wait_for("Element receives accepted caller's desktop audio",lambda:widget.evaluate("window.qaAudioStats?.().then(s=>s.got480) ?? false"),25)
    assert received.wait(6)
    assert len(streams("source-outputs"))==1 and capture_starts()==1
    tracks=sfu_audio(secret)
    assert len(tracks)==2 and all(v==["GCM"] for v in tracks.values()) and {p.rsplit(":",1)[0] for p in tracks}=={alice,bob},tracks
    if set_dnd:
        print("New incoming call under DND: no ringtone stream was opened; Accept still established two-way encrypted speech.",flush=True)
        set_dnd(False)
    if sibling:
        assert set(tracks)=={alice+":"+sibling.main_id,bob+":"+page.evaluate("window.mxMatrixClientPeg.get().getDeviceId()")}
        sibling.quiet()
    print("Second Element call: Accept opened 1 microphone; Element decoded 480 Hz, desktop played 960 Hz; both tracks GCM.",flush=True)
    check_relay(widget)
    widget.get_by_role("button",name="End call",exact=True).click(timeout=8000)
    wait_for("both devices left after Element hung up",lambda:roster()==[],20)
    wait_for("desktop audio closed",lambda:not streams("source-outputs") and not streams("sink-inputs"))
    assert not find("Accept") and find("Voice")
    if sibling:
        sibling.quiet()
        assert len(ring_ids())==2
        declines=page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room).getLiveTimeline().getEvents()
            .filter(e=>e.getType()==='org.matrix.msc4310.rtc.decline').map(e=>e.getRelation()?.event_id)""",room)
        assert declines==[ring],declines
    print("Element hung up: both devices left, 0 desktop capture/playback streams.",flush=True)
