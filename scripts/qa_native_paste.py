"""Drive real Ctrl+V and Send in a disposable desktop; see test-media-interop.py."""
import base64
import hashlib
import io
import json
import os
import select
import signal
import sqlite3
import subprocess as sp
import time
from contextlib import ExitStack, contextmanager
from pathlib import Path
from urllib.parse import quote

import requests
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from PIL import Image, ImageDraw


def wait_for(description, check, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        result = check()
        if result:
            return result
        time.sleep(.12)
    raise RuntimeError(f"Timed out waiting for {description}")


@contextmanager
def child(args, env, **kw):
    stderr=kw.pop("stderr",None if os.getenv("PASTE_QA_DEBUG") else sp.DEVNULL)
    proc = sp.Popen(args, env=env, stdout=kw.pop("stdout",sp.DEVNULL), stderr=stderr, start_new_session=True, **kw)
    try:
        yield proc
    finally:
        try:
            os.killpg(proc.pid,signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            proc.wait(timeout=4)
        except sp.TimeoutExpired:
            os.killpg(proc.pid,signal.SIGKILL)
            proc.wait()


def isolated_env(tmp,backend,platform):
    env = os.environ.copy()
    for k in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "SWAYSOCK", "XAUTHORITY",
              "DBUS_SESSION_BUS_ADDRESS", "SESSION_MANAGER"):
        env.pop(k, None)
    for k in ("HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        folder = tmp/k
        folder.mkdir(mode=0o700)
        env[k] = str(folder)
    env.update(XDG_SESSION_TYPE=platform, DBUS_SESSION_BUS_ADDRESS="unix:path="+env["XDG_RUNTIME_DIR"]+"/bus", UNTETHERED_BACKEND=str(backend),
               UNTETHERED_PROFILE=str(tmp/"seed"))
    return env


def start_display(stack,tmp,env,platform,width=1180,height=760):
    runtime = Path(env["XDG_RUNTIME_DIR"])
    r,w=os.pipe()
    try:
        stack.enter_context(child(["Xvfb","-displayfd",str(w),"-screen","0",f"{max(1280,width)}x{max(900,height)}x24",
                                   "-noreset","-nolisten","tcp"],env,pass_fds=(w,)))
        assert select.select([r],[],[],10)[0], "private X server failed"
        # Read through newline before closing: Xvfb writes it separately.
        with os.fdopen(r,"rb",closefd=False) as ready_pipe:
            env["DISPLAY"]=":"+str(int(ready_pipe.readline(32)))
    finally:
        os.close(r);os.close(w)
    wait_for("private X display accepting connections",lambda: sp.run(
        ["xdpyinfo"],env=env,stdout=sp.DEVNULL,stderr=sp.DEVNULL,timeout=3).returncode==0)
    if platform=="wayland":
        conf=tmp/"sway.conf"
        conf.write_text(f"output * mode {width}x{height}\nseat seat0 fallback true\nxwayland disable\ndefault_border none\n")
        env.update(WLR_BACKENDS="x11",WLR_RENDERER="pixman")
        sway=stack.enter_context(child(["sway","-c",str(conf)],env))
        def sway_socket():
            assert sway.poll() is None, "private sway compositor failed to start"
            return next(runtime.glob("sway-ipc.*.sock"),None)
        env["SWAYSOCK"]=str(wait_for("isolated sway",sway_socket))
        env["WAYLAND_DISPLAY"]=wait_for("Wayland socket",lambda:next((s.name for s in runtime.glob("wayland-*") if s.is_socket()),None))


def check_paste(tmp, root, backend, f, headers, platform):
    env=isolated_env(tmp,backend,platform)
    def run(*args, text=True):
        return sp.check_output(args, env=env, text=text, stderr=sp.DEVNULL, timeout=10)

    def room_events():
        r = requests.get(f["server"]+"/_matrix/client/v3/rooms/"+quote(f["room"], safe="")+"/messages",
                         headers=headers, params={"dir":"b","limit":80}, timeout=5)
        r.raise_for_status()
        return [e for e in r.json()["chunk"] if e["type"] in ("m.room.encrypted","m.room.message")]

    def media_count():
        with sqlite3.connect((tmp/"homeserver.db").as_uri()+"?mode=ro", uri=True) as db:
            return db.execute("select count(*) from local_media_repository where media_length is not null").fetchone()[0]

    def screenshot():
        args = ("grim", "-") if platform=="wayland" else ("import","-window","root","png:-")
        im=Image.open(io.BytesIO(run(*args, text=False))).convert("RGB")
        if dest:=os.getenv("PASTE_QA_DEBUG"):
            im.save(Path(dest)/f"{platform}.png")
        return im

    def colored_in_tray():
        im = screenshot().crop((285,510,1180,670))
        counts = im.getcolors(im.width*im.height)
        red = sum(n for n,(r,g,b) in counts if r>230 and g<25 and 96<b<120)
        green = sum(n for n,(r,g,b) in counts if r<28 and g>216 and 170<b<195)
        return min(red,green)>1200

    def clip(stack, mime, contents):
        path = tmp/("clipboard-"+mime.replace("/","-"))
        path.write_bytes(contents)
        args = (["wl-copy","--foreground","--type",mime] if platform=="wayland" else
                ["xclip","-selection","clipboard","-target",mime,"-in","-quiet"])
        with path.open("rb") as src:
            proc = stack.enter_context(child(args, env, stdin=src))
        # Check MIME ownership before injecting any key into the client.
        query=(["wl-paste","--list-types"] if platform=="wayland" else ["xclip","-selection","clipboard","-t","TARGETS","-o"])
        wait_for("clipboard offer",lambda: mime in sp.run(query,env=env,capture_output=True,text=True,timeout=3).stdout)
        assert proc.poll() is None

    def decode64(s):
        return base64.urlsafe_b64decode(s+"="*(-len(s)%4))

    with ExitStack() as stack:
        start_display(stack,tmp,env,platform)
        client=stack.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
        def ready():
            assert client.poll() is None, "native GUI exited"
            if platform=="wayland":
                def find(node):
                    return ("media-proof" in (node.get("name") or "") and node.get("shell")=="xdg_shell") or any(find(n) for n in node.get("nodes",[])+node.get("floating_nodes",[]))
                return find(json.loads(run("swaymsg","-t","get_tree")))
            proc=sp.run(["xdotool","search","--onlyvisible","--name","media-proof"],env=env,capture_output=True,text=True)
            if proc.returncode==0:
                run("xdotool","windowfocus",proc.stdout.splitlines()[-1])
                return True
            return False
        wait_for("verified Matrix room in native window",ready,35)
        initial_events={e["event_id"] for e in room_events()}
        initial_media=media_count()

        original=Image.new("RGB",(360,160),(251,12,108))
        ImageDraw.Draw(original).rectangle((180,0,359,159),fill=(9,233,183))
        png=io.BytesIO();original.save(png,format="PNG")
        # Paste normal text first. It must remain the caption of the one eventual event.
        caption="Clipboard screenshot proof"
        run("xdotool","mousemove","550","697","click","1")
        with ExitStack() as owner:
            clip(owner,"text/plain" if platform=="wayland" else "UTF8_STRING",caption.encode())
            run("xdotool","key","--clearmodifiers","ctrl+v")
            wait_for("pasted caption",lambda: caption.lower() in sp.run(
                ["tesseract","stdin","stdout","--psm","7"], input=_png(screenshot().crop((335,668,900,728)).resize((1130,120))),
                capture_output=True).stdout.decode().lower())
        with ExitStack() as owner:
            clip(owner,"image/png",png.getvalue())
            run("xdotool","key","--clearmodifiers","ctrl+v")
            wait_for("native image preview pixels",colored_in_tray)
        assert {e["event_id"] for e in room_events()}==initial_events and media_count()==initial_media
        print(f"{platform}: actual Ctrl+V staged screenshot and preserved text; before Send: 0 uploads, 0 message events.",flush=True)
        run("xdotool","mousemove","1140","698","click","1")
        def delivered():
            events=[e for e in room_events() if e["event_id"] not in initial_events]
            assert len(events)<=1
            return events[0] if events and not colored_in_tray() else None
        event=wait_for("one confirmed encrypted attachment and cleared draft",delivered,35)
        assert event["type"]=="m.room.encrypted" and media_count()-initial_media==1
        # Read metadata only from this fixture's device; independently retrieve
        # the stored encrypted object and verify its hash and decoded PNG pixels.
        with sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True) as db:
            content=json.loads(db.execute("select decrypted from event where event_id=?",(event["event_id"],)).fetchone()[0])
        assert content["msgtype"]=="m.image" and content["body"]==caption
        assert content["filename"].startswith("Screenshot-") and content["filename"].endswith(".png")
        file=content["file"]
        server,media_id=file["url"][6:].split("/",1)
        resp=requests.get(f["server"]+f"/_matrix/client/v1/media/download/{quote(server,safe='')}/{quote(media_id,safe='')}",headers=headers,timeout=15)
        resp.raise_for_status()
        assert hashlib.sha256(resp.content).digest()==decode64(file["hashes"]["sha256"])
        aes=Cipher(algorithms.AES(decode64(file["key"]["k"])),modes.CTR(decode64(file["iv"]))).decryptor()
        result=Image.open(io.BytesIO(aes.update(resp.content)+aes.finalize())).convert("RGB")
        assert result.size==original.size and result.tobytes()==original.tobytes()
        assert len([e for e in room_events() if e["event_id"] not in initial_events])==1
        print(f"{platform}: Send: 1 upload, 1 encrypted room event, matching caption and 360×160 PNG pixels.",flush=True)
        if platform=="wayland":
            run("swaymsg","kill")
            client.wait(timeout=6)


def _png(im):
    out=io.BytesIO();im.save(out,"PNG");return out.getvalue()
