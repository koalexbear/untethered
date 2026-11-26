"""Acceptance proof: slow a real Matrix upload, cut it off, then retry.

Only the disposable loopback media endpoint is throttled. The shipping clients
use their actual transports. The server counts room events independently.
"""
import base64
import hashlib
import http.client
import io
import json
import os
import re
import socket
import sqlite3
import subprocess
import threading
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import quote, urlsplit

import requests
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from PIL import Image
from playwright.sync_api import sync_playwright


SIZE = 16 * 1024 * 1024


def until(label, fn, seconds=35):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        v = fn()
        if v:
            return v
        time.sleep(.08)
    raise RuntimeError("Timed out waiting for "+label)


@contextmanager
def slow_gateway(upstream,port,hold_first=None,observe_path=None,reject_once=None):
    records = []
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def log_message(self, *args):
            pass
        def forward(self):
            if observe_path:observe_path(self.command,urlsplit(self.path).path)
            self.close_connection = True
            if reject_once and reject_once(self.command,urlsplit(self.path).path):
                body=b'{"errcode":"M_FORBIDDEN","error":"Disposable gateway: link not forwarded"}'
                self.send_response(403);self.send_header("Content-Type","application/json")
                self.send_header("Content-Length",str(len(body)));self.send_header("Access-Control-Allow-Origin","*")
                self.send_header("Connection","close");self.end_headers();self.wfile.write(body)
                return
            n = int(self.headers.get("Content-Length", "0"))
            is_upload = self.command == "POST" and urlsplit(self.path).path=="/_matrix/media/v3/upload"
            if is_upload:
                index = len(records)
                record = {"expected": n, "received": 0, "partial": False}
                records.append(record)
                if index==0 and hold_first is not None and not hold_first.wait(timeout=25):
                    self.send_error(504)
                    return
                # Both first attempts are slow. On cancellation the transport
                # must actually close before the complete file reaches us.
                body = bytearray()
                while len(body) < n:
                    try:
                        chunk = self.rfile.read(min(n-len(body), 65536))
                    except (OSError, TimeoutError):
                        chunk = None
                    if not chunk:
                        record["partial"] = True
                        return
                    body.extend(chunk)
                    record["received"] = len(body)
                    if index % 2 == 0:
                        time.sleep(.04)
            else:
                body = self.rfile.read(n) if n else None
            conn = http.client.HTTPConnection("127.0.0.1", urlsplit(upstream).port, timeout=65)
            try:
                skip = {"host","connection","content-length","transfer-encoding","accept-encoding"}
                headers = {k:v for k,v in self.headers.items() if k.lower() not in skip}
                conn.request(self.command,self.path,body=body,headers=headers)
                upstream_resp = conn.getresponse()
                data = upstream_resp.read()
                self.send_response(upstream_resp.status)
                for k,v in upstream_resp.getheaders():
                    if k.lower() not in {"connection","content-length","transfer-encoding","server","date"}:
                        self.send_header(k,v)
                self.send_header("Content-Length",str(len(data)))
                self.send_header("Connection","close")
                self.end_headers()
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass  # e.g. closing long-poll sync when a test device exits
            finally:
                conn.close()
        do_GET = do_POST = do_PUT = do_DELETE = do_OPTIONS = forward
    with ThreadingHTTPServer(("127.0.0.1",port),Handler,bind_and_activate=False) as server:
        server.socket.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,128*1024)
        server.server_bind()
        server.server_activate()
        thread=threading.Thread(target=server.serve_forever,daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}", records
        finally:
            server.shutdown()
            thread.join()


