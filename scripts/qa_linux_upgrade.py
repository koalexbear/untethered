"""Install and upgrade two real packages against an untouched, disposable XDG profile."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import sqlite3
import subprocess as sp
from contextlib import ExitStack, closing
from urllib.parse import quote, unquote, urlsplit

from playwright.sync_api import sync_playwright
from qa_kwin_element import element_server, login_element
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_replies import screen_words
from qa_upload_cancel import slow_gateway


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f,"sha256").hexdigest()


class Installation:
    def __init__(self,tmp):
        # Root only inside an unprivileged namespace; never use host pacman paths.
        uidmap=Path("/proc/self/uid_map").read_text().split()
        assert len(uidmap)==3 and uidmap[0]=="0" and int(uidmap[1])>0 and uidmap[2]=="1"
        assert os.getuid()==0 and {name for _,name in socket.if_nameindex()}=={"lo"},"use unshare -Urn"
        root,db,cache,hooks=[tmp/p for p in ("root","db","cache","hooks")]
        for p in (root,db,cache,hooks):p.mkdir()
        cfg=tmp/"pacman.conf";cfg.write_text("[options]\nSigLevel = Never\n")
        self.log=tmp/"pacman.log"
        self.cmd=["pacman","--config",cfg,"--root",root,"--dbpath",db,"--cachedir",cache,
                  "--hookdir",hooks,"--logfile",self.log,"--noconfirm"]
        self.exe=root/"usr/bin/untethered"
        self.engine=root/"usr/lib/untethered/gomuks"

    def install(self,pkg):
        # Test root contains only this package; the GUI uses host system libraries.
        sp.run([*self.cmd,"-Udd","--noscriptlet",pkg],check=True,stdout=sp.DEVNULL)
        name,ver=sp.check_output([*self.cmd,"-Q","untethered"],text=True).split()
        assert name=="untethered" and self.engine.is_file()
        return ver,sha(self.exe)


def close_helper(tmp):
    # With bare Xvfb there is no WM to send the normal close request for us.
    source=r"""#include <X11/Xlib.h>
