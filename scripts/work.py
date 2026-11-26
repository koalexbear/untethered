#!/usr/bin/env python3
"""The agents' work board: who is working on what, on which branch, touching which files.

The board lives on its own git ref (`refs/heads/work-board`) that nobody checks out, so
every worktree of this repository sees a claim the moment it is made, without merging.
Each change is one commit, written with git plumbing and published with a
compare-and-swap `update-ref`: two agents can't both win the same claim. With
`git config workboard.remote origin` the board is also fetched from and pushed to that
remote, for agents on other machines or in the cloud.

    scripts/work.py                      # the board (same as `list`)
    scripts/work.py claim M2 --title "Composer autocomplete" --paths rust-client/src/view.rs
    scripts/work.py who rust-client/src/view.rs
    scripts/work.py check                # does this branch touch files someone else claimed?
    scripts/work.py note M2 "I also need the composer's key bindings" --by m3-typing
    scripts/work.py touch M2 --status review
    scripts/work.py done M2 --result "merged as 1a2b3c4"

Identify yourself with --by NAME or WORK_AGENT=NAME; otherwise the current branch names
you. AGENTS.md describes when to use each command.
"""
import argparse
import datetime as dt
import fnmatch
import json
import os
import shutil
import subprocess
import sys

REF = "refs/heads/work-board"
BRANCH = "work-board"
# A claim nobody touched for this long may be stale; taking it over needs the user's OK.
STALE_HOURS = 24
STATUSES = ("working", "blocked", "review", "paused")


