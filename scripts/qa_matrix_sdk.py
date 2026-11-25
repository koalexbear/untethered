"""Acceptance test for the matrix-sdk engine (UNTETHERED_ENGINE=matrix-sdk).

From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    python scripts/qa_matrix_sdk.py

Starts a disposable loopback Synapse, seeds the account and its recovery key
through gomuks (like test-media-interop.py), and runs a second account in
gomuks as the encrypted peer. The Rust side is two ignored tests that drive
the real App:
- `matrix_sdk_login_restore_and_encrypted_round_trip`: login, recovery, sync,
  pagination, send and a restart, with the store passphrase in a private
  GNOME Keyring on a private D-Bus (never your own keyring);
- `matrix_sdk_account_flows`: emoji verification between two engine
  instances, the device list and remote sign-out, recovery setup, key change
  and reset on an account without keys, and sign-out, with the keyring off.
UNTETHERED_BACKEND selects the gomuks executable. UNTETHERED_QA_SOCKETS picks
a short directory for the private bus socket (default: the temp directory).
Pass --keep to leave the profile and homeserver directory for inspection.
"""
from contextlib import contextmanager
import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import parse_qs, quote, urlsplit

import requests
import websocket

ROOT = Path(__file__).resolve().parents[1]
BACKEND = Path(os.environ.get("UNTETHERED_BACKEND", ROOT / ".dev/backends/0.2611.1/gomuks")).resolve()
SERVER_NAME = "test.invalid"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def http(method, url, **kw):
    r = requests.request(method, url, timeout=15, **kw)
    r.raise_for_status()
    return r


class Gomuks:
    """One gomuks --desktop process, spoken to over its loopback WebSocket."""

    def __init__(self, root, hs, user, pwd):
        root.mkdir(parents=True)
        key = secrets.token_hex(32)
        self.p = subprocess.Popen([BACKEND, "--desktop"], cwd=root, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL,
                                  env={**os.environ, "GOMUKS_ROOT": str(root), "GOMUKS_DESKTOP_KEY": key})
        addr = json.loads(self.p.stdout.readline())["address"]
        # --desktop keeps writing notification previews; never let the pipe fill.
        threading.Thread(target=self.p.stdout.read, daemon=True).start()
        cookie = http("POST", f"http://{addr}/_gomuks/auth?insecure_cookie=true",
                      auth=("desktop-key", key)).cookies["gomuks_auth"]
        self.ws = websocket.create_connection(f"ws://{addr}/_gomuks/websocket", suppress_origin=True,
                                              cookie="gomuks_auth=" + cookie, timeout=30)
        self.n, self.last, self.pinged, self.events, self.rooms = 0, 0, time.monotonic(), [], set()
        self.rpc("login", dict(homeserver_url=hs, username=user, password=pwd))

    def frame(self, timeout):
        if time.monotonic() - self.pinged > 15:
            self.ws.send(json.dumps(dict(command="ping", request_id=0, data=dict(last_received_id=self.last))))
            self.pinged = time.monotonic()
        self.ws.settimeout(timeout)
        try:
            f = json.loads(self.ws.recv())
        except websocket.WebSocketTimeoutException:
            return None
        if isinstance(f.get("request_id"), int) and f["request_id"] < 0:
            self.last = f["request_id"]
        if f["command"] == "sync_complete":
            for room_id, room in (f["data"].get("rooms") or {}).items():
                self.rooms.add(room_id)
                self.events += room.get("events") or []
        elif f["command"] == "events_decrypted":
            self.events += f["data"].get("events") or []
        return f

    def rpc(self, cmd, body):
        self.n += 1
        self.ws.send(json.dumps(dict(command=cmd, request_id=self.n, data=body)))
        end = time.monotonic() + 60
        while time.monotonic() < end:
            f = self.frame(5)
            if f and f["command"] in ("error", "response") and f.get("request_id") == self.n:
                assert f["command"] == "response", (cmd, f["data"])
                return f["data"]
        raise RuntimeError(f"gomuks {cmd} timed out")

    def recovery(self, pwd):
        keys = self.rpc("generate_recovery_key", {})
        self.rpc("reset_encryption", {**keys, "account_password": pwd})
        return keys["recovery_key"]

    def joined(self, room, timeout=60):
        end = time.monotonic() + timeout
        while room not in self.rooms:
            if time.monotonic() > end:
                raise RuntimeError(f"gomuks never synced {room}")
            self.frame(1)

    def find(self, label, matches, timeout=60):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for e in self.events:
                if matches(e):
                    return e
            self.frame(1)
        raise RuntimeError(f"gomuks never saw {label}")

    def decrypted(self, body, timeout=60):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for e in self.events:
                if (e.get("decrypted") or {}).get("body") == body:
                    return e
            self.frame(1)
        raise RuntimeError(f"gomuks never decrypted {body!r}")

    def close(self):
        self.ws.close()
        self.p.terminate()
        try:
            self.p.wait(timeout=12)
        except subprocess.TimeoutExpired:
            self.p.kill()


