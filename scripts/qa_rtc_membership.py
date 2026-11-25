"""Observe the Rust/gomuks device from a separate actual Element Web session."""
import json
import os
import shutil
import subprocess
import time
from urllib.parse import quote

from playwright.sync_api import sync_playwright
from qa_kwin_element import element_server, login_element


def test(tmp, hs, root, backend, seed, http):
    native, hdr = seed(tmp, hs)
    other = tmp/"observer"
    other.mkdir()
    peer, ph = seed(other, hs)
    room = native["room"]
    room_api = hs+"/_matrix/client/v3/rooms/"+quote(room, safe="")
    http("POST", room_api+"/invite", headers=hdr, json={"user_id": "@"+peer["username"]+":test.invalid"})
    http("POST", room_api+"/join", headers=ph, json={})
    # This test never negotiates an SFU. The advertised URL is a local sentinel.
    service = hs+"/qa-focus"
    (tmp/"rtc.json").write_text(json.dumps({"room": room, "service": service}))
    user = "@"+native["username"]+":test.invalid"
    env = {**os.environ, "UNTETHERED_RTC_QA": str(tmp), "UNTETHERED_PROFILE": str(tmp/"seed"),
           "UNTETHERED_BACKEND": str(backend)}
    with element_server(hs) as web, sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"))
        try:
            ctx = browser.new_context()
            ctx.route("**/*", lambda route: route.continue_() if route.request.url.startswith((hs+"/", web+"/", "data:", "blob:")) else route.abort())
            page = login_element(ctx, web, peer, room)
            members = """room => {
                const c = window.mxMatrixClientPeg.get();
                return c.matrixRTC.getRoomSession(c.getRoom(room)).memberships.map(m => ({
                    user: m.userId, device: m.deviceId, member: m.memberId, slot: m.slotId,
                    intent: m.callIntent, service: m.getTransport()?.livekit_service_url
                }));
            }"""
            roster = lambda: page.evaluate(members, room)
            assert page.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()") != user
            assert roster() == []
            cmd = ["cargo", "test", "--locked", "-j2", "--manifest-path", "rust-client/Cargo.toml",
                   "rtc_membership_seen_by_element", "--", "--ignored", "--nocapture"]
            with subprocess.Popen(cmd, cwd=root, env=env) as proc:
                try:
                    def until(label, check, timeout=25):
                        end = time.monotonic()+timeout
                        while time.monotonic() < end:
                            if proc.poll() not in (0, None):
                                raise RuntimeError("Rust membership client failed")
                            if check():
                                return
                            page.wait_for_timeout(80)
                        raise RuntimeError("Timed out: "+label)

                    until("Element RTC roster join", lambda: len(roster()) == 1, 90)
                    device = (tmp/"rtc-device").read_text()
                    wanted = [{"user": user, "device": device, "member": f"{user}:{device}",
                               "slot": "m.call#ROOM", "intent": "audio", "service": service}]
                    assert roster() == wanted, roster()
                    print("Element RTC roster: exactly one native voice member; device, slot and transport match.", flush=True)
                    start = time.monotonic()
                    page.wait_for_timeout(4200)
                    (tmp/"restart").touch()
                    until("gomuks acknowledged delayed leave restart", lambda: (tmp/"restarted").exists())
                    # Past the first eight-second deadline, before the renewed deadline.
                    while time.monotonic()-start < 9:
                        assert roster() == wanted
                        page.wait_for_timeout(150)
                    assert roster() == wanted
                    (tmp/"leave").touch()
                    until("explicit leave observed before renewed deadline", lambda: not roster(), 2)
                    print("Heartbeat kept membership past original deadline; explicit hangup emptied Element's roster.", flush=True)
                    (tmp/"rejoin").touch()
                    until("second join", lambda: roster() == wanted)
                    assert proc.wait(timeout=6) == 0
                    until("homeserver cleared disconnected member", lambda: not roster(), 12)
                    # Check server state to distinguish delayed leave from Element's own expiry timer.
                    content = http("GET", room_api+"/state/org.matrix.msc3401.call.member/"+quote(
                        f"_{user}_{device}_m.call", safe=""), headers=ph).json()
                    assert content == {}
                    print("Disconnected native backend: Synapse published the delayed leave; Element roster empty.", flush=True)
                finally:
                    if proc.poll() is None:
                        # Let the driver shut its own gomuks child down before the
                        # homeserver and profile are removed, also on observer failure.
                        for marker in ("restart", "leave", "rejoin"):
                            (tmp/marker).touch()
        finally:
            browser.close()
