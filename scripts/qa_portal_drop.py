"""Prove portal-only Wayland DnD using real GTK, document portal and Synapse."""
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import re
import signal
import subprocess as sp
import threading
import time
from contextlib import ExitStack
from urllib.parse import quote

import requests
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from PIL import Image

import qa_events
from qa_native_paste import child, isolated_env, start_display, wait_for
from qa_upload_cancel import slow_gateway


def test(tmp, hs, port, root, backend, seed):
    env=isolated_env(tmp,backend,"wayland")
    env.update(GDK_BACKEND="wayland",GTK_A11Y="none",GSK_RENDERER="cairo",GIO_USE_VFS="local",XDG_CURRENT_DESKTOP="sway")
    gate=threading.Event()
    def run(*args,text=True):
        return sp.check_output(args,env=env,text=text,timeout=10,stderr=sp.DEVNULL)
    def events(room):
        resp=requests.get(hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")+"/messages",
                          headers=headers,params={"dir":"b","limit":60},timeout=6)
        resp.raise_for_status()
        return [e for e in resp.json()["chunk"] if e["type"] in ("m.room.encrypted","m.room.message")]
    def window(name):
        nodes=[json.loads(run("swaymsg","-t","get_tree"))]
        while nodes:
            n=nodes.pop()
            if n.get("shell")=="xdg_shell" and name in (n.get("name") or ""):return n["rect"]
            nodes.extend(n.get("nodes",[])+n.get("floating_nodes",[]))
    def click(x,y):run("xdotool","mousemove",str(x),str(y),"click","1")
    def cards(rect):
        im=Image.open(io.BytesIO(run("grim","-",text=False)))
        x,y=rect["x"],rect["y"]+rect["height"]-265
        buf=io.BytesIO()
        im.crop((x+284,y,x+650,y+177)).resize((732,354)).save(buf,"PNG")
        return sp.run(["tesseract","stdin","stdout","--psm","6"],input=buf.getvalue(),capture_output=True,
                      check=True,timeout=5).stdout.decode().lower()

    with slow_gateway(hs,port,hold_first=gate) as (gateway,uploads), ExitStack() as stack:
        # Always release the fixture's upstream request during exception cleanup.
        stack.callback(gate.set)
        f,headers=seed(tmp,gateway)
        decoy=requests.post(hs+"/_matrix/client/v3/createRoom",headers=headers,json={"name":"zz-other",
            "preset":"private_chat","initial_state":[{"type":"m.room.encryption","state_key":"",
            "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]},timeout=10)
        decoy.raise_for_status();other=decoy.json()["room_id"];room=f["room"]
        png=tmp/"Portal shot.png";pdf=tmp/"Two notes.pdf"
        im=Image.new("RGB",(200,96),(22,197,129));im.save(png);im.save(pdf,"PDF")
        originals={p.name:p.read_bytes() for p in (png,pdf)}
        start_display(stack,tmp,env,"wayland",width=2000,height=760)
        env["DBUS_SESSION_BUS_ADDRESS"]="unix:path="+env["XDG_RUNTIME_DIR"]+"/bus"
        stack.enter_context(child(["dbus-daemon","--session","--nofork","--nopidfile","--address="+env["DBUS_SESSION_BUS_ADDRESS"]],env))
        wait_for("private D-Bus socket",lambda:(Path(env["XDG_RUNTIME_DIR"])/"bus").is_socket())
        # Only manipulate the portal subprocess that THIS test owns. Never
        # pause or replace the user's active portal or touch their session bus.
        portal=stack.enter_context(child([os.environ.get("DOCUMENT_PORTAL","/usr/lib/xdg-document-portal")],env))
        def portal_ready():
            assert portal.poll() is None,"private xdg-document-portal exited"
            v=sp.run(["gdbus","call","--session","--dest","org.freedesktop.DBus","--object-path","/org/freedesktop/DBus",
                      "--method","org.freedesktop.DBus.GetConnectionUnixProcessID","org.freedesktop.portal.Documents"],env=env,capture_output=True,text=True)
            return v.returncode==0 and re.search(rf"\b{portal.pid}\b",v.stdout)
        wait_for("our private document portal",portal_ready)
        client=stack.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
        wait_for("original encrypted room",lambda:window("media-proof"),35)
        trace=tmp/"sender-wire"
        with trace.open("w") as log:
            sender=stack.enter_context(child([os.environ.get("GTK_PYTHON","/usr/bin/python3"),root/"scripts/qa_portal_sender.py",tmp],
                                          {**env,"WAYLAND_DEBUG":"client"},stderr=log))
        def sender_ready():
            assert sender.poll() is None,"GTK fixture failed: "+trace.read_text(errors="replace")[-900:]
            return (tmp/"source-ready").is_file() and window("Portal-only source")
        src=wait_for("real portal transfer and GTK drag source",sender_ready)
        dst=window("media-proof")
        assert dst and src["x"]!=dst["x"],"test windows must be tiled side by side"
        sx,sy=src["x"]+src["width"]//2,src["y"]+src["height"]//2
        dx,dy=dst["x"]+dst["width"]//2,dst["y"]+dst["height"]//2
        def select_room(index):
            click(dst["x"]+157,dst["y"]+115+index*40)
            return wait_for("selected room",lambda:window("zz-other" if index else "media-proof"))
        # Let Wayland deliver the token, but delay the actual RetrieveFiles
        # answer so the user can switch chats while the drop is outstanding.
        run("xdotool","mousemove",str(sx),str(sy))
        wait_for("pointer entered the GTK source",lambda:re.search(r"wl_pointer[#@]\d+\.enter\(",trace.read_text(errors="replace")))
        run("xdotool","mousedown","1")
        # Cross the toolkit's drag threshold within the source window before
        # crossing the boundary, and wait for protocol negotiation explicitly.
        for delta in [3,8,16,28]:
            run("xdotool","mousemove",str(sx+delta),str(sy));time.sleep(.09)
        wait_for("Wayland start_drag",lambda:".start_drag(" in trace.read_text(errors="replace"))
        for i in range(1,25):
            run("xdotool","mousemove",str(sx+28+(dx-sx-28)*i//24),str(sy+(dy-sy)*i//24));time.sleep(.03)
        mime="application/vnd.portal.filetransfer"
        wait_for("receiver accepted portal MIME",lambda:re.search(
            rf'wl_data_source[#@]\d+\.target\("{re.escape(mime)}"',trace.read_text(errors="replace")))
        os.kill(portal.pid,signal.SIGSTOP)
        try:
            run("xdotool","mouseup","1")
            send=wait_for("portal-only payload request",lambda: re.search(
                rf'wl_data_source([#@]\d+)\.send\("{re.escape(mime)}"',trace.read_text(errors="replace")))
            obj=re.escape(send.group(1))
            offered=re.findall(rf'wl_data_source{obj}\.offer\("([^"]+)"',trace.read_text(errors="replace"))
            # GTK may also add its process-local drag identity marker. It
            # carries no file data and is unusable by the separate Rust client.
            assert set(offered)-{"application/x-gtk-local-dnd"}=={mime},f"unexpected advertised types: {offered}"
            select_room(1)
            assert not re.search(rf'wl_data_source{obj}\.dnd_finished\(',trace.read_text(errors="replace")), "drop acknowledged before retrieval"
        finally:
            os.kill(portal.pid,signal.SIGCONT)
        wait_for("portal retrieved and grant closed by sender",lambda:(tmp/"source-finished").is_file())
        assert all((tmp/n).read_bytes()!=expected for n,expected in originals.items())
        assert "ready" not in cards(dst),"staged in the room selected during D-Bus resolution"
        select_room(0)
        wait_for("two attachment cards in the original room",lambda:cards(dst).count("ready")==2)
        assert not uploads and not events(room) and not events(other)
        print("Portal-only Wayland drop: 1 portal MIME, no URI list; switched rooms before RetrieveFiles finished; 2 files staged in original room, 0 uploads, 0 message events.",flush=True)
        click(dst["x"]+dst["width"]-46,dst["y"]+dst["height"]-62)
        wait_for("first upload at gate",lambda:uploads)
        select_room(1);gate.set()
        accepted=wait_for("both encrypted sends",lambda:len(events(room))>=2 and events(room),35)
        assert len(accepted)==2 and not events(other)
        assert len(uploads)==2 and all(u["received"]==u["expected"] and not u["partial"] for u in uploads)
        found={}
        for event in accepted:
            assert event["type"]=="m.room.encrypted"
            content=wait_for("the native device decrypted its event",lambda:qa_events.event(tmp/"seed",room,event["event_id"]))["content"]
            assert content["filename"] not in found
            desc=content["file"];server,mid=desc["url"][6:].split("/",1)
            blob=requests.get(f"{hs}/_matrix/client/v1/media/download/{quote(server,safe='')}/{quote(mid,safe='')}",headers=headers,timeout=10)
            blob.raise_for_status()
            def b64(s):return base64.urlsafe_b64decode(s+"="*(-len(s)%4))
            assert hashlib.sha256(blob.content).digest()==b64(desc["hashes"]["sha256"])
            dec=Cipher(algorithms.AES(b64(desc["key"]["k"])),modes.CTR(b64(desc["iv"]))).decryptor()
            found[content["filename"]]=dec.update(blob.content)+dec.finalize()
        assert found==originals
        select_room(0)
        wait_for("cleared outbox",lambda:"ready" not in cards(dst))
        assert len(events(room))==2 and not events(other)
        print("Send: switched rooms mid-upload; original encrypted room: 2 uploads, 2 events (PNG and PDF exactly once); other room: 0. Both decrypted originals matched byte-for-byte after source files were replaced.",flush=True)
        run("swaymsg","kill");client.wait(timeout=6)
