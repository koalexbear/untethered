"""Ring and answer two real clients. Verify camera pixels rather than just track metadata."""
import json
import sqlite3
import subprocess as sp
from contextlib import closing

from qa_desktop_dm import sfu_participants
from qa_kwin import KWin
from qa_native_paste import child,isolated_env,wait_for
from qa_room_camera import camera_source,grant_camera,start_camera
from qa_room_layout import pattern_box
from qa_voice_room import Desktop


def check_video(stack,tmp,root,backend,room,users,secret,sidechat,share=False):
    sock=tmp/"pulse"
    desks=[];show=[]
    palettes=[[((31,215,123),(220,40,178)),((244,117,24),(40,83,242))],
              [((27,212,236),(236,218,26)),((223,43,47),(155,97,229))]]
    for i,path in enumerate([tmp,tmp/"peer"]):
        env=isolated_env(path,backend,"wayland")
        env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE=f"Mic{i}.monitor",PULSE_SINK=f"Out{i}",
                   RAYON_NUM_THREADS="2",TOKIO_WORKER_THREADS="2",GOMAXPROCS="2")
        if i==0:
            stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
                f"--load=module-native-protocol-unix socket={sock} auth-anonymous=1",
                *(f"--load=module-null-sink sink_name={k}{j} rate=48000 channels=1" for j in range(2) for k in ("Mic","Out"))],env))
            wait_for("two private audio routes",lambda:sock.is_socket() and "Out1" in sp.run(
                ["pactl","list","short","sinks"],env=env,text=True,capture_output=True).stdout)
        kde=KWin(stack,path,root,backend,env=env,screencast=True)
        desks.append(Desktop(stack,root,env,i,kde,room="media-proof"))
        show.append(camera_source(stack,kde,palettes[i]))
    a,b=desks
    def tracks():
        return {(p["identity"].rsplit(":",1)[0],t["source"]):(t["sid"],t.get("encryption"))
                for p in sfu_participants(secret,allow_empty=True) for t in p.get("tracks",[])}
    def readers(d):
        return d.kde.command("pgrep","-P",str(d.kde.native.pid),"-x","gst-launch-1.0",check=False).stdout.splitlines()
    def box(i,phase,local=False):
        return pattern_box(desks[i].kde.capture().crop((283,82,1179,850)),palettes[i if local else 1-i][phase])
    def dock_box(peer,palette):
        return pattern_box(peer.kde.capture().crop((72,250,312,795)),palette)
    def expand(peer):
        def opened():
            ws=peer.words()
            if any(w=="theater" and x>312 for w,x,y in ws):return True
            target=next(((x,y) for w,x,y in ws if w in ("open","view") and 72<x<312),None)
            if target:
                x,y=target
                peer.kde.command("kdotool","windowactivate",peer.win)
                peer.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":300},
                                           {"type":"pointerDown","button":0},{"type":"pause","duration":130},{"type":"pointerUp","button":0}])
            return False
        wait_for("expand call from sidebar dock",opened,15)
    def dock_control(peer,slot):
        # Four square controls: mic, camera, screen share, disconnect.
        # Anchor to the red disconnect button, independent of preview height.
        def locate():
            im=peer.kde.capture()
            for y in range(280,795):
                xs=[x for x in range(195,278) if im.getpixel((x,y))[:3]==(181,51,69)]
                if len(xs)>=32:
                    right=(xs[0]+xs[-1])//2
                    if slot==3:return right,y+11
                    for x in range(77,145):
                        if im.getpixel((x,y+11))[:3] in ((56,58,64),(81,84,104),(35,118,78)):
                            return x+16+slot*41,y+11
        x,y=wait_for("call dock control",locate)
        peer.kde.command("kdotool","windowactivate",peer.win)
        peer.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":380},
                                   {"type":"pointerDown","button":0},{"type":"pause","duration":150},{"type":"pointerUp","button":0}])
    def audio():return a.hears({670}) and b.hears({410})
    a.click("video")
    try:wait_for("incoming video call accepts camera-off",lambda:{"incoming","video","accept","off"}<={w for w,_,_ in b.words()},15)
    except RuntimeError:print("Caller labels:",a.words(),"callee labels:",b.words(),flush=True);raise
    with closing(sqlite3.connect((tmp/"peer/seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True)) as db:
        [(ring,body)]=db.execute("SELECT event_id,decrypted FROM event WHERE room_id=? AND type='m.room.encrypted' "
            "AND decrypted_type='org.matrix.msc4075.rtc.notification'",(room,))
        assert json.loads(body)["m.call.intent"]=="video"
    assert not tracks() and all(not readers(d) for d in desks)
    b.click("accept")
    grant_camera(a.kde,lambda:(users[0],"CAMERA") in tracks())
    wait_for("callee sees caller in docked preview",lambda:dock_box(b,palettes[0][0]),20)
    expand(a);expand(b)
    wait_for("callee sees caller on expanded stage",lambda:box(1,0),20)
    wait_for("audio live",audio)
    assert not readers(b) and (users[1],"CAMERA") not in tracks()
    dock_control(b,1);grant_camera(b.kde,lambda:(users[1],"CAMERA") in tracks())
    before=tracks()
    assert set(before)=={(u,k) for u in users for k in ("CAMERA","MICROPHONE")} and all(e=="GCM" for _,e in before.values())
    for i in range(2):
        wait_for("peer pattern on "+users[i],lambda:box(i,0),18)
        show[1-i](1)
        bounds=wait_for("changing decoded video on "+users[i],lambda:box(i,1),18)
        print(f"{users[i]} rendered its peer's changing camera at {bounds[2]-bounds[0]}×{bounds[3]-bounds[1]}.",flush=True)
    if share:
        from qa_window_share import dm_share
        def screen(pair):
            return pattern_box(b.kde.capture().crop((315,280,1179,470)),pair)
        def fresh_voice():
            for d in desks:d.windows.clear()
            wait_for("fresh two-way audio with independent DM share",audio)
        dm_share(stack,a,users[0],tracks,lambda:readers(a),screen,fresh_voice)
        wait_for("peer camera restored to main stage",lambda:(r:=box(1,1)) and r[2]-r[0]>450)
    small=wait_for("local preview visible beside peer",lambda:box(0,1,True))
    assert small[2]-small[0]<170 and box(0,1)[2]-box(0,1)[0]>450
    def select(rect):
        a.kde.command("kdotool","windowactivate",a.win)
        x,y=283+rect[0]+20,82+rect[1]+25
        a.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":250},
                                 {"type":"pointerDown","button":0},{"type":"pause","duration":120},{"type":"pointerUp","button":0}])
    def focus(local):
        def target():
            r=box(0,1,local)
            if not r:return False
            if r[2]-r[0]>450:return True
            select(r)
            try:return wait_for("stage updated",lambda:(big:=box(0,1,local)) and big[2]-big[0]>450,3)
            except RuntimeError:return False
        wait_for("selected camera focused",target)
    focus(True);focus(False)
    print("Live self-preview and remote tile swapped focus and back without replacing any publication.",flush=True)
    assert tracks()==before and audio()
    other=sidechat()
    jobs=[readers(d) for d in desks]
    def compact(phase):
        r=dock_box(a,palettes[1][phase])
        return r and 90<=r[2]-r[0]<=106 and 45<=r[3]-r[1]<=60
    def fresh():
        for d in desks:d.windows.clear()
        wait_for("fresh 410/670 Hz audio after navigation",audio,12)
        assert tracks()==before and [readers(d) for d in desks]==jobs
    def navigate(label,done):
        def visit():
            if done():return True
            a.click(label)
            try:return wait_for(label+" applied",done,2)
            except RuntimeError:return False
        wait_for(label+" navigation",visit)
    navigate("chat",lambda:compact(1))
    expand(a)
    def title(name):
        return name in a.kde.command("kdotool","getwindowname",a.win).stdout
    def go_side():
        if title("Sidechat"):return True
        pos=next(((x,y) for w,x,y in a.kde.words((76,145,274,300)) if w.lower()=="sidechat"),None)
        if pos:click_at(*pos)
        return False
    def click_at(x,y):
        a.kde.command("kdotool","windowactivate",a.win)
        a.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":220},
                               {"type":"pointerDown","button":0},{"type":"pause","duration":120},{"type":"pointerUp","button":0}])
    wait_for("open Sidechat from sidebar",go_side)
    wait_for("Sidechat selected with live compact peer video",lambda:title("Sidechat") and compact(1))
    show[1](0);wait_for("fresh peer pixels in compact view",lambda:compact(0))
    a.click("hide")
    wait_for("hidden local preview control",lambda:any(w=="show" for w,_,_ in a.words()) and not compact(0))
    show[1](1);show[0](0)
    wait_for("peer receives new camera frames while preview hidden",lambda:box(1,0))
    assert not compact(1)
    fresh()
    a.click("show");wait_for("compact view resumed",lambda:compact(1))
    def composer():
        words=a.kde.words((285,680,1179,850))
        return next(((x,y) for w,x,y in words if w.lower()=="message"),None)
    marker="meeting notes from sidechat"
    _,cy=wait_for("Sidechat composer",composer)
    click_at(650,cy);a.kde.type(marker)
    def draft():
        return next(((x,y) for w,x,y in a.kde.words((330,720,1000,820)) if w.lower()=="meeting"),None)
    wait_for("visible Sidechat draft before navigation",draft)
    expand(a)
    wait_for("return to full DM stage",lambda:title("media-proof") and (r:=box(0,1)) and r[2]-r[0]>450)
    fresh()
    wait_for("open Sidechat again",go_side);wait_for("returned to Sidechat",lambda:title("Sidechat") and compact(1))
    # Send the preserved draft without retyping it. Verify the other account
    # decrypted the event in Sidechat, not in the active call's DM.
    click_at(*wait_for("preserved Sidechat draft",draft))
    a.kde.keys(("Down","\ue007"),("Up","\ue007"))
    def delivered():
        with closing(sqlite3.connect((tmp/"peer/seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True)) as db:
            return list(db.execute("SELECT room_id,type FROM event WHERE sender=? AND "
                "decrypted_type='m.room.message' AND json_extract(decrypted,'$.body')=?",(users[0],marker)))
    wait_for("peer decrypted Sidechat message",delivered,20)
    assert delivered()==[(other,"m.room.encrypted")]
    print("Sidechat: peer video changed in the sidebar dock; hide/show preserved the call. Open call view restored the full stage, "
          "Sidechat kept its draft, and the peer decrypted it only in Sidechat. Same four GCM SIDs and camera readers.",flush=True)
    dock_control(b,1)
    wait_for("only callee's camera stops",lambda:not readers(b) and (users[1],"CAMERA") not in tracks() and not compact(1),15)
    assert box(1,0) and audio()
    wait_for("dock remains available with no stale picture",lambda:any(w=="open" and 72<x<312 for w,x,y in a.words()) and not compact(0))
    dock_control(a,3)
    wait_for("hangup releases both cameras and all SFU tracks",lambda:not tracks() and not readers(a) and not readers(b),20)
    assert title("Sidechat") and not box(0,0) and not box(0,1)
    print(f"Encrypted video invite {ring}; no camera before acceptance; receiver opted in separately. "
          "Both camera and mic pairs GCM; camera-off retained two-way audio and the other feed; hangup released capture.",flush=True)
