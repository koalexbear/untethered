"""End-to-end attachment proof using disposable profiles and a loopback Synapse.

Build the web client first. From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/test-media-interop.py
CHROMIUM and UNTETHERED_BACKEND can select local executables.
Pass --upload-cancel to run the native and Chromium 16 MiB throttled transfer
test instead (also needs Tesseract and the cryptography Python package).
For the native clipboard acceptance test, pass --native-paste wayland or x11.
It needs a debug client build plus Xvfb, xdotool, ImageMagick, and either
sway/grim/wl-copy (Wayland) or xclip (X11); it never uses your desktop clipboard.
--portal-drop uses the same nested Wayland setup plus a private dbus-daemon,
xdg-document-portal/FUSE and a GTK4/PyGObject system Python (GTK_PYTHON).
--kwin-element uses a virtual KWin, KDE inputsynth, Spectacle, kdotool,
Dolphin and a local extracted Element Web release (see scripts/kwin-element.md).
"""
import argparse
import io
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from urllib.parse import quote

import requests
import websocket
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
BACKEND = Path(os.environ.get("UNTETHERED_BACKEND", ROOT / ".dev/backends/0.2611.1/gomuks")).resolve()



def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@contextmanager
def service(args, cwd, **kw):
    p = subprocess.Popen(args, cwd=cwd, stdout=kw.pop("stdout", subprocess.DEVNULL),
                         stderr=subprocess.DEVNULL, **kw)
    try:
        yield p
    finally:
        p.terminate()
        try:
            p.wait(timeout=12)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()


def http(method, url, **kw):
    r = requests.request(method, url, timeout=15, **kw)
    r.raise_for_status()
    return r


def seed(tmp, hs, *, encrypted=True):
    user = "media" + secrets.token_hex(4)
    pwd = secrets.token_urlsafe(24)
    auth = http("POST", hs+"/_matrix/client/v3/register", json={
        "auth": {"type": "m.login.dummy"}, "username": user, "password": pwd}).json()
    hdr = {"Authorization": "Bearer "+auth["access_token"]}
    room = http("POST", hs+"/_matrix/client/v3/createRoom", headers=hdr, json={
        "name": "media-proof", "preset": "private_chat", "initial_state": [
            {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}] if encrypted else []}).json()["room_id"]
    key = secrets.token_hex(32)
    with service([BACKEND, "--desktop"], tmp, stdout=subprocess.PIPE, env={
        **os.environ, "GOMUKS_ROOT": str(tmp/"seed"), "GOMUKS_DESKTOP_KEY": key}) as p:
        addr = json.loads(p.stdout.readline())["address"]
        cookie = http("POST", f"http://{addr}/_gomuks/auth?insecure_cookie=true",
                      auth=("desktop-key", key)).cookies["gomuks_auth"]
        ws = websocket.create_connection(f"ws://{addr}/_gomuks/websocket", suppress_origin=True,
                                         cookie="gomuks_auth="+cookie, timeout=30)
        def rpc(n, cmd, body):
            ws.send(json.dumps(dict(command=cmd, request_id=n, data=body)))
            while True:
                f = json.loads(ws.recv())
                if f["command"] in ("error", "response") and f.get("request_id") == n:
                    assert f["command"] == "response", cmd
                    return f["data"]
        rpc(1, "login", dict(homeserver_url=hs, username=user, password=pwd))
        keys = rpc(2, "generate_recovery_key", {})
        rpc(3, "reset_encryption", {**keys, "account_password": pwd})
        ws.close()
    fixture = dict(server=hs, username=user, password=pwd, key=keys["recovery_key"], room=room)
    # gomuks only prepared the account and its recovery key. The client's own
    # session: a debug client signs in and verifies, then exits (engine/qa.rs).
    client = os.environ.get("UNTETHERED_QA_CLIENT") or str(ROOT/"rust-client/target/debug/untethered-iced")
    subprocess.run([client], check=True, timeout=360, env={
        **os.environ, "UNTETHERED_PROFILE": str(tmp/"seed"), "UNTETHERED_QA_SEED": json.dumps(fixture),
        "UNTETHERED_KEYRING": "off"})
    return fixture, hdr


