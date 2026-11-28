"""Channel order (ROADMAP N5), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_channel_order.py

The test account's space, Workshop, lists general, design and random, and two
subspaces, Projects (alpha, beta) and Archive (old-notes), as Element makes
them: `m.space.child` edges with no `order`. The sidebar shows the subspaces
as categories. The release client in a private KWin session (as qa_kwin.py
needs) must:
- drag random above general: Workshop's edges, sorted the way the spec says
  and Element's space view does, list random first;
- drag design below alpha, into Projects: the edge moves from Workshop to
  Projects (its `via` kept), which lists alpha, design, beta;
- drag the Archive category above Projects: Workshop lists Archive first.
Each order is read back over the client-server API (`/state` and the
`/hierarchy` Element's space home reads), and the sidebar shows it.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import SERVER_NAME, http  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402

WINDOW = (0, 0, 1180, 900)
SIDEBAR = (72, 40, 330, 840)
RAIL = 36
FIRST_SPACE = 186
# Rail icons are 48 px apart by 8 px.
RAIL_STEP = 56


def element_order(edges):
    """Children as Element's space view sorts them (SpaceStore's
    getChildOrder): a valid `order` first, then the edge's timestamp, then
    the room ID."""
    def key(e):
        order = e["content"].get("order")
        valid = isinstance(order, str) and len(order) <= 50 and all(0x20 <= ord(c) <= 0x7E for c in order)
        return (0, order, e["origin_server_ts"], e["state_key"]) if valid else \
            (1, "", e["origin_server_ts"], e["state_key"])
    return [e["state_key"] for e in sorted(edges, key=key)]


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-order-"))
    synapse = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]

        def create(**body):
            return http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=body).json()["room_id"]

        def path(room):
            return f"{hs}/_matrix/client/v3/rooms/{quote(room, safe='')}"

        def link(space, child):
            http("PUT", f"{path(space)}/state/m.space.child/{quote(child, safe='')}", headers=hdr,
                 json={"via": [SERVER_NAME]})
            time.sleep(0.05)  # distinct timestamps, as edges made one by one

        names = {}

        def room(name, space=False):
            body = dict(name=name, preset="private_chat")
            if space:
                body["creation_content"] = {"type": "m.space"}
            names[rid := create(**body)] = name
            return rid
        workshop = room("Workshop", space=True)
        general, design, random = room("general"), room("design"), room("random")
        projects, archive = room("Projects", space=True), room("Archive", space=True)
        alpha, beta, notes = room("alpha"), room("beta"), room("old-notes")
        for child in (general, design, random, projects, archive):
            link(workshop, child)
        for child in (alpha, beta):
            link(projects, child)
        link(archive, notes)
        http("PUT", f"{path(general)}/send/m.room.message/hello", headers=hdr,
             json={"msgtype": "m.text", "body": "Welcome to the workshop"})

        def edges(space):
            state = http("GET", f"{path(space)}/state", headers=hdr).json()
            return [e for e in state if e["type"] == "m.space.child" and e["content"].get("via")]

        def listed(space):
            """The space's children by name, as Element's space view orders them."""
            return [names.get(c, c) for c in element_order(edges(space))]

        def hierarchy(space):
            """The same from `/hierarchy`, what Element's space home reads."""
            rooms = http("GET", f"{hs}/_matrix/client/v1/rooms/{quote(space, safe='')}/hierarchy?max_depth=1",
                         headers=hdr).json()["rooms"]
            top = next(r for r in rooms if r["room_id"] == space)
            return [names.get(c, c) for c in element_order(top["children_state"])]

        assert listed(workshop) == ["general", "design", "random", "Projects", "Archive"], listed(workshop)
        print(f"Synapse at {hs}: {me}'s Workshop with categories Projects and Archive", flush=True)
        look(tmp, args.shots, workshop, projects, listed, hierarchy, edges)
        print("PASS: channels and categories dragged in the sidebar come back over the client-server API in "
              "that order, sorted as Element's space view sorts them", flush=True)
    finally:
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