SSO_USER = "ssouser"


class MockCas(BaseHTTPRequestHandler):
    """Just enough CAS for Synapse's SSO: every login succeeds as SSO_USER."""

    def do_GET(self):
        url = urlsplit(self.path)
        query = parse_qs(url.query)
        if url.path == "/cas/login" and "service" in query:
            service = query["service"][0]
            self.send_response(302)
            self.send_header("Location", service + ("&" if "?" in service else "?") + "ticket=ST-untethered")
            self.end_headers()
        elif url.path == "/cas/proxyValidate" and query.get("ticket") == ["ST-untethered"]:
            body = (f'<cas:serviceResponse xmlns:cas="http://www.yale.edu/tp/cas"><cas:authenticationSuccess>'
                    f'<cas:user>{SSO_USER}</cas:user><cas:attributes><cas:displayName>Single Sign-on</cas:displayName>'
                    f'</cas:attributes></cas:authenticationSuccess></cas:serviceResponse>').encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/xml")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *args):
        pass


@contextmanager
def private_keyring(tmp):
    """A private session bus with an unlocked GNOME Keyring, or None."""
    if not shutil.which("gnome-keyring-daemon") or not shutil.which("dbus-daemon"):
        yield None
        return
    # Unix socket paths are limited to 108 bytes.
    sockets = Path(tempfile.mkdtemp(prefix="utq", dir=os.environ.get("UNTETHERED_QA_SOCKETS")))
    home = tmp / "keyring-home"
    home.mkdir()
    bus = f"unix:path={sockets}/bus"
    env = {**os.environ, "DBUS_SESSION_BUS_ADDRESS": bus, "XDG_RUNTIME_DIR": str(sockets),
           "HOME": str(home), "XDG_DATA_HOME": str(home / ".local/share")}
    procs = [subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--nopidfile", "--address=" + bus],
                              env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)]
    try:
        for _ in range(100):
            if (sockets / "bus").is_socket():
                break
            time.sleep(.05)
        keyring = subprocess.Popen(["gnome-keyring-daemon", "--foreground", "--components=secrets", "--unlock"],
                                   env=env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL)
        procs.append(keyring)
        keyring.stdin.write(b"qa-keyring")
        keyring.stdin.close()
        for _ in range(100):
            owner = subprocess.run(["dbus-send", "--session", "--print-reply", "--dest=org.freedesktop.DBus",
                                    "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner",
                                    "string:org.freedesktop.secrets"], env=env, capture_output=True, text=True)
            if "true" in owner.stdout:
                break
            time.sleep(.1)
        else:
            raise RuntimeError("private GNOME Keyring didn't start")
        yield bus
    finally:
        for p in reversed(procs):
            p.terminate()
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                p.kill()
        shutil.rmtree(sockets, ignore_errors=True)