def check_browser(fixture, web):
    from qa_web_login import login_client
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                     args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        ctx = browser.new_context(viewport={"width": 1180, "height": 760})
        page = ctx.new_page()
        login_client(page,web,fixture)
        for attempt in range(2):
            if attempt:
                page.reload()
            page.wait_for_function("document.title.includes('media-proof')", timeout=35000)
            end = time.monotonic()+35
            while True:
                im = Image.open(io.BytesIO(page.screenshot())).convert("RGB").crop((280,50,1180,650))
                colors = im.getcolors(im.width * im.height)
                pink = sum(n for n,(r,g,b) in colors if r>205 and g<58 and 120<b<158)
                teal = sum(n for n,(r,g,b) in colors if r<38 and g>195 and 152<b<196)
                if min(pink,teal)>5000:
                    break
                if time.monotonic()>end:
                    raise RuntimeError(f"Encrypted thumbnail not visible after login/reload: {pink}/{teal} matching pixels")
            print(f"Chromium {'reload' if attempt else 'fresh verified device'}: {pink} pink and {teal} teal pixels in timeline.")
        browser.close()


def main():
    args=argparse.ArgumentParser(description=__doc__)
    args.add_argument("--native-paste",choices=("wayland","x11"))
    args.add_argument("--portal-drop",action="store_true",help="Native Wayland portal-only PNG/PDF drag with a real document portal")
    args.add_argument("--kwin-element",action="store_true",help="Drive a real Dolphin drag under KWin and round-trip both files through Element Web")
    args.add_argument("--file-limits",choices=("native","web"),help="Round-trip plain PNG/PDF through Element; enforce an exact 64 KiB server boundary on actual clients")
    args.add_argument("--rtc-membership",action="store_true",help="Observe the Rust client's MatrixRTC join, heartbeat, hangup and abandoned membership in Element Web")
    args.add_argument("--replies",action="store_true",help="Send and edit encrypted replies in Iced and Element, reconnect both and jump to the originals beyond recent history")
    args.add_argument("--reactions",action="store_true",help="Toggle the same reaction from Iced and Element on an edited reply and verify ownership after reconnect")
    args.add_argument("--room-search",action="store_true",help="Find an older edited reply in the local room cache and verify the exact reading position on return")
    args.add_argument("--room-alerts",action="store_true",help="Verify per-room push rules and visible direct-mention notifications on a private Linux desktop")
    args.add_argument("--browser-alerts",action="store_true",help="Verify a real browser notification from a hidden Untethered tab in a muted encrypted room")
    args.add_argument("--account-dnd",action="store_true",help="Toggle account-wide DND between Linux and Chromium; check both desktops and their persistent Matrix settings")
    args.add_argument("--space-alerts",choices=("native","web"),help="Verify nested Matrix space rules, room overrides, live membership and real popups")
    args.add_argument("--read-state",action="store_true",help="Verify private read markers, manual reminders and unread badges between two actual Linux and web devices")
    args.add_argument("--dnd-ring",action="store_true",help="Verify active and new Linux incoming ringtone suppression while accepting a real encrypted DM under DND")
    args.add_argument("--room-permissions",action="store_true",help="Restrict and restore room posting from Iced; check the member's actual Synapse send response")
    args.add_argument("--community-setup",choices=("native","web"),help="Create a private space and encrypted text/voice channels; verify restricted joins from Element")
    args.add_argument("--room-ban",action="store_true",help="Ban, unban and reinvite a member from Untethered; verify Synapse join enforcement and encrypted delivery")
    args.add_argument("--room-kick",action="store_true",help="Kick a member from a public room; verify denied posting, invitation-free rejoin and encrypted delivery")
    args.add_argument("--message-report",nargs="?",const="native",choices=("native","web"),help="Report a selected encrypted message as a regular member and inspect the Synapse admin queue")
    args.add_argument("--mentions-inbox",nargs="?",const="native",choices=("native","web"),help="Verify cross-room direct mentions after a device restart and open exact events beyond loaded history")
    args.add_argument("--linux-upgrade",nargs=2,type=Path,metavar=("OLD_PKG","NEW_PKG"),help="Upgrade an installed Arch package in a disposable root and verify Matrix device and decryption continuity")
    args.add_argument("--federated-ban",action="store_true",help="Test the member ban flow through a second Synapse homeserver in a private loopback namespace")
    args.add_argument("--rtc-audio",action="store_true",help="Verify encrypted Rust ↔ Element Call tones using real LiveKit and Matrix OpenID authorization")
    args.add_argument("--desktop-dm",action="store_true",help="Drive the Iced DM call controls and Element's incoming call prompt with isolated audio")
    args.add_argument("--native-dm",action="store_true",help="Call in both directions between two Linux desktops behind separate TURN-only networks")
    args.add_argument("--dm-video",action="store_true",help="Verify encrypted two-way DM video, camera consent and call controls on two isolated Linux desktops")
    args.add_argument("--dm-window",action="store_true",help="Verify a portal-selected window with two cameras and two microphones in a DM; add --element-video for an Element receiver")
    args.add_argument("--element-video",action="store_true",help="Exchange changing encrypted camera frames with Element Call in both caller directions")
    args.add_argument("--switch-camera",action="store_true",help="Switch two portal-authorized cameras in a live encrypted video call with Element")
    args.add_argument("--unsupported-dm",action="store_true",help="Check the Linux call controls when Synapse advertises no MatrixRTC service")
    args.add_argument("--incoming-dm",action="store_true",help="Ring the Iced desktop twice from Element, decline then accept and hang up")
    args.add_argument("--multi-device-ring",action="store_true",help="Ring two independently verified Linux devices on one Matrix account; decline on one, answer on the other")
    args.add_argument("--call-controls",action="store_true",help="Measure Linux mute/deafen and track continuity during a live encrypted call with Element")
    args.add_argument("--call-devices",action="store_true",help="Switch two microphones and two speakers using the Linux call bar during a live encrypted call with Element")
    args.add_argument("--call-reconnect",action="store_true",help="Cut the Linux desktop's private network link during an encrypted Element call, then verify automatic two-way recovery")
    args.add_argument("--call-turn",action="store_true",help="Block direct ICE from both clients and verify selected TURN routes and encrypted audio")
    args.add_argument("--voice-room",action="store_true",help="Join a silent encrypted voice room with three native Linux clients and measure every audio path")
    args.add_argument("--window-share",action="store_true",help="Share a selected test window through KDE's ScreenCast portal to two encrypted voice peers")
    args.add_argument("--room-camera",action="store_true",help="Toggle a portal-authorized virtual camera and measure the changing decoded frames at both voice-room peers")
    args.add_argument("--room-layout",action="store_true",help="Measure grid and focus views of two live portal cameras on a third desktop")
    args.add_argument("--room-ptt",action="store_true",help="Verify key-gated microphone and uninterrupted peer audio in a three-client Linux voice room")
    args.add_argument("--upload-cancel",action="store_true",help="Test native and browser cancellation against a throttled Matrix upload")
    args=args.parse_args()
    if args.room_kick and (args.room_ban or args.federated_ban):
        raise SystemExit("Run the kick and ban fixtures separately")
    if args.room_layout and (args.room_ptt or args.window_share or args.room_camera):
        raise SystemExit("Run --room-layout separately; it uses two independent camera desktops")
    if args.room_ptt and (args.window_share or args.room_camera):
        raise SystemExit("--room-ptt uses private X displays; run the KWin video fixtures separately")
    if args.window_share or args.room_camera or args.room_ptt or args.room_layout: args.voice_room=True
    if args.dm_window:
        if args.voice_room:args.error("Run the DM and group window fixtures separately")
        if not args.element_video:args.dm_video=True
        args.window_share=True
    if args.call_controls or args.call_devices or args.call_reconnect or args.call_turn: args.desktop_dm=True
    if args.switch_camera:args.element_video=True
    if args.native_dm or args.unsupported_dm or args.dm_video or args.element_video:args.desktop_dm=True
    if args.native_dm and args.unsupported_dm:args.error("run --native-dm and --unsupported-dm separately")
    if args.multi_device_ring:args.incoming_dm=True
    if args.dnd_ring:args.incoming_dm=True
    if args.account_dnd or args.read_state or args.space_alerts=="web":args.browser_alerts=True
    if args.desktop_dm or args.incoming_dm or args.voice_room: args.rtc_audio=True
    assert BACKEND.is_file(), "Run scripts/rust-client.sh once or set UNTETHERED_BACKEND"
    if args.federated_ban:
        from qa_ban_federation import federated_ban
        os.umask(0o077)
        with tempfile.TemporaryDirectory(prefix="untethered-ban-",dir=os.environ.get("TMPDIR")) as d:
            federated_ban(Path(d),ROOT,BACKEND,seed,http,free_port,service)
        return
    dist = ROOT/"rust-client/web/dist"
    chat_test=args.replies or args.reactions or args.room_search or args.room_alerts or args.room_permissions or args.room_ban or args.room_kick or args.message_report or args.mentions_inbox or args.space_alerts=="native" or args.community_setup
    web_chat="web" in (args.message_report,args.mentions_inbox,args.community_setup)
    if web_chat or args.file_limits=="web" or not any((args.file_limits,args.native_paste,args.portal_drop,args.kwin_element,args.rtc_membership,args.rtc_audio,chat_test,args.linux_upgrade)):
        assert (dist/"engine-worker.js").is_file(), "Run scripts/rust-web.sh first"
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="untethered-media-",dir=os.environ.get("TMPDIR")) as d:
        tmp = Path(d)
        server_name="test.invalid"
        extra_listeners=extra_settings=""
        if args.rtc_audio:
            from qa_rtc_audio import configure, test as audio_test
            server_name,extra_listeners,extra_settings,rtc_service=configure(tmp,free_port)
            if args.unsupported_dm:extra_settings=extra_settings.split("extra_well_known_client_content:")[0]
        if chat_test or args.browser_alerts:
            extra_settings="rc_message: {per_second: 1000, burst_count: 1000}\n"
        if args.file_limits:extra_settings+="max_upload_size: 64K\n"
        # Each account signs in twice (gomuks prepares it, then the client):
        # past Synapse's default burst of five logins per address.
        extra_settings+="rc_login: {address: {per_second: 1000, burst_count: 1000}, account: {per_second: 1000, burst_count: 1000}}\n"
        subprocess.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config",
                        "-H", server_name, "--report-stats", "no"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
        port = free_port()
        hs = f"http://127.0.0.1:{port}"
        gateway_port=free_port() if args.upload_cancel or args.portal_drop or args.kwin_element or args.file_limits or chat_test or args.linux_upgrade else None
        public_url=f"http://127.0.0.1:{gateway_port}" if gateway_port else hs
        (tmp/"local.yaml").write_text(f"""listeners:
{extra_listeners}  - port: {port}
    type: http
    tls: false
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [client, media]}}]
public_baseurl: {public_url}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: []
{extra_settings}""")
        if args.rtc_membership or args.rtc_audio:
            with (tmp/"local.yaml").open("a") as cfg:
                cfg.write("max_event_delay_duration: 1h\n" if args.rtc_audio else "max_event_delay_duration: 15s\n")
        with service([sys.executable,"-m","synapse.app.homeserver","-c","hs.yaml","-c","local.yaml"],tmp) as proc:
            for _ in range(200):
                if proc.poll() is not None:
                    raise RuntimeError("Disposable homeserver exited")
                try:
                    http("GET",hs+"/_matrix/client/versions")
                    break
                except requests.ConnectionError:
                    time.sleep(.1)
            if args.browser_alerts:
                from qa_browser_alerts import check_browser_alerts
                wp=free_port()
                with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                    check_browser_alerts(tmp,hs,f"http://127.0.0.1:{wp}",BACKEND,seed,http,dnd=args.account_dnd,reading=args.read_state,spaces=args.space_alerts=="web")
                return
            if args.linux_upgrade:
                from qa_linux_upgrade import check_upgrade
                check_upgrade(tmp,hs,gateway_port,BACKEND,seed,http,*[p.resolve(strict=True) for p in args.linux_upgrade])
                return
            if chat_test:
                from qa_replies import test_replies
                if web_chat:
                    wp=free_port()
                    with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                        test_replies(tmp,hs,ROOT,BACKEND,seed,http,gateway_port,report=args.message_report,mentions=args.mentions_inbox,setup=args.community_setup,client_web=f"http://127.0.0.1:{wp}")
                    return
                test_replies(tmp,hs,ROOT,BACKEND,seed,http,gateway_port,reactions=args.reactions,room_search=args.room_search,alerts=args.room_alerts,permissions=args.room_permissions,ban=args.room_ban,kick=args.room_kick,report=args.message_report,mentions=args.mentions_inbox,spaces=args.space_alerts=="native",setup=args.community_setup)
                return
            if args.rtc_audio:
                audio_test(tmp,hs,server_name,rtc_service,ROOT,BACKEND,seed,http,service,desktop=args.desktop_dm or args.incoming_dm,incoming=args.incoming_dm,controls=args.call_controls,devices=args.call_devices,reconnect=args.call_reconnect,turn=args.call_turn,group=args.voice_room,share=args.window_share,camera=args.room_camera,ptt=args.room_ptt,layout=args.room_layout,multi_ring=args.multi_device_ring,dnd_ring=args.dnd_ring,native_dm=args.native_dm,unsupported_dm=args.unsupported_dm,dm_video=args.dm_video,element_video=args.element_video,switch_camera=args.switch_camera)
                return
            if args.rtc_membership:
                from qa_rtc_membership import test
                test(tmp,hs,ROOT,BACKEND,seed,http)
                return
            if args.file_limits=="web":
                from qa_kwin_element import test
                wp=free_port()
                with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                    test(tmp,hs,gateway_port,ROOT,BACKEND,seed,plain=True,web_client=f"http://127.0.0.1:{wp}")
                return
            if args.kwin_element or args.file_limits=="native":
                from qa_kwin_element import test
                test(tmp,hs,gateway_port,ROOT,BACKEND,seed,plain=bool(args.file_limits))
                return
            if args.portal_drop:
                from qa_portal_drop import test
                test(tmp,hs,gateway_port,ROOT,BACKEND,seed)
                return
            if args.upload_cancel:
                from qa_upload_cancel import test
                wp=free_port()
                with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                    test(tmp,hs,gateway_port,f"http://127.0.0.1:{wp}",BACKEND,ROOT,seed)
                return
            f, hdr = seed(tmp,hs)
            if args.native_paste:
                from qa_native_paste import check_paste
                check_paste(tmp, ROOT, BACKEND, f, hdr, args.native_paste)
                return
            # The default round trip moved to scripts/qa_matrix_sdk.py (desktop) and
            # its UNTETHERED_QA_ONLY=web mode (browser) with the engine switch.
            raise SystemExit("Pick a mode, or run scripts/qa_matrix_sdk.py for the encrypted round trip.")
        print("PASS encrypted image interoperability; disposable services stopped.")


if __name__ == "__main__":
    main()
