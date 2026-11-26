"""Real Dunst popup + wire-level absence on a private X display and session bus."""
import io
import json
import os
import shutil
import subprocess as sp
from pathlib import Path
from urllib.parse import quote

from PIL import Image
import qa_events
from qa_ui import CHAT_X
from qa_native_paste import child,wait_for


class NotificationDesktop:
    def __init__(self,stack,tmp,env):
        self.env=env
        address="unix:path="+env["XDG_RUNTIME_DIR"]+"/bus"
        assert env["DBUS_SESSION_BUS_ADDRESS"]==address
        env["DBUS_SYSTEM_BUS_ADDRESS"]="unix:path="+env["XDG_RUNTIME_DIR"]+"/no-system-bus"
        stack.enter_context(child(["dbus-daemon","--session","--nofork","--nopidfile","--address="+address],env))
        wait_for("test session bus",lambda:Path(env["XDG_RUNTIME_DIR"],"bus").is_socket())
        self.log=tmp/"alerts-bus.log"
        output=stack.enter_context(self.log.open("wb"))
        stack.enter_context(child(["stdbuf","-oL","dbus-monitor","--session",
            "type='method_call',interface='org.freedesktop.Notifications',member='Notify'"],env,stdout=output))
        dunst=os.environ.get("DUNST") or shutil.which("dunst")
        assert dunst,"Set DUNST to the QA dunst executable"
        self.ctl=str(Path(dunst).with_name("dunstctl"))
        conf=tmp/"dunstrc"
        conf.write_text('[global]\norigin=top-right\nwidth=520\nfont=Liberation Sans 16\n'
                        'format="<b>%s</b>\\n%b"\nicon_position=off\nstack_duplicates=false\n'
                        '[urgency_normal]\ntimeout=0\n')
        service=stack.enter_context(child([dunst,"-conf",str(conf)],env))
        def owner():
            r=sp.run(["busctl","--address="+address,"call","org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus",
                      "GetConnectionUnixProcessID","s","org.freedesktop.Notifications"],env=env,capture_output=True,text=True)
            return r.returncode==0 and r.stdout.strip()==f"u {service.pid}"
        wait_for("private Dunst owns the notification service",owner)
        stack.enter_context(child(["xmessage","-title","QA background window","-geometry","240x90+10+660","Focus target"],env))
        self.other=wait_for("private focus target",lambda:sp.run(["xdotool","search","--name","^QA background window$"],
            env=env,text=True,capture_output=True).stdout.strip())

    def run(self,*args):
        return sp.check_output(args,env=self.env,text=True,timeout=6).strip()

    def focus(self,w):
        self.run("xdotool","windowfocus","--sync",w)
        assert self.run("xdotool","getwindowfocus")==w

    def count(self):
        return self.log.read_text().count("member=Notify")

    def popup(self,text,appname="Untethered",activate=False,on_visible=None):
        def visible():
            if self.run(self.ctl,"count","displayed")!="1":return False
            win=self.run("xdotool","search","--onlyvisible","--name","^Dunst$")
            if not win:return False
            geom=dict(line.split("=",1) for line in self.run("xdotool","getwindowgeometry","--shell",win).splitlines())
            x,y,w,h=(int(geom[k]) for k in ("X","Y","WIDTH","HEIGHT"))
            raw=sp.check_output(["import","-window","root","png:-"],env=self.env,timeout=5)
            im=Image.open(io.BytesIO(raw)).crop((x,y,x+w,y+h))
            buf=io.BytesIO();im.resize((2*w,2*h)).save(buf,"PNG")
            ocr=sp.check_output(["tesseract","stdin","stdout","--psm","6"],input=buf.getvalue(),stderr=sp.DEVNULL).decode()
            return text.lower() in " ".join(ocr.lower().split())
        wait_for("visible Dunst popup: "+text,visible,15)
        if on_visible:on_visible()
        self.run(self.ctl,"action" if activate else "close-all")
        wait_for("archived desktop notification",lambda:self.run(self.ctl,"count","displayed")=="0")
        history=json.loads(self.run(self.ctl,"history"))["data"][0]
        assert any(n["appname"]["data"]==appname and text in n["body"]["data"] for n in history),history
        print(f'Dunst displayed and archived "{text}".',flush=True)


