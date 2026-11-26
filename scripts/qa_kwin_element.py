"""Dolphin → Untethered → Element Web → Untethered, with two Matrix users."""
import hashlib
import http.server
import json
import os
import re
import shutil
import sqlite3
import threading
import time
from contextlib import ExitStack, contextmanager
from pathlib import Path
from urllib.parse import quote, urlsplit

import requests
from PIL import Image, ImageDraw
from playwright.sync_api import sync_playwright
import qa_events
from qa_kwin import KWin
from qa_native_paste import child, wait_for
from qa_upload_cancel import slow_gateway


@contextmanager
def element_server(hs, *, server_name="test.invalid", rtc_service=None):
    directory=Path(os.environ["ELEMENT_WEB_DIR"]).resolve()
    assert (directory/"index.html").is_file(), "Set ELEMENT_WEB_DIR to an extracted Element Web release"
    config={"default_server_config":{"m.homeserver":{"base_url":hs,"server_name":server_name}},
            "disable_guests":True,"brand":"Element","disable_telemetry":True,"default_federate":False,
            "integrations_ui_url":None,"integrations_rest_url":None,"room_directory":{"servers":[]}}
    if rtc_service:
        config["default_server_config"]["org.matrix.msc4143.rtc_foci"]=[{"type":"livekit","livekit_service_url":rtc_service}]
        config["element_call"]={"use_exclusively":True}
        config["setting_defaults"]={"feature_element_call_react":False}
    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self,*a,**kw):super().__init__(*a, directory=directory, **kw)
        def log_message(self,*args):pass
        def do_GET(self):
            if re.fullmatch(r"/config(?:\.[\w.:-]+)?\.json",urlsplit(self.path).path):
                body=json.dumps(config).encode();self.send_response(200)
                self.send_header("Content-Type","application/json");self.send_header("Content-Length",str(len(body)))
                self.end_headers();self.wfile.write(body)
            else:super().do_GET()
    with http.server.ThreadingHTTPServer(("127.0.0.1",0),Handler) as srv:
        th=threading.Thread(target=srv.serve_forever,daemon=True);th.start()
        try:yield f"http://127.0.0.1:{srv.server_port}"
        finally:srv.shutdown();th.join()


def login_element(ctx,web,peer,room):
    page=ctx.new_page()
    page.set_default_timeout(16000)
    page.goto(web+"/#/login")
    page.get_by_role("textbox",name="Username").fill(peer["username"])
    page.get_by_placeholder("Password",exact=True).fill(peer["password"])
    page.get_by_role("button",name="Sign in",exact=True).click()
    page.get_by_role("button",name="Use recovery key",exact=True).click()
    page.get_by_title("Recovery key",exact=True).fill(peer["key"])
    page.get_by_role("button",name="Continue",exact=True).click()
    page.get_by_role("button",name="Done",exact=True).click()
    page.wait_for_url("**/#/home",timeout=30000)
    page.goto(web+"/#/room/"+room)
    page.locator('input[type=file]').first.wait_for(state="attached",timeout=30000)
    return page


def test(tmp,hs,port,root,backend,seed,*,plain=False,web_client=None):
    release=threading.Event()
    with slow_gateway(hs,port,hold_first=release) as (gateway,uploads):
        try:
            round_trip(tmp,hs,gateway,uploads,release,root,backend,seed,plain,web_client)
        finally:
            release.set()