#include <stdlib.h>
int main(int argc,char **argv) {
    Display *d=XOpenDisplay(NULL); if (!d || argc!=2) return 1;
    XEvent e={0}; Window w=strtoul(argv[1],NULL,10);
    e.xclient.type=ClientMessage; e.xclient.window=w; e.xclient.format=32;
    e.xclient.message_type=XInternAtom(d,"WM_PROTOCOLS",False);
    e.xclient.data.l[0]=XInternAtom(d,"WM_DELETE_WINDOW",False);
    if (!XSendEvent(d,w,False,NoEventMask,&e)) return 1;
    XSync(d,False); XCloseDisplay(d); return 0;
}"""
    helper=tmp/"close-window"
    sp.run(["cc","-x","c","-","-o",helper,"-lX11"],input=source,text=True,check=True)
    return helper


def check_upgrade(tmp,hs,port,backend,seed,http,old_pkg,new_pkg):
    pkg=Installation(tmp)
    old_ver,old_hash=pkg.install(old_pkg)
    close_window=close_helper(tmp)
    with ExitStack() as stack:
        paths=[]
        gateway,_=stack.enter_context(slow_gateway(hs,port,observe_path=lambda method,path:paths.append((method,unquote(path)))))
        alice,ah=seed(tmp,gateway)
        other=tmp/"peer";other.mkdir()
        bob,bh=seed(other,hs)
        base=hs+"/_matrix/client/v3"
        who=lambda headers:http("GET",base+"/account/whoami",headers=headers).json()
        alice_id,bob_id=who(ah)["user_id"],who(bh)["user_id"]
        assert alice_id!=bob_id
        room=alice["room"];api=base+"/rooms/"+quote(room,safe="")
        http("POST",api+"/invite",headers=ah,json={"user_id":bob_id})
        http("POST",api+"/join",headers=bh,json={})
        env=isolated_env(tmp,backend,"x11")
        for k in ("UNTETHERED_PROFILE","UNTETHERED_BACKEND"):env.pop(k)
        profile=Path(env["XDG_DATA_HOME"])/"untethered-iced"
        assert not profile.exists()
        db=profile/"data/gomuks.db"
        def sql(query,*args):
            with closing(sqlite3.connect(db.as_uri()+"?mode=ro",uri=True)) as c:
                return c.execute(query,args).fetchall()
        def local(eid):
            return sql("SELECT type,json_extract(content,'$.session_id'),decrypted_type,json_extract(decrypted,'$.body') "
                       "FROM event WHERE room_id=? AND event_id=?",room,eid)
        def has_session(sid):
            return sql("SELECT count(*) FROM crypto_megolm_inbound_session WHERE room_id=? AND session_id=? "
                       "AND session IS NOT NULL",room,sid)==[(1,)]
        def identity():
            [(user,device,token)]=sql("SELECT user_id,device_id,access_token FROM account")
            actual=who({"Authorization":"Bearer "+token})
            assert user==actual["user_id"]==alice_id and device==actual["device_id"]
            devices=http("POST",base+"/keys/query",headers=bh,json={"device_keys":{user:[device]}}).json()
            record=devices["device_keys"][user][device]
            assert record["user_id"]==user and record["device_id"]==device
            assert sql("SELECT device_id FROM crypto_account")==[(device,)]
            keys={alg:record["keys"][alg+":"+device] for alg in ("ed25519","curve25519")}
            assert all(re.fullmatch(r"[A-Za-z0-9+/]{43}",v) for v in keys.values())
            assert sql("SELECT signing_key,identity_key FROM crypto_device WHERE user_id=? AND device_id=?",
                       user,device)==[(keys["ed25519"],keys["curve25519"])]
            return {"device_id":device,**keys}
        start_display(stack,tmp,env,"x11")
        def launch():
            proc=stack.enter_context(child([pkg.exe],env))
            def visible():
                assert proc.poll() is None,"packaged GUI exited"
                p=sp.run(["xdotool","search","--all","--onlyvisible","--pid",str(proc.pid),"--name","Untethered"],
                         env=env,text=True,capture_output=True)
                return p.stdout.splitlines()[-1] if p.returncode==0 else None
            window=wait_for("packaged Untethered window",visible,35)
            sp.run(["xdotool","windowsize",window,"1180","760","windowfocus",window],env=env,check=True)
            assert Path(f"/proc/{proc.pid}/exe").resolve()==pkg.exe
            def engine_pid():
                for children in Path(f"/proc/{proc.pid}/task").glob("*/children"):
                    for pid in children.read_text().split():
                        if Path(f"/proc/{pid}/exe").resolve()==pkg.engine:return pid
            return proc,window,wait_for("packaged gomuks child",engine_pid)
        def words():
            return screen_words(sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6))
        def point(word):
            return wait_for("visible "+word,lambda:next(((x,y) for w,x,y in words() if w==word),None),35)
        def click(x,y):sp.run(["xdotool","mousemove","--window",win,str(x),str(y),"click","1"],env=env,check=True)
        def enter(value):
            sp.run(["xdotool","type","--clearmodifiers","--file","-"],input=value,text=True,env=env,check=True)
        def submit():sp.run(["xdotool","key","Return"],env=env,check=True)
        def shown(body):
            a,b=body.lower().split()
            def visible():
                ws=words()
                return any(0<x2-x<150 and abs(y2-y)<5 for w,x,y in ws if w==a and x>280
                           for v,x2,y2 in ws if v==b)
            return wait_for("cleartext "+body,visible,35)
        def devices():return {d["device_id"] for d in http("GET",base+"/devices",headers=ah).json()["devices"]}
        prior_devices=devices()
        proc,win,old_engine=launch()
        # Sign in and verify on the old installed build itself. Never copy the seed profile.
        click(162,728)
        point("welcome")
        for y,value in [(333,gateway),(391,alice["username"]),(447,alice["password"])]:
            click(510,y);enter(value)
        submit()
        point("verify")
        click(*point("passphrase"));enter(alice["key"]);submit()
        point("media-proof")
        before=identity()
        roster=devices()
        assert roster==prior_devices|{before["device_id"]} and before["device_id"] not in prior_devices
        inode=db.stat().st_ino
        web=stack.enter_context(element_server(hs));pw=stack.enter_context(sync_playwright())
        browser=pw.chromium.launch(executable_path=os.getenv("CHROMIUM") or shutil.which("chromium"),env=env,
                                   args=["--disable-dev-shm-usage"])
        stack.callback(browser.close)
        ctx=browser.new_context(viewport={"width":1280,"height":920},locale="en-US")
        ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"),lambda r:r.abort())
        peer=login_element(ctx,web,bob,room)
        def send(body):
            eid=peer.evaluate("async ([r,b])=>(await window.mxMatrixClientPeg.get().sendTextMessage(r,b)).event_id",[room,body])
            event=http("GET",api+"/event/"+quote(eid,safe=""),headers=ah).json()
            c=event["content"]
            assert event["sender"]==bob_id and event["type"]=="m.room.encrypted" and c["algorithm"]=="m.megolm.v1.aes-sha2"
            assert c["ciphertext"] and body not in json.dumps(event)
            return eid,c["session_id"]
        warm,sid=send("Copper orchard");shown("Copper orchard")
        assert local(warm)==[("m.room.encrypted",sid,"m.room.message","Copper orchard")] and has_session(sid)
        print("Before:",old_ver,old_hash,json.dumps(before),flush=True)
        sp.run([close_window,win],env=env,check=True)
        assert proc.wait(timeout=12)==0
        wait_for("old engine closed",lambda:not Path(f"/proc/{old_engine}").exists())
        peer_key_sends=[]
        peer.on("request",lambda r:peer_key_sends.append(r.method) if "/sendToDevice/" in unquote(urlsplit(r.url).path) else None)
        old,same_sid=send("Amber lighthouse")
        assert same_sid==sid and local(old)==[] and has_session(sid)
        new_ver,new_hash=pkg.install(new_pkg)
        assert int(sp.check_output(["vercmp",new_ver,old_ver]))>0 and new_hash!=old_hash
        upgrade=f"upgraded untethered ({old_ver} -> {new_ver})"
        assert upgrade in pkg.log.read_text() and db.stat().st_ino==inode and local(old)==[]
        print("pacman:",upgrade,"; pre-upgrade event",old,"absent from the stopped profile.",flush=True)
        paths.clear()
        proc,win,_=launch()
        shown("Amber lighthouse")
        shown("Copper orchard")
        after=identity()
        assert before==after and devices()==roster and db.stat().st_ino==inode
        assert local(old)==[("m.room.encrypted",sid,"m.room.message","Amber lighthouse")]
        peer.evaluate("true")  # process queued Playwright request events
        assert not peer_key_sends,"sender re-shared a key before old ciphertext was read"
        assert not any(method=="GET" and "/room_keys/keys" in path for method,path in paths)
        print("After: ",new_ver,new_hash,json.dumps(after),flush=True)
        print("Upgraded UI decrypted historical 'Amber lighthouse' with the retained session; no backup fetch or key resend.",flush=True)
        peer.evaluate("async r=>await window.mxMatrixClientPeg.get().getCrypto().forceDiscardSession(r)",room)
        fresh,new_sid=send("Violet compass")
        assert fresh!=old and new_sid!=sid
        shown("Violet compass")
        assert local(fresh)==[("m.room.encrypted",new_sid,"m.room.message","Violet compass")] and has_session(new_sid)
        assert not any(m=="POST" and p.endswith("/login") for m,p in paths) and devices()==roster
        print("Fresh Element event",fresh,"decrypted as 'Violet compass' in a new Megolm session; zero upgrade logins or added devices.",flush=True)
