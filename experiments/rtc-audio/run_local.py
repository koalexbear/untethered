"""Run the disposable two-peer probe using a local livekit-server executable."""
import base64
import hashlib
import hmac
import json
import ipaddress
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time


def main():
    crate = Path(__file__).resolve().parent
    server = shutil.which(sys.argv[1] if len(sys.argv) > 1 else "livekit-server")
    if not server:
        raise SystemExit("Pass the path to a local livekit-server executable.")
    subprocess.run(["cargo", "build", "--locked", "-j2"], cwd=crate, check=True)
    binary = Path(os.environ.get("CARGO_TARGET_DIR", crate / "target")) / "debug/untethered-rtc-probe"
    port = int(os.environ.get("RTC_PROBE_PORT", 18820))
    candidate = ipaddress.IPv4Address(os.environ.get("RTC_PROBE_ICE_IP", "127.0.0.1"))
    room = "untethered-" + secrets.token_hex(6)
    secret = secrets.token_hex(32)

    def token(i):
        def encode(data):
            return base64.urlsafe_b64encode(data).rstrip(b"=")
        claims = {"iss": "testkey", "sub": f"probe-{i}", "exp": int(time.time()) + 120,
                  "video": {"roomJoin": True, "room": room, "canPublish": True, "canSubscribe": True}}
        data = encode(b'{"alg":"HS256","typ":"JWT"}') + b"." + encode(json.dumps(claims).encode())
        return (data + b"." + encode(hmac.digest(secret.encode(), data, hashlib.sha256))).decode()

    with tempfile.TemporaryDirectory() as tmp:
        cfg = Path(tmp) / "livekit.yaml"
        cfg.write_text(f"""port: {port}
bind_addresses: ["127.0.0.1"]
keys: {{testkey: "{secret}"}}
rtc:
  tcp_port: {port+1}
  udp_port: {port+2}
  node_ip: {candidate}
  enable_loopback_candidate: true
  ips: {{includes: ["{candidate}/32"]}}
""")
        with open(Path(tmp) / "server.log", "w") as log:
            child = subprocess.Popen([server, "--config", str(cfg)], stdout=log, stderr=log)
            try:
                for _ in range(80):
                    if child.poll() is not None:
                        raise RuntimeError("Test SFU did not start; check that the three test ports are free.")
                    try:
                        with socket.create_connection(("127.0.0.1", port), .25):
                            break
                    except OSError:
                        time.sleep(.1)
                subprocess.run([binary.resolve()], check=True, timeout=60, env={
                    **os.environ, "RTC_PROBE_PORT": str(port), "RTC_TOKEN_A": token(0), "RTC_TOKEN_B": token(1)})
            finally:
                child.terminate()
                child.wait(timeout=10)


if __name__ == "__main__":
    main()
