"""Independent second receiver for the two-device incoming DM call lifecycle."""
import array
import cmath
import json
import math
import subprocess as sp
import threading
import time
from pathlib import Path

import qa_events
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_replies import screen_words


class Sibling:
    def __init__(self,stack,tmp,root,backend,account,user):
        directory=tmp/"second";directory.mkdir()
        env=self.env=isolated_env(directory,backend,"x11")
        start_display(stack,directory,env,"x11")
        sock=Path(env["XDG_RUNTIME_DIR"])/"pulse-qa"
        env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE="Spare.monitor",PULSE_SINK="Spare")
        stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
            "--load=module-native-protocol-unix socket="+str(sock)+" auth-anonymous=1",
            "--load=module-null-sink sink_name=Spare rate=48000 channels=1"],env))
        wait_for("second private PulseAudio",lambda:sock.is_socket() and self.cmd("pactl","info",check=False).returncode==0)
        monitor=stack.enter_context(child(["pacat","--record","--raw","--format=s16le","--rate=16000","--channels=1",
            "--device=Spare.monitor","--latency-msec=40"],env,stdout=sp.PIPE))
        self.ring_frames=self.silent=self.capture_starts=0
        def listen():
            bins=[[cmath.exp(-1j*math.tau*f*n/16000) for n in range(1600)] for f in (440,620)]
            while data:=monitor.stdout.read(3200):
                pcm=array.array("h");pcm.frombytes(data)
                if len(pcm)!=1600:return
                self.silent=self.silent+1 if max(map(abs,pcm))<32 else 0
                if all(2*abs(sum(v*b for v,b in zip(pcm,ws)))/len(pcm)>600 for ws in bins):
                    self.ring_frames+=1
        threading.Thread(target=listen,daemon=True).start()
        # Subscribe after the harness's sole monitor starts but before Iced
        # starts, so even a briefly opened second-device microphone is counted.
        watcher=stack.enter_context(child(["stdbuf","-oL","pactl","subscribe"],{**env,"LC_ALL":"C"},stdout=sp.PIPE,text=True))
        subscribed=threading.Event()
        def observe():
            for line in watcher.stdout:
                subscribed.set()
                if "'new' on source-output" in line:self.capture_starts+=1
        threading.Thread(target=observe,daemon=True).start()
        def ready():
            self.cmd("pactl","info")
            return subscribed.wait(.05)
        wait_for("second-device stream event subscription",ready)
        gui=stack.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
        def window():
            assert gui.poll() is None
            return self.cmd("xdotool","search","--all","--onlyvisible","--pid",str(gui.pid),"--name","Untethered",check=False).stdout.strip()
        self.win=wait_for("second Linux window",window)
        self.cmd("xdotool","windowsize",self.win,"1180","760","windowfocus",self.win)
        self.click(162,728);self.label("welcome")
        # The fields sit under their captions; the card recentres for notices.
        for caption,field in [("homeserver","server"),("username","username"),("password","password")]:
            x,y=self.label(caption);self.click(590,y+37);self.type(account[field])
        self.cmd("xdotool","key","Return");self.tap("passphrase");self.type(account["key"]);self.cmd("xdotool","key","Return")
        self.label("media-proof")
        self.profile=directory/"seed"
        def identity(profile):
            session=json.loads((profile/"matrix-sdk/session.json").read_text())["session"]
            assert session["user_id"]==user
            return session["device_id"]
        self.main_id=identity(tmp/"seed");self.other_id=identity(self.profile)
        assert self.main_id!=self.other_id
        print(f"Two verified Linux devices on {user}: {self.main_id} and {self.other_id}, separate X and audio servers.",flush=True)

    def cmd(self,*args,check=True):
        return sp.run(args,env=self.env,capture_output=True,text=True,timeout=8,check=check)
    def words(self):
        raw=sp.check_output(["import","-window",self.win,"png:-"],env=self.env,timeout=6)
        return screen_words(raw)
    def label(self,word):
        return wait_for("second desktop "+word,lambda:next(((x,y) for w,x,y in self.words() if w==word),None),25)
    def click(self,x,y):self.cmd("xdotool","mousemove","--window",self.win,str(x),str(y),"click","1")
    def tap(self,word):self.click(*self.label(word))
    def type(self,value):
        sp.run(["xdotool","type","--clearmodifiers","--file","-"],env=self.env,input=value,text=True,check=True)
    def playback(self):
        return [s["properties"]["media.name"] for s in json.loads(self.cmd("pactl","-f","json","list","sink-inputs").stdout)
                if s["properties"].get("application.name")=="Untethered"]
    def ringing(self,ring,room,peer,baseline):
        self.label("accept");self.label("decline")
        wait_for("second speaker chime (440 + 620 Hz)",lambda:self.ring_frames>baseline,5)
        assert self.playback()==["Incoming call"] and self.capture_starts==0
        e=wait_for("second device decrypted the ring",lambda:qa_events.event(self.profile,room,ring),5)
        assert e["encrypted"] and e["sender"]==peer,e
        assert e["type"]=="org.matrix.msc4075.rtc.notification" and e["content"]["notification_type"]=="ring"
    def quiet(self):
        wait_for("second speaker silent and ringtone stream removed",lambda:self.silent>=6 and not self.playback(),6)
        assert self.capture_starts==0
        assert not {w for w,_,_ in self.words()} & {"accept","decline"}
    def answered(self,started,peer,room,ring):
        self.label("answered");self.label("another");self.quiet()
        elapsed=time.monotonic()-started
        # The still-live invite is well inside its actual Matrix expiry.
        remaining=peer.evaluate("""([r,id])=>{const e=window.mxMatrixClientPeg.get().getRoom(r).findEventById(id),
            c=e.getContent();return c.sender_ts+c.lifetime-Date.now()}""",[room,ring])
        assert remaining>0
        members=peer.evaluate("""r=>{const c=window.mxMatrixClientPeg.get();return c.matrixRTC
            .getRoomSession(c.getRoom(r)).memberships.map(m=>m.deviceId)}""",room)
        assert len(members)==2 and self.main_id in members and self.other_id not in members
        print(f"Answer on {self.main_id}: {self.other_id} showed 'answered on another device', "
              f"0 mic opens, no ringtone stream, ≥600 ms silence ({elapsed:.1f}s after click; {remaining/1000:.1f}s before expiry).",flush=True)