def check_room_alerts(tmp,hs,page,room,native,hdr,http,desk,locate,click,restart,window,ui):
    me="@"+native["username"]+":test.invalid"
    foreign=page.evaluate("()=>window.mxMatrixClientPeg.get().getSafeUserId()")
    assert me!=foreign
    other=page.evaluate("""async uid=>(await window.mxMatrixClientPeg.get().createRoom({
        name:"zz-control",preset:"private_chat",invite:[uid],initial_state:[{type:"m.room.encryption",
        state_key:"",content:{algorithm:"m.megolm.v1.aes-sha2"}}]})).room_id""",me)
    http("POST",hs+"/_matrix/client/v3/rooms/"+quote(other,safe="")+"/join",headers=hdr,json={})
    page.wait_for_function("([r,u])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(u)?.membership==='join'",arg=[other,me])
    def send(r,body,mention=None):
        eid=page.evaluate("""async ([r,body,id])=>(await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
            body,msgtype:'m.text','m.mentions':id?{user_ids:[id]}:{}})).event_id""",[r,body,mention])
        wire=http("GET",hs+"/_matrix/client/v3/rooms/"+quote(r,safe="")+"/event/"+quote(eid,safe=""),headers=hdr).json()
        assert wire["type"]=="m.room.encrypted" and "ciphertext" in wire["content"]
        # The device's own push rule evaluation of the decrypted event.
        e=wait_for("native decoded "+body,lambda:qa_events.event(tmp/"seed",r,eid),15)
        assert e["encrypted"] and e["content"]["body"]==body,e
        assert e["content"]["m.mentions"].get("user_ids",[])==([mention] if mention else [])
        return e["notify"]
    def words(ws,w):return [(x,y) for v,x,y in ws if x>CHAT_X and v==w]
    def panel():
        # The bell icon in the channel header.
        click(wait_for("Notification Settings header control",lambda:ui.header("notification settings"),20))
    def option(w):
        def find(ws):
            # The row of mode buttons contains Account, All and Mute.
            for x,y in words(ws,w):
                if all(any(abs(yy-y)<5 for _,yy in words(ws,other)) for other in ("account","all","mute")):return x,y
        click(locate("room setting: "+w,find))
    def setting(w):
        return locate("active room setting: "+w,lambda ws:any(
            any(abs(yy-y)<5 for _,yy in words(ws,w)) for _,y in words(ws,"room")))
    def rules():
        return {r["rule_id"]:r for r in http("GET",hs+"/_matrix/client/v3/pushrules/",headers=hdr).json()["global"]["room"]}
    panel();setting("default");option("all");setting("all")
    assert rules()[room]["actions"]==["notify"] and rules()[room]["enabled"]
    desk.focus(desk.other)
    assert send(room,"Baseline lantern")
    desk.popup("Baseline lantern")
    desk.focus(window());option("mute");setting("mute")
    assert rules()[room]["actions"]==[] and rules()[room]["enabled"] and other not in rules()
    desk.focus(desk.other)
    assert not send(room,"Quiet marble")
    assert not send(room,"Different recipient",foreign)
    assert send(room,"Direct beacon",me)
    desk.popup("Direct beacon")
    assert desk.count()==2
    print("Same muted encrypted room: ordinary and other-user mention delivered to timeline, 0 Notify calls; direct mention produced 1.",flush=True)
    assert send(other,"Control meadow")
    desk.popup("Control meadow")
    assert desk.count()==3
    restart();panel();setting("mute")
    assert send(room,"Already reading",me)
    locate("mention rendered in the focused room",lambda ws:words(ws,"already") and words(ws,"reading"))
    desk.focus(desk.other)
    assert not send(room,"Still quiet")
    assert send(room,"Restored beacon",me)
    desk.popup("Restored beacon")
    assert desk.count()==4 and desk.run(desk.ctl,"count","history")=="4"
    assert not any(s in desk.log.read_text() for s in ("Quiet marble","Different recipient","Already reading","Still quiet"))
    desk.focus(window());option("account");setting("default")
    assert room not in rules()
    print("Untouched control room notified. Restart preserved mute with no replay; focused room stayed quiet. "
          "Exactly 4 D-Bus Notify calls and 4 archived popups. Account default removed the override.",flush=True)
