"""Upgraded rooms as one channel, against a disposable Synapse.

From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    python scripts/qa_room_upgrades.py

Seeds three upgrade chains through the client-server API, then runs the
ignored `matrix_sdk_room_upgrades` test, which drives the real App on the
matrix-sdk engine:
- "general", upgraded twice (room versions 10, 11, 12) with messages
  before, between and after, listed in a space by its first version: the
  list shows only the newest room, in the space too, and scrolling back
  reaches the first version's messages past two dividers. A Matrix link into
  the first version opens the merged channel at that message, and search
  finds old messages.
- "lounge", upgraded by the peer, who invites us to the new room: the old
  room stays with its "Join the new room" offer, which joins and hides it.
- "archive", upgraded by the peer before we joined: its history isn't ours
  to read, so the divider says so.
- "patio", like the lounge, left with its offer for a look at the UI.
Pass --ui to then open the profile in the release client in a private KWin
session (KWin, inputsynth, spectacle and tesseract, as qa_kwin.py needs):
"general" is listed once, scrolling it back shows the dividers, and the
patio offers its new room. --shots DIR keeps the screenshots. Pass --keep to
keep the profile (its recovery key is in `recovery-key`) and the homeserver
running; Ctrl+C stops it.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote

import requests

from qa_matrix_sdk import SERVER_NAME, free_port, http, register

ROOT = Path(__file__).resolve().parents[1]


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--keep", action="store_true")
    args.add_argument("--ui", action="store_true")
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-upgrades-", dir=os.environ.get("TMPDIR")))
    synapse = None
    try:
        subprocess.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config",
                        "-H", SERVER_NAME, "--report-stats", "no"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
        port = free_port()
        hs = f"http://127.0.0.1:{port}"
        limit = "{per_second: 1000, burst_count: 1000}"
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
rc_message: {limit}
rc_room_creation: {limit}
rc_invites: {{per_room: {limit}, per_user: {limit}, per_issuer: {limit}}}
rc_joins: {{local: {limit}, remote: {limit}}}
rc_login: {{address: {limit}, account: {limit}, failed_attempts: {limit}}}
""")
        synapse = subprocess.Popen([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "-c", "local.yaml"],
                                   cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(300):
            if synapse.poll() is not None:
                raise RuntimeError("Disposable homeserver exited")
            try:
                http("GET", hs + "/_matrix/client/versions")
                break
            except requests.ConnectionError:
                time.sleep(.1)

        alice, alice_pwd, alice_hdr = register(hs, "sdk" + secrets.token_hex(3))
        bob, _, bob_hdr = register(hs, "peer" + secrets.token_hex(3))

        def room_path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def create(hdr, **body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def join(hdr, room):
            http("POST", f"{hs}/_matrix/client/v3/join/{quote(room, safe='')}", headers=hdr, json={})

        def invite(hdr, room, user):
            http("POST", room_path(room) + "/invite", headers=hdr, json={"user_id": user})

        def say(hdr, room, body):
            return http("PUT", room_path(room) + f"/send/m.room.message/{secrets.token_hex(8)}", headers=hdr,
                        json={"msgtype": "m.text", "body": body}).json()["event_id"]

        def upgrade(hdr, room, version):
            r = requests.post(room_path(room) + "/upgrade", headers=hdr, json={"new_version": version}, timeout=30)
            if not r.ok:
                raise RuntimeError(f"upgrade to {version} refused: {r.status_code} {r.text}")
            return r.json()["replacement_room"]

        # general (room version 10): before → v11 → between → v12 → after, in a
        # space by its first version.
        general = create(alice_hdr, name="general", preset="private_chat", invite=[bob], room_version="10")
        join(bob_hdr, general)
        space = create(alice_hdr, name="Upgrade Space", preset="private_chat", creation_content={"type": "m.space"})
        http("PUT", room_path(space) + f"/state/m.space.child/{quote(general, safe='')}", headers=alice_hdr,
             json={"via": [SERVER_NAME]})
        before = [say(alice_hdr if i % 2 else bob_hdr, general, f"before {i}") for i in range(1, 61)]
        general_v2 = upgrade(alice_hdr, general, "11")
        between = [say(alice_hdr, general_v2, f"between {i}") for i in range(1, 4)]
        general_v3 = upgrade(alice_hdr, general_v2, "12")
        after = [say(alice_hdr, general_v3, f"after {i}") for i in range(1, 6)]

        # lounge: the peer upgrades and invites us to the new room.
        lounge = create(bob_hdr, name="lounge", preset="private_chat", invite=[alice], room_version="10")
        join(alice_hdr, lounge)
        say(bob_hdr, lounge, "lounge before")
        lounge_new = upgrade(bob_hdr, lounge, "11")
        invite(bob_hdr, lounge_new, alice)

        # archive: upgraded before we were ever in it.
        archive = create(bob_hdr, name="archive", preset="private_chat", room_version="10")
        say(bob_hdr, archive, "archive secret")
        archive_new = upgrade(bob_hdr, archive, "11")
        invite(bob_hdr, archive_new, alice)
        join(alice_hdr, archive_new)
        say(bob_hdr, archive_new, "archive after")

        # patio: like the lounge, but left with its offer.
        patio = create(bob_hdr, name="patio", preset="private_chat", invite=[alice], room_version="10")
        join(alice_hdr, patio)
        say(bob_hdr, patio, "patio before")
        patio_new = upgrade(bob_hdr, patio, "11")
        invite(bob_hdr, patio_new, alice)

        fixture = tmp / "fixture.json"
        fixture.write_text(json.dumps({
            "server": hs, "username": alice, "password": alice_pwd, "peer": bob, "space": space,
            "general": [general, general_v2, general_v3], "before": before, "between": between, "after": after,
            "lounge": [lounge, lounge_new], "archive": [archive, archive_new], "patio": [patio, patio_new],
        }))
        print(f"Synapse at {hs}: general {general} → {general_v2} → {general_v3}", flush=True)
        env = {**os.environ, "UNTETHERED_QA_FIXTURE": str(fixture), "UNTETHERED_KEYRING": "off",
               "UNTETHERED_QA_PROFILES": str(tmp / "profiles")}
        test = subprocess.run(["cargo", "test", "--manifest-path", "rust-client/Cargo.toml",
                               "matrix_sdk_room_upgrades", "--", "--ignored", "--nocapture"], cwd=ROOT, env=env)
        if test.returncode:
            raise SystemExit("matrix_sdk_room_upgrades failed")
        print("PASS: upgraded rooms read as one channel", flush=True)
        if args.ui:
            look(tmp, args.shots)
        if args.keep:
            print(f"Profile: {tmp / 'profiles' / 'upgrades'} (UNTETHERED_KEYRING=off); homeserver {hs}. "
                  "Ctrl+C stops.", flush=True)
            synapse.wait()
    except KeyboardInterrupt:
        pass
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        if not args.keep:
            shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots):
    """The test's profile in the release client, in a private KWin session."""
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = ROOT / "rust-client/target/release/untethered-iced"
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "profiles" / "upgrades"), UNTETHERED_KEYRING="off")
        kde = KWin(stack, ui, ROOT, None, env=env)
        def window():
            assert kde.native.poll() is None, "the client exited"
            return kde.window("Untethered")
        wid = wait_for("client window", window, 60)
        kde.position(wid, 0, 0, 1180, 900)
        def shot(name):
            im = kde.capture().crop((0, 0, 1180, 900))
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im
        def words(rect=(0, 0, 1180, 900)):
            return kde.words(rect)
        def sidebar():
            names = [w.strip(".,#|").lower() for w, x, _ in words() if 72 <= x < 330]
            return names if "patio" in names and "general" in names else None
        names = wait_for("rooms in the sidebar", sidebar, 60)
        assert names.count("general") == 1, names
        assert "lounge" in names and "archive" in names, names
        print(f"UI: the sidebar lists general once: {sorted(set(names))}", flush=True)
        def find(word, x_max=330):
            return next(((x, y) for w, x, y in words() if w.strip(".,#|").lower() == word and x < x_max), None)
        kde.click(*wait_for("general in the sidebar", lambda: find("general"), 20))
        def divider():
            ws = [w.lower() for w, x, _ in words() if x >= 330]
            return ws if "upgraded" in ws else None
        # History fills a screen on open; Page Up reaches the dividers.
        found = None
        for _ in range(12):
            found = divider()
            if found and "before" in found:
                break
            kde.keys(("Down", "\ue00e"), ("Up", "\ue00e"))
            time.sleep(1.2)
        shot("general-divider")
        assert found and "before" in found, found
        print("UI: scrolling general back shows a Room upgraded divider above the newest messages, "
              "with the first version's messages beyond it", flush=True)
        kde.click(*wait_for("patio in the sidebar", lambda: find("patio"), 20))
        def offer():
            ws = [w.lower() for w, x, _ in words() if x >= 330]
            return ws if "join" in ws and "upgraded" in ws else None
        wait_for("Join the new room offer", offer, 20)
        shot("patio-offer")
        print("UI: the patio shows This room was upgraded with Join the new room", flush=True)


if __name__ == "__main__":
    main()