def round_trip(tmp,hs,gateway,uploads,release,root,backend,seed,plain,web_client):
    f,headers=seed(tmp,gateway,encrypted=not plain)
    room=f["room"];alice="@"+f["username"]+":test.invalid"
    peer_dir=tmp/"peer";peer_dir.mkdir()
    peer,peer_hdr=seed(peer_dir,gateway)
    bob="@"+peer["username"]+":test.invalid"
    def matrix(method,path,auth,**kw):
        response=requests.request(method,hs+"/_matrix/client/v3/"+path,headers=auth,timeout=12,**kw)
        response.raise_for_status();return response.json()
    room_path="rooms/"+quote(room,safe="")
    matrix("POST",room_path+"/invite",headers,json={"user_id":bob})
    matrix("POST",room_path+"/join",peer_hdr,json={})
    wire_type="m.room.message" if plain else "m.room.encrypted"
    if plain:
        assert not any(e["type"]=="m.room.encryption" for e in matrix("GET",room_path+"/state",headers))
        assert http_config(hs,headers)==65536
    else:assert matrix("GET",room_path+"/state/m.room.encryption",headers)["algorithm"]=="m.megolm.v1.aes-sha2"
    other=matrix("POST","createRoom",headers,json={"name":"zz-other","preset":"private_chat",
                 "initial_state":[{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]})["room_id"]
    def events(target=room):
        return [e for e in matrix("GET","rooms/"+quote(target,safe="")+"/messages",headers,params={"dir":"b","limit":80})["chunk"]
                if e["type"] in ("m.room.encrypted","m.room.message")]
    sources=tmp/"InteropFiles";sources.mkdir()
    png=sources/"Map.png";pdf=sources/"Plan.pdf"
    img=Image.new("RGB",(240,128),(229,36,112));ImageDraw.Draw(img).rectangle((120,0,239,127),fill=(12,205,158))
    img.save(png);img.save(pdf,"PDF",title="KWin Matrix interoperability proof",
                           creationDate=time.gmtime(0),modDate=time.gmtime(0))
    if plain:
        raw=pdf.read_bytes();i=raw.rindex(b"%%EOF")
        pdf.write_bytes(raw[:i]+b" "*(65536-len(raw))+raw[i:])
        (sources/"Oversize.bin").write_bytes(bytes(65537))
    original={p.name:p.read_bytes() for p in (png,pdf)}
    returned={};receipts={}
    with element_server(gateway) as web, sync_playwright() as pw, ExitStack() as stack:
        browser=pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                   args=["--enable-unsafe-swiftshader","--ignore-gpu-blocklist"])
        stack.callback(browser.close)
        ctx=browser.new_context(viewport={"width":1300,"height":900},locale="en-US")
        ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"),lambda r:r.abort())
        page=login_element(ctx,web,peer,room)
        assert page.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()")==bob!=alice
        version=requests.get(web+"/version",timeout=3)
        version.raise_for_status()
        print(f"Element Web {version.text.strip()} signed in as a distinct Matrix user and verified its device.",flush=True)
        if web_client:
            from qa_plain_web_files import browser_roundtrip
            browser_roundtrip(tmp,hs,browser,f,page,room,alice,bob,headers,uploads,release,events,original,sources,web_client)
            return
        kde=KWin(stack,tmp,root,backend)
        native=wait_for("Untethered joined room",lambda:kde.window("media-proof"),35)
        kde.position(native,0,0,1180,960)
        trace=stack.enter_context((tmp/"dolphin-wire").open("w"))
        dolphin=stack.enter_context(child(["dolphin","--new-window",str(sources)],{**kde.env,"WAYLAND_DEBUG":"client"},stderr=trace))
        dw=wait_for("Dolphin window",lambda:kde.window("InteropFiles.*Dolphin"))
        kde.position(dw,1190,0,900,950)
        def select_room(name):
            kde.command("kdotool","windowactivate",native)
            def locate():
                return next(((x,y) for text,x,y in kde.words((77,107,275,265),6)
                             if name.split("-")[0] in text.lower()),None)
            kde.click(*wait_for("sidebar label "+name,locate))
            wait_for("native window switched to "+name,lambda:kde.window("Untethered.*"+name))
        # Check real clients are managed by this compositor, not a parent compositor.
        assert int(kde.command("kdotool","getwindowpid",native).stdout)==kde.native.pid
        assert int(kde.command("kdotool","getwindowpid",dw).stdout)==dolphin.pid
        kde.command("kdotool","windowactivate",dw)
        kde.keys(("Down","\ue009"),("Down","3"),("Up","3"),("Up","\ue009"))
        def file_label():
            # Isolate the filename column from previews, dates and the root
            # banner some Dolphin configurations show inside a namespace.
            for rect,psm in [((1445,78,1670,340),6),((1412,80,1680,340),11),((1190,70,2090,730),11)]:
                hit=next(((x,y) for text,x,y in kde.words(rect,psm) if text.lower() in ("map.png","plan.pdf","oversize.bin")),None)
                if hit:return hit
        sx,sy=wait_for("file in Dolphin details view",file_label)
        # Focus the file view on empty space. Clicking the filename and then
        # pressing it again to drag can be interpreted as a double-click.
        kde.click(1650,590);kde.select_all()
        kde.drag(sx,sy,820,420)
        def staged():
            words=kde.words((285,690,680,915),6)
            return sum("ready" in t.lower() for t,_,_ in words)==2
        wait_for("two native attachment cards",staged)
        if plain:
            def skipped():
                words=" ".join(w.lower() for w,_,_ in kde.words((282,615,1180,905)))
                return "oversize.bin" in words and "65537" in words and "65536" in words
            wait_for("native names rejected file and exact byte limit",skipped,20)
        wire=(tmp/"dolphin-wire").read_text(errors="replace")
        assert re.search(r'wl_data_source[#@]\d+\.send\("(text/uri-list|application/vnd\.portal\.filetransfer)"',wire)
        assert not uploads and not events()
        with sqlite3.connect((tmp/"homeserver.db").as_uri()+"?mode=ro",uri=True) as db:
            assert db.execute("select count(*) from local_media_repository where media_length is not null").fetchone()[0]==0
        print(kde.command("dolphin","--version").stdout.strip()+": Wayland DnD staged PNG + PDF together; before Send: 0 uploads, 0 messages.",flush=True)
        kde.click(1140,908)  # Send arrow: 1180×960 window, 28 px title bar
        wait_for("first outbound upload at gate",lambda:uploads)
        select_room("zz-other")
        assert not events() and not events(other)
        release.set()
        wait_for("two outbound events",lambda:len(events())>=2)
        outgoing=events()
        assert len(outgoing)==2 and all(e["type"]==wire_type and e["sender"]==alice for e in outgoing)
        assert len(uploads)==2 and not events(other)
        print(f"Switched rooms mid-upload: exactly 2 {wire_type} events in the original room; 0 in the other room.",flush=True)
        # Check the actual remote account's decrypted Element timeline and IDs.
        js="""([room,sender])=>window.mxMatrixClientPeg.get().getRoom(room)?.getLiveTimeline().getEvents()
            .filter(e=>e.getSender()===sender && ['m.image','m.file'].includes(e.getContent()?.msgtype))
            .map(e=>({id:e.getId(),name:e.getContent().filename??e.getContent().body,type:e.getContent().msgtype}))"""
        wait_for("Element received both outgoing messages",lambda:len(page.evaluate(js,[room,alice]) or [])>=2)
        records=page.evaluate(js,[room,alice])
        assert len(records)==2 and {r["name"] for r in records}==set(original)
        assert {r["id"] for r in records}=={e["event_id"] for e in outgoing}
        returned=element_return(tmp,page,original,next(r["id"] for r in records if r["name"]=="Map.png"))
        wait_for("two return events",lambda:len(events())>=4,35)
        all_events=events()
        assert len(all_events)==4 and all(e["type"]==wire_type for e in all_events)
        if plain:check_plain_objects(hs,headers,all_events,original,alice,bob,uploads)
        back_ids={e["event_id"] for e in all_events if e["sender"]==bob}
        assert len(back_ids)==2
        def read_back():
            # Decrypted (or plain) on the native device, as its sync brought them.
            rows={e["event_id"]:e["content"] for e in qa_events.entries(tmp/"seed") if e["room_id"]==room and
                  e["sender"]==bob and e["encrypted"]==(wire_type=="m.room.encrypted") and qa_events.decrypted(e)}
            return rows if len(rows)==2 and all(c.get("msgtype") for c in rows.values()) else None
        rows=wait_for("native device received both return events",read_back)
        assert set(rows)==back_ids
        assert {c.get("filename",c.get("body")) for c in rows.values()}=={p.name for p in returned.values()}
        select_room("media-proof")
        # Wait for both the newly focused room's read marker and both
        # decoded previews: removing the unread divider moves Save buttons.
        user_path="user/"+quote(alice,safe="")+"/rooms/"+quote(room,safe="")
        def read_latest():
            res=requests.get(hs+"/_matrix/client/v3/"+user_path+"/account_data/m.fully_read",headers=headers,timeout=4)
            return res.ok and res.json()["event_id"]==all_events[0]["event_id"]
        wait_for("focused receiver read both returned files",read_latest,15)
        def both_previews():
            im=kde.capture().crop((345,85,672,854))
            return sum(n for n,(r,g,b) in im.getcolors(im.width*im.height)
                       if r>209 and g<54 and 94<b<130)>45000
        wait_for("both image previews laid out",both_previews)
        for idx,(name,src) in enumerate(returned.items()):
            point=wait_for("native Save "+src.name,lambda:kde.save_button(src.name))
            kde.click(*point)
            dialog=wait_for("KDE Save dialog",lambda:kde.window("Save attachment"))
            kde.command("kdotool","windowactivate",dialog)
            dest=tmp/("saved-"+src.name)
            kde.select_all();kde.type(str(dest));kde.keys(("Down","\ue006"),("Up","\ue006"))
            wait_for("native download "+src.name,lambda:dest.is_file() and dest.stat().st_size==len(original[name]),25)
            assert dest.read_bytes()==original[name],f"native download mismatch: {src.name}"
            receipts[name]=hashlib.sha256(dest.read_bytes()).hexdigest()
            if idx==0:
                wait_for("first save acknowledged before locating second button",lambda:any(w.lower().rstrip(".")=="saved"
                    for w,_,_ in kde.words((285,76,1178,161),11)))
        assert len(events())==4 and not events(other) and len(receipts)==2
        print(f"Return from Element: 2 {wire_type} events; native Save downloaded both originals byte-for-byte: "+json.dumps(receipts),flush=True)
        kde.command("kdotool","windowclose",native);kde.native.wait(timeout=8)


