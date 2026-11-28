"""A room left elsewhere while the app was closed leaves the room list.

From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with pillow python scripts/qa_left_rooms.py

Starts a disposable loopback Synapse with one account that has rooms "alpha"
and "bravo", then runs the ignored Rust test `matrix_sdk_room_left_while_closed`
(engine/live_test.rs): the engine signs in and syncs, stops, another device of
the account leaves alpha (the room open, as the first listed), and the restarted
engine must drop it from its room list. Then the same for bravo after Synapse
has forgotten the engine's sliding sync connection (restarted without its
`sliding_sync_connection*` rows, as if it had expired), so the next sync starts
without a position and never mentions bravo. No gomuks and no encryption
setup: the room list doesn't need them.
Pass --keep to leave the profile and homeserver directory for inspection;
UNTETHERED_QA_LOG (an SDK log filter) logs to sdk.log there.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import http, register  # noqa: E402
from qa_media_previews import homeserver  # noqa: E402


def forget_sliding_sync(tmp, synapse, hs):
    """Restart Synapse without any sliding sync connection: what it does to a
    connection unused for a week (or an hour, for a busy account)."""
    synapse.terminate()
    synapse.wait(timeout=30)
    db = sqlite3.connect(tmp / "homeserver.db")
    tables = [name for (name,) in db.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'sliding_sync_connection%'")]
    assert "sliding_sync_connection_positions" in tables, tables
    for table in tables:
        db.execute(f"DELETE FROM {table}")
    db.commit()
    db.close()
    synapse = subprocess.Popen([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "-c", "local.yaml"],
                               cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(300):
        if synapse.poll() is not None:
            raise RuntimeError("Disposable homeserver exited")
        try:
            http("GET", hs + "/_matrix/client/versions")
            return synapse
        except requests.ConnectionError:
            time.sleep(.1)
    raise RuntimeError("Disposable homeserver didn't come back")


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--keep", action="store_true")
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-left-", dir=os.environ.get("TMPDIR")))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        user, pwd, hdr = register(hs, "leaver")
        for name in ("alpha", "bravo"):
            http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr,
                 json={"name": name, "preset": "private_chat"})
        fixture = tmp / "fixture.json"
        fixture.write_text(json.dumps(dict(server=hs, username=user.split(":")[0][1:], password=pwd,
                                           token=hdr["Authorization"].split()[1], signals=str(tmp))))
        env = {**os.environ, "UNTETHERED_QA_FIXTURE": str(fixture), "UNTETHERED_PROFILE": str(tmp / "profile"),
               "UNTETHERED_KEYRING": "off", "UNTETHERED_QA_LOG_FILE": str(tmp / "sdk.log")}
        env.pop("DBUS_SESSION_BUS_ADDRESS", None)
        test = subprocess.Popen(["cargo", "test", "--locked", "--manifest-path", "rust-client/Cargo.toml",
                                 "matrix_sdk_room_left_while_closed", "--", "--ignored", "--nocapture"],
                                cwd=ROOT, env=env)
        while test.poll() is None:
            if (tmp / "expire").exists() and not (tmp / "expired").exists():
                synapse = forget_sliding_sync(tmp, synapse, hs)
                (tmp / "expired").write_text("")
            time.sleep(.2)
        if test.returncode:
            raise SystemExit("FAILED: matrix_sdk_room_left_while_closed")
        print("PASS a room left while the app was closed left the room list", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=10)
        if args.keep:
            print("Kept", tmp, flush=True)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
