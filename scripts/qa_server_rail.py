"""Server rail order and folders (ROADMAP N8), live against a disposable Synapse.

Build the debug client (it seeds the profiles) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_server_rail.py

The test account has five spaces, Kilo, Lima, Mike, Nova and Oscar (a
subspace of Kilo, which the rail lists too), with no order, as Element
leaves them, and two devices: the client profile the UI runs on and a
second one. The release client in a private KWin session (as qa_kwin.py
needs), driven with drags as qa_channel_order.py does them, must:
- drag the last server above the first: each written order is one MSC3230
  allows (printable ASCII, at most 50 characters), and sorting the
  top-level spaces as Element does (valid order by its bytes, then room ID)
  gives the rail's order;
- drop a server on another: a folder of the two in our account data
  (io.github.koalexbear.untethered.space_folders), its servers next to each
  other in Element's order; drop a third on the folder, open the folder,
  drag that one out again; name and colour the folder in Folder Settings;
- follow an order another client writes over the API, at once;
- show the same order and folder after a restart and on the second device;
- there, drag both servers out of the folder: an empty folder goes, from
  the rail and from the account data.
The rail's layout comes from the client's QA event log (`{"kind":"rail"}`
entries, rail_order.rs), since server icons are initials OCR reads badly.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote

import requests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_events import entries, use_profile  # noqa: E402
from qa_matrix_sdk import SERVER_NAME, http  # noqa: E402
from qa_media_previews import homeserver, interop  # noqa: E402

WINDOW = (0, 0, 1180, 900)
RAIL = 36
# The first server's icon, then one every 48 + 8 px (as qa_channel_order.py).
FIRST = 186
STEP = 56
ORDER = "org.matrix.msc3230.space_order"
FOLDERS = "io.github.koalexbear.untethered.space_folders"


def valid(order):
    """An order MSC3230 allows (and Element's validOrder accepts)."""
    return isinstance(order, str) and len(order) <= 50 and all(0x20 <= ord(c) <= 0x7E for c in order)


def element_order(roots, orders):
    """Top-level spaces as Element's SpaceStore sorts them (sortRootSpaces:
    lodash sortBy on [valid order, roomId], no order last)."""
    def key(room):
        order = orders.get(room)
        return (0, order, room) if valid(order) else (1, "", room)
    return sorted(roots, key=key)


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-rail-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        fixture, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        # A second device of the same account (engine/qa.rs signs it in and
        # verifies it with the recovery key).
        seeder = os.environ.get("UNTETHERED_QA_CLIENT") or str(ROOT / "rust-client/target/debug/untethered-iced")
        subprocess.run([seeder], check=True, timeout=360, env={
            **os.environ, "UNTETHERED_PROFILE": str(tmp / "seed2"), "UNTETHERED_QA_SEED": json.dumps(fixture),
            "UNTETHERED_KEYRING": "off"})

        names = {}

        def space(name):
            rid = http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json={
                "name": name, "preset": "private_chat", "creation_content": {"type": "m.space"}}).json()["room_id"]
            names[rid] = name
            return rid
        kilo, lima, mike, nova, oscar = (space(n) for n in ("Kilo", "Lima", "Mike", "Nova", "Oscar"))
        http("PUT", f"{hs}/_matrix/client/v3/rooms/{quote(kilo, safe='')}/state/m.space.child/{quote(oscar, safe='')}",
             headers=hdr, json={"via": [SERVER_NAME]})
        roots = [kilo, lima, mike, nova]
        user = f"{hs}/_matrix/client/v3/user/{quote(me, safe='')}"

        def read(url):
            """Account data over the API, or None where there is none (404)."""
            r = requests.get(url, headers=hdr, timeout=15)
            if r.status_code == 404:
                return None
            r.raise_for_status()
            return r.json()

        def orders():
            """Each space's MSC3230 order (None: none)."""
            return {room: (read(f"{user}/rooms/{quote(room, safe='')}/account_data/{ORDER}") or {}).get("order")
                    for room in names}

        def folders():
            return (read(f"{user}/account_data/{FOLDERS}") or {}).get("folders", [])

        def set_order(room, order):
            http("PUT", f"{user}/rooms/{quote(room, safe='')}/account_data/{ORDER}", headers=hdr,
                 json={"order": order})

        print(f"Synapse at {hs}: {me} with spaces {', '.join(names.values())} (Oscar in Kilo), "
              "and a second device", flush=True)
        look(tmp, args.shots, names, roots, orders, folders, set_order)
        print("PASS: servers dragged in the rail and folders made there are written as Element reads them "
              "(MSC3230) and in our own account data, picked up live, and the same after a restart and on a "
              "second device", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, names, roots, orders, folders, set_order):
    from qa_kwin import KWin
    from qa_media_previews import tap
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        profile = [tmp / "seed"]
        use_profile(env, profile[0])
        env.update(UNTETHERED_KEYRING="off")
        kde = KWin(stack, ui, ROOT, None, env=env)
        n = [0]

        def place():
            kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)
        place()

        def shot(name):
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                n[0] += 1
                im.save(shots / f"{n[0]:02d}-{name}.png")
            return im

        def layouts():
            return [e["entries"] for e in entries(profile[0], "rail")]

        def layout(since=0):
            """The rail's latest layout logged after `since` entries, once it
            lists every space."""
            logged = layouts()[since:]
            if not logged:
                return None
            last = logged[-1]
            listed = {s["space"] for e in last for s in (e.get("servers") or [e])}
            return last if listed >= set(names) else None

        def shape(entries_):
            """A layout by name, folders as (name, servers), open or not left out."""
            return [(e["name"], tuple(s["name"] for s in e["servers"])) if "folder" in e else e["name"]
                    for e in entries_]

        def flat(entries_):
            return [s["space"] for e in entries_ for s in (e.get("servers") or [e])]

        def rows(entries_):
            """The rail's icons top to bottom: ("server", id) or ("folder", id)."""
            out = []
            for e in entries_:
                if "folder" in e:
                    out.append(("folder", e["folder"]))
                    if e["open"]:
                        out += [("server", s["space"]) for s in e["servers"]]
                else:
                    out.append(("server", e["space"]))
            return out

        def y(entries_, kind, key):
            return FIRST + STEP * rows(entries_).index((kind, key))

        def away():
            kde.actions("pointer", [{"type": "pointerMove", "x": 700, "y": 600}])

        def drag(entries_, held, target, part):
            """Press `held`'s icon and drop it on the top, middle or bottom
            of `target`'s, as kde.drag does: while a button is down,
            inputsynth moves the pointer reliably only by one relative move
            with a duration."""
            sy = y(entries_, *held)
            ty = y(entries_, *target) + {"top": -17, "middle": 0, "bottom": 17}[part]
            kde.actions("pointer", [
                {"type": "pointerMove", "x": RAIL, "y": sy}, {"type": "pause", "duration": 300},
                {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 250},
                {"type": "pointerMove", "origin": "pointer", "x": 0, "y": ty - sy, "duration": 900},
                {"type": "pause", "duration": 400}, {"type": "pointerUp", "button": 0}])
            time.sleep(0.5)
            away()

        def changed(since, check, what, timeout=30):
            """Wait for a logged layout after `since` that passes `check`."""
            def ok():
                now = layout(since)
                return now if now and check(now) else None
            return wait_for(what, ok, timeout)

        def element_matches(now):
            """The rail's top-level spaces in Element's order of what's written."""
            written = orders()
            bad = {names[r]: o for r, o in written.items() if o is not None and not valid(o)}
            assert not bad, f"orders MSC3230 doesn't allow: {bad}"
            rail = [s for s in flat(now) if s in roots]
            return rail == element_order(roots, written)

        start = wait_for("every space in the rail", layout, 120)
        print("UI: the rail lists " + ", ".join(shape(start)) + " (by room ID, no orders yet)", flush=True)
        shot("start")

        # 1. The last server above the first.
        mark = len(layouts())
        first, last = flat(start)[0], flat(start)[-1]
        drag(start, ("server", last), ("server", first), "top")
        try:
            now = changed(mark, lambda l: flat(l)[0] == last, f"{names[last]} first in the rail")
            wait_for("Element's order to match the rail", lambda: element_matches(layout(mark)), 30)
        finally:
            shot("moved")
        written = {names[r]: o for r, o in orders().items() if o is not None}
        print(f"Matrix: {names[last]} dragged to the top; orders written {written}; Element sorts the top-level "
              f"spaces as the rail shows them: {', '.join(names[s] for s in flat(now) if s in roots)}", flush=True)

        # 2. A folder: the fourth icon dropped on the second.
        mark = len(layouts())
        a, b = flat(now)[1], flat(now)[3]
        drag(now, ("server", b), ("server", a), "middle")
        try:
            now = changed(mark, lambda l: any("folder" in e and [s["space"] for s in e["servers"]] == [a, b]
                                              for e in l), f"a folder of {names[a]} and {names[b]}")
            wait_for("the folder in account data", lambda: [f["spaces"] for f in folders()] in
                     ([[a, b]], [[b, a]]), 30)
            wait_for("Element's order to match", lambda: element_matches(layout(mark)), 30)
        finally:
            shot("folder")
        folder = next(e["folder"] for e in now if "folder" in e)
        order = [s for s in flat(now)]
        assert abs(order.index(a) - order.index(b)) == 1, order
        print(f"UI: {names[b]} dropped on {names[a]} makes a folder: rail {shape(now)}; account data "
              f"{folders()}", flush=True)

        # 3. A third server dropped on the closed folder joins it.
        mark = len(layouts())
        c = next(e["space"] for e in reversed(now) if "space" in e)
        drag(now, ("server", c), ("folder", folder), "middle")
        try:
            now = changed(mark, lambda l: any(e.get("folder") == folder and len(e["servers"]) == 3 for e in l),
                          f"{names[c]} in the folder")
            wait_for("three servers in the folder's account data",
                     lambda: [len(f["spaces"]) for f in folders()] == [3], 30)
        finally:
            shot("third-in")
        print(f"UI: {names[c]} dropped on the folder joins it: {shape(now)}", flush=True)

        # 4. Open the folder, and drag that server out below the last icon.
        mark = len(layouts())
        tap(kde, RAIL, y(now, "folder", folder))
        away()
        now = changed(mark, lambda l: any(e.get("folder") == folder and e["open"] for e in l), "the folder open")
        shot("open")
        mark = len(layouts())
        bottom = rows(now)[-1]
        drag(now, ("server", c), bottom, "bottom")
        try:
            now = changed(mark, lambda l: flat(l)[-1] == c and "space" in l[-1], f"{names[c]} out of the folder")
            wait_for("two servers in the folder's account data",
                     lambda: [sorted(f["spaces"]) for f in folders()] == [sorted([a, b])], 30)
            wait_for("Element's order to match", lambda: element_matches(layout(mark)), 30)
        finally:
            shot("third-out")
        print(f"UI: {names[c]} dragged out of the open folder: {shape(now)}", flush=True)

        # 5. Folder Settings, from the folder's right-click menu: name, colour.
        fy = y(now, "folder", folder)
        kde.actions("pointer", [{"type": "pointerMove", "x": RAIL, "y": fy}, {"type": "pause", "duration": 250},
                                {"type": "pointerDown", "button": 2}, {"type": "pause", "duration": 80},
                                {"type": "pointerUp", "button": 2}])

        def word(region, text):
            found = [(x, y_) for w, x, y_ in kde.words(region) if w.strip(".,:").lower() == text.lower()]
            return found[0] if found else None
        try:
            mx, my = wait_for("Folder Settings in the menu", lambda: word((0, fy, 330, fy + 120), "settings"), 15)
        finally:
            shot("menu")
        tap(kde, mx, my)
        try:
            label = wait_for("Folder Settings dialog", lambda: word((300, 150, 900, 750), "color"), 15)
        finally:
            shot("settings")
        kde.type("Games")
        # The fourth swatch (0x3498db), 28 px squares 8 px apart, under the label.
        lx, ly = label
        fx = wait_for("the label's first word", lambda: word((300, ly - 15, 900, ly + 15), "folder"), 10)[0]
        tap(kde, fx - 12 + 14 + 3 * 36, ly + 30)
        done = wait_for("Done", lambda: word((300, 150, 900, 800), "done"), 10)
        shot("named")
        mark = len(layouts())
        tap(kde, *done)
        away()
        now = changed(mark, lambda l: any(e.get("folder") == folder and e["name"] == "Games" for e in l),
                      "the folder named")
        wait_for("name and colour in account data", lambda: [(f["name"], f["color"]) for f in folders()] ==
                 [("Games", "#3498db")], 30)
        shot("renamed")
        print(f"UI: Folder Settings named it Games, colour #3498db: {folders()}", flush=True)

        # 6. An order another client writes is picked up live.
        mark = len(layouts())
        top = next(e["space"] for e in reversed(now) if "space" in e)
        set_order(top, "!")
        now = changed(mark, lambda l: flat(l)[0] == top, f"{names[top]} first after another client's write", 20)
        assert element_matches(now)
        shot("api-order")
        print(f"UI: another client's order for {names[top]} moves it to the top at once: {shape(now)}", flush=True)
        expected = shape(now)

        # 7. After a restart, and on the second device: the same rail.
        mark = len(layouts())
        kde.restart_native()
        place()
        again = wait_for("the rail after a restart", lambda: layout(mark), 120)
        again = wait_for("the folder after a restart", lambda: (lambda l: l if l and shape(l) == expected
                                                                else None)(layout(mark)), 60)
        shot("restarted")
        print(f"UI: after a restart the rail is the same: {shape(again)}", flush=True)
        profile[0] = tmp / "seed2"
        args, native_env, stderr = kde.native_args
        native_env = use_profile(dict(native_env), profile[0])
        kde.native_args = (args, native_env, stderr)
        kde.restart_native()
        place()
        second = wait_for("the rail on the second device", lambda: (lambda l: l if l and shape(l) == expected
                                                                    else None)(layout()), 150)
        shot("second-device")
        print(f"UI: the second device shows the same rail: {shape(second)}", flush=True)

        # 8. There, both servers dragged out of the folder: it goes.
        mark = len(layouts())
        tap(kde, RAIL, y(second, "folder", folder))
        away()
        now = changed(mark, lambda l: any(e.get("folder") == folder and e["open"] for e in l), "the folder open")
        for i in range(2):
            mark = len(layouts())
            inside = next(e for e in now if e.get("folder") == folder)["servers"]
            held = inside[-1]["space"]
            drag(now, ("server", held), rows(now)[-1], "bottom")
            try:
                now = changed(mark, lambda l: flat(l)[-1] == held and "space" in l[-1],
                              f"{names[held]} out of the folder")
            finally:
                shot(f"out-{i + 1}")
        assert not any("folder" in e for e in now), now
        wait_for("no folder in account data", lambda: folders() == [], 30)
        wait_for("Element's order to match", lambda: element_matches(layout()), 30)
        print(f"UI: with both servers dragged out the folder is gone, here and in account data: {shape(now)}",
              flush=True)


if __name__ == "__main__":
    main()
