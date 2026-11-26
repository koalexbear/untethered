"""End-to-end encrypted audio with the Element Call distribution embedded in Element Web."""
import array
import base64
import hashlib
import json
import math
import os
import re
import secrets
import shutil
import subprocess as sp
import threading
import wave
from contextlib import ExitStack, contextmanager
from urllib.parse import quote, urlsplit

import requests
from cryptography import x509
from cryptography.hazmat.primitives import serialization
from playwright.sync_api import Error as PlaywrightError, sync_playwright
from qa_kwin_element import element_server, login_element
from qa_native_paste import wait_for


def configure(tmp, free_port):
    links=json.loads(sp.check_output(["ip","-j","link","show"]))
    assert {x["ifname"] for x in links} == {"lo","rtc-test"}, "Run in the private QA network namespace"
    assert "192.0.2.2/24" in sp.check_output(["ip","-br","addr","show","rtc-test"],text=True)
    name=f"127.0.0.1:{free_port()}"
    jwt=f"http://127.0.0.1:{free_port()}"
    def openssl(*args):
        sp.run(["openssl",*args],cwd=tmp,check=True,stdout=sp.DEVNULL,stderr=sp.DEVNULL)
    openssl("req","-x509","-newkey","rsa:2048","-nodes","-days","1","-subj","/CN=RTC QA CA",
            "-addext","basicConstraints=critical,CA:TRUE","-keyout","ca.key","-out","ca.crt")
    openssl("req","-new","-newkey","rsa:2048","-nodes","-subj","/CN=127.0.0.1","-keyout","tls.key","-out","tls.csr")
    (tmp/"ext").write_text("subjectAltName=IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
    openssl("x509","-req","-in","tls.csr","-CA","ca.crt","-CAkey","ca.key","-CAcreateserial",
            "-days","1","-extfile","ext","-out","tls.crt")
    # OpenID userinfo must actually be served by Synapse on the federation listener.
    listen=f"""  - port: {urlsplit("https://"+name).port}
    type: http
    tls: true
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [openid, client]}}]
"""
    settings=f"""tls_certificate_path: {tmp}/tls.crt
tls_private_key_path: {tmp}/tls.key
extra_well_known_client_content:
  org.matrix.msc4143.rtc_foci: [{{type: livekit, livekit_service_url: "{jwt}"}}]
"""
    return name,listen,settings,jwt


@contextmanager
def call_services(tmp,name,jwt,service,turn=False):
    secret=secrets.token_hex(32)
    (tmp/"lk.yaml").write_text(f"""port: 18820
bind_addresses: ["127.0.0.1"]
keys: {{testkey: "{secret}"}}
rtc:
  tcp_port: 18821
  udp_port: 18822
  node_ip: 192.0.2.2
  enable_loopback_candidate: true
  ips: {{includes: ["192.0.2.2/32"]}}
""")
    with ExitStack() as stack:
        if turn:
            from qa_rtc_turn import configure_turn
            configure_turn(tmp,stack,service)
        stack.enter_context(service([os.environ["LIVEKIT_SERVER"],"--config",str(tmp/"lk.yaml")],tmp))
        auth=stack.enter_context(service([os.environ["LK_JWT_SERVICE"]],tmp,env={**os.environ,
            "LIVEKIT_JWT_BIND":urlsplit(jwt).netloc,"LIVEKIT_URL":"ws://127.0.0.1:18820","LIVEKIT_KEY":"testkey",
            "LIVEKIT_SECRET":secret,"LIVEKIT_FULL_ACCESS_HOMESERVERS":name,"SSL_CERT_FILE":str(tmp/"ca.crt"),
            "LIVEKIT_LOG_LEVEL":"warn"}))
        def ready():
            assert auth.poll() is None, "lk-jwt-service exited"
            try:return requests.get(jwt+"/healthz",timeout=2).ok
            except requests.ConnectionError:return False
        wait_for("authorization service",ready)
        yield secret


def test(tmp, hs, name, jwt, root, backend, seed, http, service, *, desktop=False,incoming=False,controls=False,devices=False,reconnect=False,turn=False,group=False,share=False,camera=False,ptt=False,layout=False,multi_ring=False,dnd_ring=False,muted_peer=False,native_dm=False,unsupported_dm=False,dm_video=False,element_video=False,switch_camera=False):
    if group:
        from qa_voice_room import test_group
        with call_services(tmp,name,jwt,service) as secret:
            test_group(tmp,hs,name,root,backend,seed,http,secret,share=share,camera=camera,ptt=ptt,layout=layout)
        return
    native,hdr=seed(tmp,hs)
    peer_dir=tmp/"peer";peer_dir.mkdir()
    peer,ph=seed(peer_dir,hs)
    room=native["room"]
    api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
    bob="@"+peer["username"]+":"+name
    http("POST",api+"/invite",headers=hdr,json={"user_id":bob})
    http("POST",api+"/join",headers=ph,json={})
    if desktop:
        alice="@"+native["username"]+":"+name
        for me,them,auth_header in ((alice,bob,hdr),(bob,alice,ph)):
            http("PUT",hs+"/_matrix/client/v3/user/"+quote(me,safe="")+"/account_data/m.direct",
                 headers=auth_header,json={them:[room]})
    power=http("GET",api+"/state/m.room.power_levels",headers=hdr).json()
    power["events"]["org.matrix.msc3401.call.member"]=0
    power["events"]["org.matrix.msc4075.rtc.notification"]=0
    power["events"]["org.matrix.msc4143.rtc.slot"]=0
    if incoming or element_video:
        power.setdefault("notifications",{})["room"]=0
    http("PUT",api+"/state/m.room.power_levels",headers=hdr,json=power)
    if controls:
        http("POST",hs+"/_matrix/client/v3/createRoom",headers=hdr,json={"name":"Sidechat","preset":"private_chat"})
    assert http("GET",api+"/state/m.room.encryption",headers=ph).json()["algorithm"]=="m.megolm.v1.aes-sha2"
    if dm_video:
        from qa_dm_video import check_video
        http("POST",hs+"/_matrix/client/v3/rooms/"+quote(peer["room"],safe="")+"/leave",headers=ph,json={})
        with call_services(tmp,name,jwt,service) as secret, ExitStack() as stack:
            def sidechat():
                other=http("POST",hs+"/_matrix/client/v3/createRoom",headers=hdr,json={"name":"Sidechat",
                    "preset":"private_chat","invite":[bob],"initial_state":[{"type":"m.room.encryption",
                    "state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]}).json()["room_id"]
                http("POST",hs+"/_matrix/client/v3/rooms/"+quote(other,safe="")+"/join",headers=ph,json={})
                return other
            check_video(stack,tmp,root,backend,room,(alice,bob),secret,sidechat,share)
        return
    if native_dm or unsupported_dm:
        from qa_native_dm import check_native_dm
        # The seed users each own a room. Leave the unused room so both GUIs
        # open the intended encrypted DM without relying on sort tie-breaks.
        http("POST",hs+"/_matrix/client/v3/rooms/"+quote(peer["room"],safe="")+"/leave",headers=ph,json={})
        with ExitStack() as stack:
            secret=None if unsupported_dm else stack.enter_context(call_services(tmp,name,jwt,service,turn=True))
            check_native_dm(stack,tmp,hs,name,jwt,root,backend,room,(alice,bob),(hdr,ph),secret,http)
        return
    wav=tmp/"element-mic.wav"
    with wave.open(str(wav),"wb") as w:
        w.setparams((1,2,48000,0,"NONE","not compressed"))
        hz=670 if element_video else 960
        w.writeframes(array.array("h",(round(8500*math.sin(i*math.tau*hz/48000)) for i in range(48000))).tobytes())
    with call_services(tmp,name,jwt,service,turn) as secret, ExitStack() as stack:
        web=stack.enter_context(element_server(hs,server_name=name,rtc_service=jwt))
        links=None
        chrome=shutil.which("chromium")
        if turn:
            from qa_rtc_turn import relay_clients
            ports=[urlsplit(u).port for u in (hs,jwt,web,"https://"+name)]+[18820]
            links,chrome=relay_clients(stack,tmp,ports)
        pw=stack.enter_context(sync_playwright())
        leaf=x509.load_pem_x509_certificate((tmp/"tls.crt").read_bytes())
        spki=leaf.public_key().public_bytes(serialization.Encoding.DER,serialization.PublicFormat.SubjectPublicKeyInfo)
        pin=base64.b64encode(hashlib.sha256(spki).digest()).decode()
        video_flags=[]
        if element_video:
            from qa_element_video import y4m
            camera_file=tmp/"element-camera.y4m";y4m(camera_file)
            video_flags=["--use-file-for-fake-video-capture="+str(camera_file)]
        launch={}
        capture_flags=["--use-fake-ui-for-media-stream"]
        if share and element_video:
            from qa_native_paste import child,isolated_env,start_display
            chromium_dir=tmp/"browser-display";chromium_dir.mkdir()
            env=isolated_env(chromium_dir,backend,"x11")
            start_display(stack,chromium_dir,env,"x11",1280,900)
            audio_sock=chromium_dir/"pulse"
            env.update(PULSE_SERVER="unix:"+str(audio_sock),PULSE_SINK="BrowserOutput")
            stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
                "--load=module-native-protocol-unix socket="+str(audio_sock)+" auth-anonymous=1",
                "--load=module-null-sink sink_name=BrowserOutput rate=48000"],env))
            wait_for("isolated browser speaker",audio_sock.is_socket)
            launch=dict(env=env,headless=False,ignore_default_args=["--mute-audio"])
            capture_flags=["--auto-select-tab-capture-source-by-title=Untethered Prism QA","--ozone-platform=x11"]
        browser=pw.chromium.launch(executable_path=chrome,**launch,args=[*video_flags,"--use-fake-device-for-media-stream",
            "--ignore-certificate-errors-spki-list="+pin,*capture_flags,*([] if share and element_video else ["--mute-audio"]),
            "--use-file-for-fake-audio-capture="+str(wav),"--autoplay-policy=no-user-gesture-required"])
        stack.callback(browser.close)
        ctx=browser.new_context(permissions=["microphone","camera"])
        ctx.add_init_script(path=root/"scripts/qa_rtc_pcm.js")
        if muted_peer:
            from qa_desktop_dm import UNPLUGGED_MIC
            ctx.add_init_script(script=UNPLUGGED_MIC)
        page=login_element(ctx,web,peer,room)
        if element_video:
            from qa_element_video import verify
            verify(stack,tmp,root,backend,page,room,alice,bob,secret,incoming,switch_camera,share)
            return
        if desktop:
            from qa_desktop_dm import desktop_call
            ports=[urlsplit(hs).port,urlsplit(jwt).port,urlsplit("https://"+name).port,18820] if reconnect else None
            sibling=None
            if multi_ring:
                from qa_multi_device_ring import Sibling
                sibling=Sibling(stack,tmp,root,backend,native,alice)
            def set_dnd(on):
                endpoint=hs+"/_matrix/client/v3/pushrules/global/override/.m.rule.master/enabled"
                http("PUT",endpoint,headers=hdr,json={"enabled":on})
                assert http("GET",endpoint,headers=hdr).json()=={"enabled":on}
            desktop_call(stack,tmp,page,room,root,backend,alice,bob,secret,incoming=incoming,controls=controls,devices=devices,network_ports=ports,relay_links=links,sibling=sibling,set_dnd=set_dnd if dnd_ring else None,muted_peer=muted_peer)
            return
        sockets=[]
        def watch_sfu(ws):
            if urlsplit(ws.url).netloc=="127.0.0.1:18820":
                sockets.append(ws)
        page.on("websocket",watch_sfu)
        dev=page.evaluate("window.mxMatrixClientPeg.get().getDeviceId()")
        call=page.get_by_role("button",name="Voice call",exact=True)
        call.click(timeout=12000)
        def widget_joined():
            for f in page.frames:
                if f==page.main_frame:continue
                join=f.get_by_role("button",name=re.compile(r"^(Join|Join call|Start call|Enter call)$",re.I))
                try:
                    if join.count() and join.first.is_visible():join.first.click(timeout=1000)
                except PlaywrightError:
                    if not f.is_detached():raise
            session=page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get();return c.matrixRTC.getRoomSession(c.getRoom(room)).memberships.map(m=>m.deviceId)}""",room)
            return dev in session
        wait_for("Element Call publishes membership",widget_joined,30)
        widget=next(f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/"))
        print("Element Call joined the encrypted room using its test microphone.",flush=True)
        key=secrets.token_hex(32)
        engine=stack.enter_context(service([backend,"--desktop"],tmp,stdout=sp.PIPE,env={
            **os.environ,"GOMUKS_ROOT":str(tmp/"seed"),"GOMUKS_DESKTOP_KEY":key}))
        addr=json.loads(engine.stdout.readline())["address"]
        session=http("POST",f"http://{addr}/_gomuks/auth?insecure_cookie=true",auth=("desktop-key",key))
        probe=sp.Popen([root/"experiments/rtc-audio/target/debug/untethered-rtc-probe","--element"],
                       stdin=sp.PIPE,stdout=sp.PIPE,text=True)
        rotated=threading.Event()
        def progress():
            for line in probe.stdout:
                print(line,end="",flush=True)
                if line.strip()=="ROTATION_ACTIVE index=1 tone_hz=730":
                    rotated.set()
        reader=threading.Thread(target=progress)
        reader.start()
        def stop_probe():
            if probe.poll() is None:probe.terminate()
            probe.wait(timeout=8)
            reader.join(timeout=3)
        stack.callback(stop_probe)
        probe.stdin.write(json.dumps({"ws":f"ws://{addr}/_gomuks/websocket","cookie":session.cookies["gomuks_auth"],
            "room":room,"service":jwt,"user":bob,"device":dev})+"\n")
        probe.stdin.flush()
        def stats():
            assert probe.poll() is None, "native probe exited during the media test"
            return widget.evaluate("window.qaAudioStats?.() ?? null")
        def signal(message):
            probe.stdin.write(message+"\n");probe.stdin.flush()
        def inbound():
            s=stats()
            if not s:return False
            assert not s["got480"],"Element decoded 480 Hz without its sender key"
            if s["packets"]>=60 and s["spectra"]>=15:return s["packets"]
        packets=wait_for("Element receiving encrypted RTP before key delivery",inbound,40)
        print(f"Withheld native sender key: {packets} inbound RTP packets, no decoded 480 Hz tone.",flush=True)
        signal("release-sender-key")
        initial=wait_for("Element decoded 480 Hz with key 0",lambda:s if (s:=stats())["got480"] else None,45)
        print("Element decoded 480 Hz from Rust in 10 consecutive audio spectra.",flush=True)
        assert initial["ssrc"] and initial["state"]=="connected" and initial["hits730"]==0
        initial_sockets=list(sockets)
        assert initial_sockets and all(not ws.is_closed() for ws in initial_sockets)
        def same_stream():
            s=stats()
            assert s and s["state"]=="connected" and s["packets"]>=initial["packets"]
            for field in ("receiverId","trackId","ssrc","peerConnections","losses"):
                assert s[field]==initial[field], f"{field} changed during sender-key rotation"
            assert sockets==initial_sockets and all(not ws.is_closed() for ws in sockets),"SFU signaling reconnected"
            return s
        signal("rotate-after-480")
        assert rotated.wait(timeout=8),"Rust did not switch the active frame cryptor and audio source"
        start=same_stream()
        def withheld():
            s=same_stream()
            assert s["hits730"]==0,"Element decoded new audio before the rotated key was delivered"
            if s["packets"]-start["packets"]>=80 and s["spectra"]-start["spectra"]>=18 and s["spectra"]-s["last480"]>=5:
                return s
        silent=wait_for("encrypted RTP continues while only key 0 is available",withheld,12)
        delta=silent["packets"]-start["packets"]
        print(f"Withheld key 1: +{delta} packets on the original SSRC, 0 spectra at 730 Hz.",flush=True)
        signal("release-rotated-key")
        after=wait_for("Element decoded 730 Hz with key 1",lambda:s if (s:=same_stream())["got730"] else None,15)
        print(f"Released key 1: Element measured {after['measured730']:.1f} Hz for 10 consecutive spectra; "
              "same receiver, track, SSRC and SFU signaling connection.",flush=True)
        signal("element-decoded-730");probe.stdin.close()
        assert probe.wait(timeout=10)==0,"native continuity or encryption check failed"