def register(hs, name):
    pwd = secrets.token_urlsafe(24)
    auth = http("POST", hs + "/_matrix/client/v3/register", json={
        "auth": {"type": "m.login.dummy"}, "username": name, "password": pwd}).json()
    return auth["user_id"], pwd, {"Authorization": "Bearer " + auth["access_token"]}


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--keep", action="store_true")
    args = args.parse_args()
    assert BACKEND.is_file(), "Run scripts/rust-client.sh once or set UNTETHERED_BACKEND"
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-sdk-", dir=os.environ.get("TMPDIR")))
    synapse = None
    peers = []
    stack = contextlib.ExitStack()
    try:
        subprocess.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config",
                        "-H", SERVER_NAME, "--report-stats", "no"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
        port = free_port()
        hs = f"http://127.0.0.1:{port}"
        cas = ThreadingHTTPServer(("127.0.0.1", free_port()), MockCas)
        threading.Thread(target=cas.serve_forever, daemon=True).start()
        stack.callback(cas.shutdown)
        (tmp / "local.yaml").write_text(f"""listeners:
  - port: {port}
    type: http
    tls: false
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [client, media]}}]
public_baseurl: {hs}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: []
rc_message: {{per_second: 1000, burst_count: 1000}}
rc_login: {{address: {{per_second: 1000, burst_count: 1000}}, account: {{per_second: 1000, burst_count: 1000}}, failed_attempts: {{per_second: 1000, burst_count: 1000}}}}
cas_config:
  enabled: true
  server_url: "http://127.0.0.1:{cas.server_address[1]}/cas"
  displayname_attribute: displayName
sso:
  # Redirect straight back to the client's loopback listener.
  client_whitelist: ["http://127.0.0.1:"]
""")
        synapse = subprocess.Popen([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "-c", "local.yaml"],
                                   cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(300):
            if synapse.poll() is not None:
                raise RuntimeError("Disposable homeserver exited")
            try:
                versions = http("GET", hs + "/_matrix/client/versions").json()
                break
            except requests.ConnectionError:
                time.sleep(.1)
        server_version = http("GET", hs + "/_matrix/federation/v1/version").json()["server"]["version"] \
            if False else subprocess.run([sys.executable, "-c", "import synapse;print(synapse.__version__)"],
                                         capture_output=True, text=True).stdout.strip()
        sss = versions.get("unstable_features", {}).get("org.matrix.simplified_msc3575")
        print(f"Synapse {server_version}; Simplified Sliding Sync advertised: {sss}", flush=True)

        # The account the matrix-sdk engine signs into, with recovery set up
        # by gomuks; that gomuks device also writes encrypted history.
        alice, alice_pwd, alice_hdr = register(hs, "sdk" + secrets.token_hex(3))
        bob, bob_pwd, bob_hdr = register(hs, "peer" + secrets.token_hex(3))
        # No cross-signing keys or recovery: for setting recovery up.
        fresh, fresh_pwd, _ = register(hs, "fresh" + secrets.token_hex(3))

        def create(hdr, **body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def join(hdr, room):
            http("POST", f"{hs}/_matrix/client/v3/join/{quote(room, safe='')}", headers=hdr, json={})

        def put_state(hdr, room, kind, key, content):
            http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/state/{kind}/{quote(key, safe='')}",
                 headers=hdr, json=content)

        room = create(alice_hdr, name="sdk-proof", preset="private_chat", invite=[bob], initial_state=[
            {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}])
        join(bob_hdr, room)
        space = create(alice_hdr, name="SDK Space", preset="private_chat", creation_content={"type": "m.space"})
        put_state(alice_hdr, space, "m.space.child", room, {"via": [SERVER_NAME]})
        dm = create(bob_hdr, preset="trusted_private_chat", is_direct=True, invite=[alice])
        join(alice_hdr, dm)
        http("PUT", f"{hs}/_matrix/client/v3/user/{quote(alice, safe='')}/account_data/m.direct",
             headers=alice_hdr, json={bob: [dm]})
        unread = create(alice_hdr, name="mentions", preset="private_chat", invite=[bob])
        join(bob_hdr, unread)
        http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(unread, safe='')}/send/m.room.message/{secrets.token_hex(8)}",
             headers=bob_hdr, json={"msgtype": "m.text", "body": f"{alice} ping", "m.mentions": {"user_ids": [alice]}})

        seed = Gomuks(tmp / "seed", hs, alice, alice_pwd)
        peers.append(seed)
        key = seed.recovery(alice_pwd)
        seed.joined(room)
        history = 55
        for i in range(history):
            seed.rpc("send_message", {"room_id": room, "text": f"history {i}"})
        seed.decrypted(f"history {history - 1}")
        old_event = seed.decrypted("history 3")["event_id"]
        time.sleep(3)  # let gomuks upload the new Megolm session to key backup
        seed.close()
        peers.remove(seed)

        peer = Gomuks(tmp / "peer", hs, bob, bob_pwd)
        peers.append(peer)
        peer.recovery(bob_pwd)
        peer.joined(room)

        signals = tmp / "signals"
        signals.mkdir()
        fixture = tmp / "fixture.json"
        fixture.write_text(json.dumps(dict(server=hs, username=alice.split(":")[0][1:], password=alice_pwd, key=key,
                                           room=room, space=space, dm=dm, unread_room=unread, peer=bob,
                                           history=history, fresh_username=fresh.split(":")[0][1:],
                                           fresh_password=fresh_pwd, sso_user=f"@{SSO_USER}:{SERVER_NAME}",
                                           old_event=old_event)))
        bus = stack.enter_context(private_keyring(tmp))
        isolated = {**os.environ, "UNTETHERED_QA_FIXTURE": str(fixture)}
        if bus:
            isolated.update(DBUS_SESSION_BUS_ADDRESS=bus, UNTETHERED_QA_KEYRING="1")
            print("Private GNOME Keyring on a private session bus", flush=True)
        else:
            isolated.update(UNTETHERED_KEYRING="off")
            isolated.pop("DBUS_SESSION_BUS_ADDRESS", None)
            print("gnome-keyring-daemon not found; testing the file fallback only", flush=True)
        test = subprocess.Popen(["cargo", "test", "--locked", "--manifest-path", "rust-client/Cargo.toml",
                                 "matrix_sdk_login_restore_and_encrypted_round_trip", "--", "--ignored", "--nocapture"],
                                cwd=ROOT, env={**isolated, "UNTETHERED_QA_SIGNALS": str(signals),
                                               "UNTETHERED_PROFILE": str(tmp / "native")})
        failed = []
        while not (signals / "sdk-sent").is_file():
            if test.poll() is not None:
                raise SystemExit(f"Rust test exited early ({test.returncode})")
            peer.frame(.5)
        mine = (signals / "sdk-sent").read_text()
        event = peer.decrypted(mine)
        assert event["type"] == "m.room.encrypted" and event["sender"] == alice, event
        assert "<strong>matrix-sdk</strong>" in event["decrypted"].get("formatted_body", ""), event["decrypted"]
        print(f"gomuks peer decrypted the matrix-sdk message {event['event_id']} with its Markdown", flush=True)
        theirs = f"gomuks to matrix-sdk {secrets.token_hex(6)}"
        peer.rpc("send_message", {"room_id": room, "text": theirs})
        (signals / "peer-sent").write_text(theirs)
        while not (signals / "sdk-acted").is_file():
            if test.poll() is not None:
                break
            peer.frame(.5)
        if (signals / "sdk-acted").is_file():
            acted = json.loads((signals / "sdk-acted").read_text())
            content = lambda e: e.get("decrypted") or e.get("content") or {}
            peer.find("the edit", lambda e: content(e).get("m.new_content", {}).get("body") == acted["edit"]
                      and content(e).get("m.relates_to", {}).get("event_id") == acted["edited"])
            peer.find("the reaction", lambda e: "m.reaction" in (e.get("type"), e.get("decrypted_type"))
                      and content(e).get("m.relates_to", {}).get("event_id") == acted["reacted"]
                      and content(e).get("m.relates_to", {}).get("key") == acted["key"])
            (signals / "peer-confirmed").write_text("")
            print("gomuks peer decrypted the matrix-sdk edit and reaction", flush=True)
        while test.poll() is None:
            peer.frame(.5)
        if (signals / "sdk-reported").is_file():
            reported = (signals / "sdk-reported").read_text()
            db = sqlite3.connect(tmp / "homeserver.db")
            reports = db.execute("SELECT reason FROM event_reports WHERE event_id = ? AND user_id = ?",
                                 (reported, alice)).fetchall()
            receipts = db.execute("SELECT receipt_type FROM receipts_linearized WHERE event_id = ? AND user_id = ?",
                                  (reported, alice)).fetchall()
            db.close()
            assert reports == [("qa report",)], reports
            assert ("m.read.private",) in receipts, receipts
            print("Synapse holds the report and the private read receipt", flush=True)
        if test.returncode:
            failed.append("matrix_sdk_login_restore_and_encrypted_round_trip")
        if os.environ.get("UNTETHERED_QA_ONLY") == "round-trip":
            if failed:
                raise SystemExit("FAILED: " + ", ".join(failed))
            print("PASS round trip only", flush=True)
            return
        flows = {**isolated, "UNTETHERED_KEYRING": "off", "UNTETHERED_QA_PROFILES": str(tmp / "profiles")}
        flows.pop("UNTETHERED_QA_KEYRING", None)
        if subprocess.run(["cargo", "test", "--locked", "--manifest-path", "rust-client/Cargo.toml",
                           "matrix_sdk_account_flows", "--", "--ignored", "--nocapture"],
                          cwd=ROOT, env=flows).returncode:
            failed.append("matrix_sdk_account_flows")
        if failed:
            raise SystemExit("FAILED: " + ", ".join(failed))
        events = http("GET", f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}/messages",
                      headers=alice_hdr, params={"dir": "b", "limit": 100}).json()["chunk"]
        assert sum(e["type"] == "m.room.encrypted" for e in events) >= history + 2
        assert all(e["type"] != "m.room.message" for e in events), "plaintext reached the encrypted room"
        print("PASS matrix-sdk engine: login, recovery, sync, pagination, encrypted send/receive, restart, "
              "keyring, emoji verification, devices, recovery setup/change/reset and sign-out", flush=True)
    finally:
        stack.close()
        for p in peers:
            p.close()
        if synapse:
            synapse.terminate()
            try:
                synapse.wait(timeout=12)
            except subprocess.TimeoutExpired:
                synapse.kill()
        if args.keep:
            print("Kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