def look(tmp, shots, workshop, projects, listed, hierarchy, edges):
    from qa_kwin import KWin
    from qa_native_paste import isolated_env, wait_for
    release = Path(os.environ.get("UNTETHERED_QA_UI_CLIENT") or ROOT / "rust-client/target/release/untethered-iced")
    assert release.is_file(), "cargo build --release first (debug builds panic under synthetic input)"
    os.environ["UNTETHERED_QA_CLIENT"] = str(release)
    ui = tmp / "ui"
    ui.mkdir()
    with contextlib.ExitStack() as stack:
        env = isolated_env(ui, None, "wayland")
        env.update(UNTETHERED_PROFILE=str(tmp / "seed"), UNTETHERED_KEYRING="off")
        kde = KWin(stack, ui, ROOT, None, env=env)
        kde.position(wait_for("client window", lambda: kde.window("Untethered"), 60), *WINDOW)

        def shot(name):
            im = kde.capture().crop(WINDOW)
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def rows():
            """The sidebar's words, top to bottom."""
            found = [(w.strip(".,:|?!#“”").lower(), x, y) for w, x, y in kde.words(SIDEBAR)]
            return sorted(found, key=lambda r: r[2])

        def where(name):
            return next(((x, y) for w, x, y in rows() if w == name.lower()), None)

        def order(*names):
            """These names in the sidebar, top to bottom, or None."""
            found = {w: y for w, _, y in rows()}
            if not all(n.lower() in found for n in names):
                return None
            return sorted(names, key=lambda n: found[n.lower()])

        def away():
            kde.actions("pointer", [{"type": "pointerMove", "x": 700, "y": 600}])

        def drag(name, target, lower, heading=False):
            """Press `name`'s row and drop it on the upper or lower half of
            `target`'s, as kde.drag does: while a button is down, inputsynth
            moves the pointer reliably only by one relative move with a
            duration. A heading's text sits low in its 40 px row."""
            (sx, sy), (tx, ty) = where(name), where(target)
            ty += 7 if lower else -18 if heading else -7
            kde.actions("pointer", [
                {"type": "pointerMove", "x": sx, "y": sy}, {"type": "pause", "duration": 300},
                {"type": "pointerDown", "button": 0}, {"type": "pause", "duration": 250},
                {"type": "pointerMove", "origin": "pointer", "x": tx - sx, "y": ty - sy, "duration": 900},
                {"type": "pause", "duration": 400}, {"type": "pointerUp", "button": 0}])
            time.sleep(0.5)
            away()

        wait_for("media-proof", lambda: where("media-proof"), 90)
        # The rail lists every joined space; Workshop is the one with categories.
        for i in range(3):
            tap(kde, RAIL, FIRST_SPACE + i * RAIL_STEP)
            away()
            try:
                wait_for("Workshop's sidebar", lambda: order("general", "projects", "archive"), 10)
                break
            except RuntimeError:
                if i == 2:
                    raise
        try:
            assert wait_for("the edges' order", lambda: order(
                "general", "design", "random", "projects", "alpha", "beta", "archive"), 20) == [
                "general", "design", "random", "projects", "alpha", "beta", "archive"], rows()
        finally:
            shot("workshop")
        print("UI: Workshop shows its own channels in the edges' order, then Projects and Archive as "
              "categories", flush=True)

        # Within the server's own channels: random above general.
        drag("random", "general", lower=False)
        try:
            wait_for("random first over the API", lambda: listed(workshop)[0] == "random", 30)
            wait_for("random first in the sidebar",
                     lambda: order("random", "general", "design") == ["random", "general", "design"], 20)
        finally:
            shot("random-first")
        assert listed(workshop) == ["random", "general", "design", "Projects", "Archive"], listed(workshop)
        assert hierarchy(workshop) == listed(workshop), hierarchy(workshop)
        written = [e["content"] for e in edges(workshop)]
        assert sum("order" in c for c in written) == 1, f"only random's edge gets an order: {written}"
        print(f"Matrix: Workshop lists {', '.join(listed(workshop))}; only random's edge was written",
              flush=True)

        # Into another category: design below alpha, in Projects.
        drag("design", "alpha", lower=True)
        try:
            wait_for("design in Projects over the API", lambda: listed(projects) == ["alpha", "design", "beta"], 30)
            wait_for("design gone from Workshop", lambda: "design" not in listed(workshop), 30)
            wait_for("design under alpha in the sidebar",
                     lambda: order("projects", "alpha", "design", "beta") == ["projects", "alpha", "design", "beta"],
                     20)
        finally:
            shot("design-moved")
        assert all(e["content"]["via"] == [SERVER_NAME] for e in edges(projects)), edges(projects)
        print(f"Matrix: Projects lists {', '.join(listed(projects))}, with design's via kept; Workshop "
              f"{', '.join(listed(workshop))}", flush=True)

        # A category: Archive above Projects.
        drag("archive", "projects", lower=False, heading=True)
        try:
            wait_for("Archive before Projects over the API",
                     lambda: listed(workshop).index("Archive") < listed(workshop).index("Projects"), 30)
            wait_for("Archive above Projects in the sidebar",
                     lambda: order("archive", "old-notes", "projects") == ["archive", "old-notes", "projects"], 20)
        finally:
            shot("archive-first")
        assert listed(workshop) == ["random", "general", "Archive", "Projects"], listed(workshop)
        assert hierarchy(workshop) == listed(workshop), hierarchy(workshop)
        print(f"Matrix: Workshop lists {', '.join(listed(workshop))}, the same in /hierarchy", flush=True)


if __name__ == "__main__":
    main()