def test(tmp, hs, port, web, backend, root, seed):
    with slow_gateway(hs,port) as (url, uploads):
        fixture,hdr = seed(tmp,url)
        room = fixture["room"]
        (tmp/"fixture.json").write_text(json.dumps(fixture))
        payload = os.urandom(SIZE)
        (tmp/"throttled.bin").write_bytes(payload)
        def events():
            r=requests.get(hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")+"/messages",headers=hdr,
                           params={"dir":"b","limit":60},timeout=5)
            r.raise_for_status()
            return [e for e in r.json()["chunk"] if e["type"] in ("m.room.encrypted","m.room.message")]
        def media_count():
            with sqlite3.connect((tmp/"homeserver.db").as_uri()+"?mode=ro",uri=True) as db:
                return db.execute("SELECT COUNT(*) FROM local_media_repository WHERE media_length IS NOT NULL").fetchone()[0]
        def assert_cancel(label, index):
            def disconnected():
                if len(uploads)<=index:return None
                row=uploads[index]
                assert row["received"]<row["expected"] or row["partial"], f"cancel did not stop network upload: {uploads}"
                return row if row["partial"] else None
            record = until("transport disconnect", disconnected)
            assert 0 < record["received"] < record["expected"] == SIZE, record
            assert len(events())==index//2 and media_count()==index//2
            print(f"{label} Cancel: transport closed at {record['received']:,}/{SIZE:,} bytes; 0 new media objects and 0 room events.",flush=True)
        def assert_retry(label,index):
            until("retry delivery",lambda:len(events())>=index//2+1)
            e=events()
            assert len(uploads)==index+2 and uploads[-1]["received"]==SIZE
            assert media_count()==len(e)==index//2+1 and all(x["type"]=="m.room.encrypted" for x in e)
            print(f"{label} Retry: 1 completed media upload and exactly 1 new encrypted room event.",flush=True)
        env={**os.environ,"UNTETHERED_UPLOAD_QA":str(tmp),"UNTETHERED_PROFILE":str(tmp/"seed"),"UNTETHERED_BACKEND":str(backend),"UNTETHERED_ENGINE":"gomuks"}
        proc=subprocess.Popen(["cargo","test","--locked","-j3","--manifest-path","rust-client/Cargo.toml",
                               "throttled_encrypted_upload_cancel_retry","--","--ignored","--nocapture"],cwd=root,env=env)
        try:
            def cancelled():
                assert proc.poll() is None, "native test exited before cancellation"
                return (tmp/"native.cancelled").exists()
            until("native cancelled draft",cancelled,100)
            assert_cancel("Native",0)
            (tmp/"native.retry").touch()
            assert proc.wait(timeout=60)==0
        finally:
            if proc.poll() is None:
                (tmp/"native.retry").touch()
                proc.terminate();proc.wait(timeout=8)
        assert_retry("Native",0)
        # Verify the successful encrypted upload independently of the UI.
        with sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True) as db:
            content=json.loads(db.execute("select decrypted from event where event_id=?",(events()[0]["event_id"],)).fetchone()[0])
        assert content["body"]=="native cancel proof" and content["filename"]=="throttled.bin"
        def b64(s):return base64.urlsafe_b64decode(s+"="*(-len(s)%4))
        def decrypt_stored_object(desc):
            server,mid=desc["url"][6:].split("/",1)
            r=requests.get(f"{hs}/_matrix/client/v1/media/download/{quote(server,safe='')}/{quote(mid,safe='')}",headers=hdr,timeout=25)
            r.raise_for_status()
            assert hashlib.sha256(r.content).digest()==b64(desc["hashes"]["sha256"])
            dec=Cipher(algorithms.AES(b64(desc["key"]["k"])),modes.CTR(b64(desc["iv"]))).decryptor()
            return dec.update(r.content)+dec.finalize()
        assert decrypt_stored_object(content["file"])==payload
        print("Native retry preserved the caption and all 16 MiB of file contents.",flush=True)

        with sync_playwright() as pw:
            browser=pw.chromium.launch(executable_path=os.environ.get("CHROMIUM","/usr/bin/chromium"),
                                       args=["--enable-unsafe-swiftshader","--ignore-gpu-blocklist"])
            page=browser.new_page(viewport={"width":1180,"height":760})
            page.goto(web);page.wait_for_function("document.title.includes('Preview')",timeout=35000)
            page.mouse.click(1095,73);page.wait_for_function("document.title.includes('Account')")
            for y,key in [(299,"server"),(381,"username"),(464,"password")]:
                page.mouse.click(510,y);page.keyboard.type(fixture[key],delay=5)
            page.keyboard.press("Enter")
            field=page.get_by_label("Recovery key or passphrase")
            field.wait_for(timeout=30000);field.fill(fixture["key"]);field.press("Enter")
            page.wait_for_function("document.title.includes('media-proof')",timeout=35000)
            page.evaluate("""n=>{const d=new DataTransfer();const bytes=new Uint8Array(n);
                for(let i=0;i<n;i++)bytes[i]=i%251;
                d.items.add(new File([bytes],'browser.bin'));
                document.querySelector('canvas').dispatchEvent(new DragEvent('drop',{bubbles:true,dataTransfer:d}));}""",SIZE)
            def card():
                # The staged card sits inside the composer, above the message field.
                im=Image.open(io.BytesIO(page.screenshot())).crop((336,535,506,676)).resize((510,423))
                buf=io.BytesIO();im.save(buf,format="PNG")
                return subprocess.run(["tesseract","stdin","stdout","--psm","6"],input=buf.getvalue(),capture_output=True,check=True).stdout.decode()
            until("staged browser file",lambda:"Ready" in card())
            page.mouse.click(1140,708)  # Send arrow
            def shown():
                m=re.search(r"Upload\s+([1-9]\d?)\s*%",card(),re.I)
                return m and m.group(0)
            print("Chromium attachment card:",until("visible upload percentage",shown),flush=True)
            page.mouse.click(465,560)  # The card's Cancel control
            until("visible cancelled draft",lambda:"Cancelled" in card())
            assert_cancel("Chromium",2)
            page.mouse.click(1140,708)  # Retry replaces the send arrow
            assert_retry("Chromium",2)
            until("cleared browser retry card",lambda:"browser.bin" not in card().lower())
            # Independently read the encrypted media object using the descriptor
            # returned by real gomuks timeline pagination, not the staged File.
            page.add_init_script("""
                window.Worker=class extends Worker {
                    constructor(...args){
                        super(...args);
                        this.addEventListener("message",({data:f})=>{
                            if(f.command!=="response" || typeof f.data!=="string")return;
                            const page=JSON.parse(f.data);
                            for(const e of page?.events ?? []){
                                if(e.decrypted?.filename==="browser.bin" || e.decrypted?.body==="browser.bin"){
                                    window.retryMedia=e.decrypted.file;
                                }
                            }
                        });
                    }
                };
            """)
            page.reload()
            page.wait_for_function("document.title.includes('media-proof')",timeout=35000)
            page.wait_for_function("!!window.retryMedia",timeout=35000)
            pattern=bytes(range(251))
            assert decrypt_stored_object(page.evaluate("window.retryMedia"))==(pattern*(SIZE//251+1))[:SIZE]
            print("Chromium reload: decrypted retry download matched the original 16 MiB.",flush=True)
            browser.close()