def http_config(hs,hdr):
    r=requests.get(hs+"/_matrix/client/v1/media/config",headers=hdr,timeout=5)
    r.raise_for_status();return r.json()["m.upload.size"]


def element_return(tmp,page,original,image_id):
    # Drive the visible download controls, then upload the actual saved
    # bytes under distinct names to distinguish replies from sender echoes.
    returned={name:tmp/("Return"+name) for name in original}
    tile=page.locator(f'[data-event-id="{image_id}"]')
    tile.wait_for()
    try:tile.locator("img:not(.mx_BaseAvatar_image)").last.click(timeout=5500)
    except Exception:
        print("Image tile:",tile.aria_snapshot(),tile.locator("img,button,a").evaluate_all(
            "els=>els.map(e=>[e.tagName,e.className,e.alt||e.title||e.getAttribute('aria-label')])"),flush=True);raise
    viewer=page.get_by_role("dialog",name="Image view")
    viewer.wait_for()
    with page.expect_download() as dl:
        viewer.get_by_role("button",name="Download",exact=True).click()
    dl.value.save_as(returned["Map.png"])
    viewer.get_by_role("button",name="Close",exact=True).press("Enter")
    viewer.wait_for(state="hidden")
    with page.expect_download() as dl:
        page.get_by_role("listitem").filter(has_text="Plan.pdf").get_by_role("button",name="Download",exact=True).click()
    dl.value.save_as(returned["Plan.pdf"])
    for name,path in returned.items():
        assert path.read_bytes()==original[name],f"Element download mismatch: {name}"
        print(f"Element saved {name}: {len(original[name])} bytes, SHA-256 {hashlib.sha256(original[name]).hexdigest()}",flush=True)
        page.locator('input[type=file]').first.set_input_files(str(path))
        page.get_by_role("button",name="Upload",exact=True).click()
    return returned


def check_plain_objects(hs,hdr,room_events,original,alice,bob,uploads):
    expected={(user,("" if user==alice else "Return")+name):raw for user in (alice,bob) for name,raw in original.items()}
    assert len(room_events)==len(expected)==4
    for e in room_events:
        c=e["content"];name=c.get("filename",c["body"])
        assert e["type"]=="m.room.message" and "file" not in c and c["url"].startswith("mxc://")
        assert c["msgtype"]==("m.image" if name.endswith(".png") else "m.file")
        raw=expected.pop((e["sender"],name))
        server,mid=c["url"][6:].split("/",1)
        r=requests.get(f"{hs}/_matrix/client/v1/media/download/{quote(server,safe='')}/{quote(mid,safe='')}",headers=hdr,timeout=10)
        r.raise_for_status()
        assert c["info"]["size"]==len(raw) and r.content==raw
    assert sorted(u["received"] for u in uploads)==sorted([len(raw) for raw in original.values()]*2)
    print("Synapse: four ordinary m.room.message attachments with plain MXC descriptors, "
          "four uploads, authenticated originals byte-identical; no oversized upload or event.",flush=True)