def git(*args, input=None, check=True):
    r = subprocess.run(["git", *args], input=input, capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout.strip() if r.returncode == 0 else None


def now():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def age(stamp):
    then = dt.datetime.fromisoformat(stamp.replace("Z", "+00:00"))
    return dt.datetime.now(dt.timezone.utc) - then


def ago(stamp):
    s = int(age(stamp).total_seconds())
    if s < 120:
        return "just now"
    if s < 7200:
        return f"{s // 60} min ago"
    if s < 172800:
        return f"{s // 3600} h ago"
    return f"{s // 86400} days ago"


def stale(claim):
    return age(claim["updated"]) > dt.timedelta(hours=STALE_HOURS)


def me(args):
    name = getattr(args, "by", None) or os.environ.get("WORK_AGENT")
    return name or git("branch", "--show-current", check=False) or "unknown"


def worktree():
    top = git("rev-parse", "--show-toplevel", check=False) or ""
    common = os.path.dirname(os.path.abspath(git("rev-parse", "--git-common-dir")))
    return os.path.relpath(top, common) if top else ""


# --- storage -----------------------------------------------------------------

def load(sha):
    if not sha:
        return {"claims": {}, "done": []}
    return json.loads(git("show", f"{sha}:board.json"))


def remote():
    return git("config", "--get", "workboard.remote", check=False)


def push_args():
    # This checkout's https remote authenticates through GitHub's CLI when it is there.
    return ["-c", "credential.helper=", "-c", "credential.helper=!gh auth git-credential"] if shutil.which("gh") else []


def sync():
    """Bring the local board up to date with the remote's, merging diverged boards."""
    name = remote()
    if not name:
        return
    r = subprocess.run(["git", *push_args(), "fetch", "-q", name, f"+{REF}:refs/work-board/{name}"],
                       capture_output=True, text=True, timeout=30)
    if r.returncode:
        if "couldn't find remote ref" not in r.stderr:
            print(f"(work board: couldn't fetch from {name}; showing the local board)", file=sys.stderr)
        return
    theirs = git("rev-parse", "-q", "--verify", f"refs/work-board/{name}", check=False)
    ours = git("rev-parse", "-q", "--verify", REF, check=False)
    if not theirs or theirs == ours:
        return
    if not ours or git("merge-base", "--is-ancestor", ours, theirs, check=False) is not None:
        git("update-ref", REF, theirs, *( [ours] if ours else [] ))
    elif git("merge-base", "--is-ancestor", theirs, ours, check=False) is None:
        merged = merge(load(ours), load(theirs))
        commit(merged, [ours, theirs], "Merge the remote work board", expect=ours)


def merge(a, b):
    """Union of two boards: the newer version of each claim wins, notes and history combine."""
    out = {"claims": {}, "done": []}
    finished = {(d["id"], d["claimed"]) for d in a["done"] + b["done"]}
    for cid in set(a["claims"]) | set(b["claims"]):
        x, y = a["claims"].get(cid), b["claims"].get(cid)
        pick = max((c for c in (x, y) if c), key=lambda c: c["updated"])
        notes = {json.dumps(n, sort_keys=True): n for c in (x, y) if c for n in c.get("notes", [])}
        pick = dict(pick, notes=sorted(notes.values(), key=lambda n: n["at"]))
        if (cid, pick["claimed"]) not in finished:
            out["claims"][cid] = pick
    seen = set()
    for d in sorted(a["done"] + b["done"], key=lambda d: d["finished"]):
        key = (d["id"], d["claimed"])
        if key not in seen:
            seen.add(key)
            out["done"].append(d)
    return out


def render(board):
    lines = ["# Work board", "", "Written by `scripts/work.py`; see AGENTS.md. Don't edit by hand.", ""]
    if board["claims"]:
        lines += ["| ID | Title | Agent | Branch | Status | Updated | Paths |", "|---|---|---|---|---|---|---|"]
        for cid, c in sorted(board["claims"].items()):
            lines.append(f"| {cid} | {c['title']} | {c['owner']} | `{c['branch']}` | {c['status']} | "
                         f"{c['updated']} | {', '.join(f'`{p}`' for p in c['paths'])} |")
    else:
        lines.append("Nothing claimed.")
    if board["done"]:
        lines += ["", "## Done", ""]
        for d in reversed(board["done"][-30:]):
            lines.append(f"- {d['finished'][:10]} **{d['id']}** {d['title']} ({d['owner']}): {d['result']}")
    return "\n".join(lines) + "\n"


def make_commit(board, parents, message):
    blob = git("hash-object", "-w", "--stdin", input=json.dumps(board, indent=1, sort_keys=True) + "\n")
    page = git("hash-object", "-w", "--stdin", input=render(board))
    tree = git("mktree", input=f"100644 blob {blob}\tboard.json\n100644 blob {page}\tBOARD.md\n")
    return git("commit-tree", tree, *sum((["-p", p] for p in parents), []), "-m", message)


def advance(sha, expect):
    """Compare and swap: move the local board to `sha` only if it is still at `expect`."""
    return git("update-ref", REF, sha, expect or "0" * 40, check=False) is not None


def commit(board, parents, message, expect):
    return advance(make_commit(board, parents, message), expect)


def change(message, edit):
    """Apply `edit` to the latest board and publish it, retrying if another agent got there first.

    With a remote, the push decides: it only lands on top of the board this edit was checked
    against, so two machines can't both win a claim. Offline, the change stays local and
    merges on the next sync."""
    for _ in range(10):
        sync()
        ours = git("rev-parse", "-q", "--verify", REF, check=False)
        board = load(ours)
        result = edit(board)
        sha = make_commit(board, [ours] if ours else [], message)
        pushed = publish(sha)
        if pushed == "moved":
            continue
        if advance(sha, ours):
            return result
        # Someone here moved the board meanwhile; fine if they built on our pushed commit.
        if pushed == "ok" and git("merge-base", "--is-ancestor", sha, REF, check=False) is not None:
            return result
    raise SystemExit("The work board kept changing under us; try again.")


def publish(sha):
    """Push `sha` as the remote's board: "ok", "moved" (someone else pushed first) or "local"."""
    name = remote()
    if not name:
        return "local"
    r = subprocess.run(["git", *push_args(), "push", "-q", name, f"{sha}:{REF}"],
                       capture_output=True, text=True, timeout=60)
    if r.returncode == 0:
        return "ok"
    if any(w in r.stderr for w in ("rejected", "non-fast-forward", "fetch first")):
        return "moved"
    first = (r.stderr.strip().splitlines() or ["unknown error"])[-1]
    print(f"(work board saved locally; couldn't reach {name}: {first[:160]})", file=sys.stderr)
    return "local"


# --- paths -------------------------------------------------------------------

def overlaps(a, b):
    """Whether two path patterns (files, directories or globs) can name the same file."""
    a, b = a.rstrip("/"), b.rstrip("/")
    return (a == b or b.startswith(a + "/") or a.startswith(b + "/")
            or fnmatch.fnmatch(a, b) or fnmatch.fnmatch(b, a))


def holders(board, paths, besides=None):
    found = []
    for cid, c in sorted(board["claims"].items()):
        if cid == besides:
            continue
        hits = sorted({p for p in paths for q in c["paths"] if overlaps(p, q)})
        if hits:
            found.append((cid, c, hits))
    return found


# --- commands ----------------------------------------------------------------

def show_claim(cid, c, notes=2):
    flag = "  STALE" if stale(c) else ""
    print(f"{cid}  {c['title']}{flag}")
    print(f"    {c['owner']} · {c['status']} · branch {c['branch']} · updated {ago(c['updated'])}")
    ahead = git("rev-list", "--count", f"main..{c['branch']}", check=False)
    if ahead is not None:
        print(f"    {ahead} commit(s) on the branch beyond main")
    print(f"    paths: {', '.join(c['paths'])}")
    for n in c.get("notes", [])[-notes:]:
        print(f"    note from {n['by']} ({ago(n['at'])}): {n['text']}")


def cmd_list(args):
    sync()
    board = load(git("rev-parse", "-q", "--verify", REF, check=False))
    if not board["claims"]:
        print("Work board: nothing claimed. Claim your task before you start (AGENTS.md).")
    else:
        print(f"Work board: {len(board['claims'])} active claim(s). Don't work on these "
              "or edit their paths without coordinating (AGENTS.md).\n")
        for cid, c in sorted(board["claims"].items()):
            show_claim(cid, c)
    if args.done and board["done"]:
        print("\nRecently done:")
        for d in reversed(board["done"][-args.done:]):
            print(f"    {d['finished'][:10]} {d['id']} {d['title']} ({d['owner']}): {d['result']}")


def cmd_show(args):
    sync()
    c = load(git("rev-parse", "-q", "--verify", REF, check=False))["claims"].get(args.id)
    if not c:
        raise SystemExit(f"{args.id} isn't claimed.")
    show_claim(args.id, c, notes=len(c.get("notes", [])))


def cmd_claim(args):
    who, branch = me(args), args.branch or git("branch", "--show-current", check=False) or ""

    def edit(board):
        held = board["claims"].get(args.id)
        if held and held["owner"] != who:
            if not stale(held) or not args.take_over:
                why = "stale; pass --take-over once the user agrees" if stale(held) else "active"
                raise SystemExit(f"{args.id} is claimed by {held['owner']} ({why}).")
        clash = holders(board, args.paths, besides=args.id)
        if clash and not args.share:
            for cid, c, hits in clash:
                print(f"{cid} ({c['owner']}, {c['status']}) also claims {', '.join(hits)}", file=sys.stderr)
            raise SystemExit("Those paths overlap with active claims. Coordinate first (a note to the "
                             "owner), then claim again with --share \"what you'll touch and why\".")
        stamp = now()
        notes = list(held.get("notes", [])) if held else []
        if held and held["owner"] != who:
            notes.append({"at": stamp, "by": who, "text": f"took over the stale claim from {held['owner']}"})
        board["claims"][args.id] = {
            "title": args.title or (held or {}).get("title") or args.id, "owner": who,
            "branch": branch, "worktree": worktree(), "paths": sorted(set(args.paths)),
            "status": "working", "claimed": (held or {}).get("claimed", stamp) if held and held["owner"] == who else stamp,
            "updated": stamp, "notes": notes,
        }
        for cid, c, hits in clash:
            line = {"at": stamp, "by": who, "text": f"[{args.id}] shares {', '.join(hits)}: {args.share}"}
            # A note leaves the owner's heartbeat alone.
            c.setdefault("notes", []).append(line)
    change(f"Claim {args.id} ({who})", edit)
    print(f"Claimed {args.id} for {who} on {branch or 'no branch'}.")


def owned(board, cid, who, force):
    c = board["claims"].get(cid)
    if not c:
        raise SystemExit(f"{cid} isn't claimed.")
    if c["owner"] != who and not force:
        raise SystemExit(f"{cid} belongs to {c['owner']}; leave it a note instead (or --force if the user says so).")
    return c


def cmd_touch(args):
    who = me(args)

    def edit(board):
        c = owned(board, args.id, who, args.force)
        if args.status:
            c["status"] = args.status
        if args.paths:
            clash = holders(board, args.paths, besides=args.id)
            for cid, other, hits in clash:
                print(f"note: {cid} ({other['owner']}) also claims {', '.join(hits)}", file=sys.stderr)
            c["paths"] = sorted(set(c["paths"]) | set(args.paths))
        c["updated"] = now()
    change(f"Update {args.id} ({who})", edit)
    print(f"Updated {args.id}.")


def cmd_note(args):
    who = me(args)

    def edit(board):
        c = board["claims"].get(args.id)
        if not c:
            raise SystemExit(f"{args.id} isn't claimed.")
        c.setdefault("notes", []).append({"at": now(), "by": who, "text": args.text})
        if c["owner"] == who:
            c["updated"] = now()
    change(f"Note on {args.id} ({who})", edit)
    print(f"Left a note on {args.id}.")


def cmd_finish(args, verb):
    who = me(args)

    def edit(board):
        c = owned(board, args.id, who, args.force)
        board["done"].append({"id": args.id, "title": c["title"], "owner": c["owner"], "branch": c["branch"],
                              "claimed": c["claimed"], "finished": now(),
                              "result": args.result if verb == "done" else f"released: {args.result}"})
        del board["claims"][args.id]
    change(f"{verb.capitalize()} {args.id} ({who})", edit)
    print(f"{args.id} is off the board ({verb}).")


def cmd_who(args):
    sync()
    board = load(git("rev-parse", "-q", "--verify", REF, check=False))
    found = holders(board, args.paths)
    if not found:
        print("Nobody has claimed those paths.")
    for cid, c, hits in found:
        print(f"{cid} ({c['owner']}, {c['status']}, branch {c['branch']}): {', '.join(hits)}")


def cmd_check(args):
    """Files this branch changed (committed or not) that another active claim covers."""
    sync()
    board = load(git("rev-parse", "-q", "--verify", REF, check=False))
    base = git("merge-base", "HEAD", args.base, check=False) or args.base
    changed = set(filter(None, (git("diff", "--name-only", base, check=False) or "").splitlines()))
    changed |= set(filter(None, (git("ls-files", "--others", "--exclude-standard", check=False) or "").splitlines()))
    branch = git("branch", "--show-current", check=False)
    mine = {cid for cid, c in board["claims"].items() if c["branch"] == branch or c["owner"] == me(args)}
    covered = [q for cid in mine for q in board["claims"][cid]["paths"]]
    # Files in your own claim are yours, even where you share them with someone.
    outside = sorted(f for f in changed if not any(overlaps(f, q) for q in covered))
    clash = [(cid, c, hits) for cid, c, hits in holders(board, outside) if cid not in mine]
    for cid, c, hits in clash:
        print(f"{cid} ({c['owner']}, branch {c['branch']}) claims files you changed: {', '.join(hits)}")
    if not mine:
        print("This branch has no claim on the board; claim your task (AGENTS.md).")
    elif outside:
        print(f"Changed outside your claim (add them with `touch ID --paths`): {', '.join(outside)}")
    if clash:
        sys.exit(1)
    print(f"No conflicts: {len(changed)} changed file(s), none claimed by someone else.")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--by", help="who you are (default: $WORK_AGENT, then the current branch)")
    sub = p.add_subparsers(dest="cmd")
    s = sub.add_parser("list", help="the board")
    s.add_argument("--done", type=int, default=5, metavar="N", help="also show the last N finished items")
    s = sub.add_parser("show", help="one claim with all its notes")
    s.add_argument("id")
    s = sub.add_parser("claim", help="claim a task before working on it")
    s.add_argument("id", help="a ROADMAP step (M2) or a short slug (fix-scrollbar-drag)")
    s.add_argument("--title")
    s.add_argument("--paths", nargs="+", required=True, help="files, directories or globs you expect to change")
    s.add_argument("--branch", help="default: the current branch")
    s.add_argument("--share", metavar="WHY", help="claim even though paths overlap; tells their owners why")
    s.add_argument("--take-over", action="store_true", help="take a STALE claim (only with the user's OK)")
    s = sub.add_parser("touch", help="heartbeat; optionally change status or add paths")
    s.add_argument("id")
    s.add_argument("--status", choices=STATUSES)
    s.add_argument("--paths", nargs="+", help="add these paths to the claim")
    s.add_argument("--force", action="store_true")
    s = sub.add_parser("note", help="leave a message on a claim (yours or someone else's)")
    s.add_argument("id")
    s.add_argument("text")
    for verb in ("done", "release"):
        s = sub.add_parser(verb, help="finished (merged)" if verb == "done" else "give the task back")
        s.add_argument("id")
        s.add_argument("--result", required=True, help="e.g. 'merged as 1a2b3c4' or why you stopped")
        s.add_argument("--force", action="store_true")
    s = sub.add_parser("who", help="who claims these paths")
    s.add_argument("paths", nargs="+")
    s = sub.add_parser("check", help="does this branch change files someone else claimed?")
    s.add_argument("--base", default="main")
    for s in sub.choices.values():
        s.add_argument("--by", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
    args = p.parse_args()
    if git("rev-parse", "--git-dir", check=False) is None:
        raise SystemExit("Run this inside the repository.")
    cmd = args.cmd or "list"
    if cmd == "list" and not hasattr(args, "done"):
        args.done = 5
    {"list": cmd_list, "show": cmd_show, "claim": cmd_claim, "touch": cmd_touch, "note": cmd_note,
     "done": lambda a: cmd_finish(a, "done"), "release": lambda a: cmd_finish(a, "release"),
     "who": cmd_who, "check": cmd_check}[cmd](args)


if __name__ == "__main__":
    main()
