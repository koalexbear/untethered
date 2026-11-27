"""Polls (ROADMAP M9), live against a disposable Synapse.

Build the debug client (it seeds the profile) and the release client (it
runs the UI). From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/qa_polls.py

The test account shares an encrypted room, lunch, with Enzo on a gomuks
device. Enzo starts a poll there with the events Element sends
(`org.matrix.msc3381.poll.*`) and votes in it. The release client in a
private KWin session (as qa_kwin.py needs) must:
- show the poll, hide its results until the test account votes, then vote,
  take the vote back and vote again, each reaching Enzo's device;
- show the final tally once Enzo ends it, the same one Enzo's events give,
  and take no more votes;
- start a multiple-choice poll from the composer's + menu, which Enzo's
  device reads as Element's poll, count Enzo's two answers alongside its
  own, and end it from the card, with the top answer in the end event's
  fallback text.
--shots DIR keeps the screenshots; UNTETHERED_QA_UI_CLIENT runs another
client build.
"""
import argparse
import contextlib
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import time
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from qa_matrix_sdk import Gomuks, http, register  # noqa: E402
from qa_media_previews import homeserver, interop, tap  # noqa: E402
from qa_ui import Ui, clean  # noqa: E402

WINDOW = (0, 0, 1180, 900)
TITLE_BAR = 28
SIDEBAR = (72, 40, 330, 840)
MAIN = (330, 40, 1180, 900)
ENTER = ""
START = "org.matrix.msc3381.poll.start"
RESPONSE = "org.matrix.msc3381.poll.response"
END = "org.matrix.msc3381.poll.end"
LUNCH = "Where should we have lunch?"
LUNCH_ANSWERS = ("Pizza", "Soup", "Tacos")
COLOUR = "Which colour for the banner?"
COLOURS = ("Crimson", "Emerald", "Cobalt")


def main():
    args = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    args.add_argument("--shots", type=Path)
    args = args.parse_args()
    os.umask(0o077)
    tmp = Path(tempfile.mkdtemp(prefix="untethered-polls-"))
    synapse = None
    peer = None
    try:
        synapse, hs = homeserver(tmp)
        _, hdr = interop().seed(tmp, hs, encrypted=False)
        me = http("GET", hs + "/_matrix/client/v3/account/whoami", headers=hdr).json()["user_id"]
        enzo, enzo_pwd, enzo_hdr = register(hs, "enzo" + secrets.token_hex(3))
        encryption = [{"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]
        lunch = http("POST", hs + "/_matrix/client/v3/createRoom", headers=hdr, json=dict(
            name="lunch", preset="private_chat", invite=[enzo], initial_state=encryption)).json()["room_id"]
        http("POST", f"{hs}/_matrix/client/v3/rooms/{quote(lunch, safe='')}/join", headers=enzo_hdr, json={})

        peer = Gomuks(tmp / "enzo", hs, enzo, enzo_pwd)
        peer.recovery(enzo_pwd)  # gomuks syncs rooms once its encryption is set up
        peer.joined(lunch)
        polls = Polls(peer, lunch)
        # Exactly what Element Web sends for a new poll.
        answers = [{"id": secrets.token_urlsafe(9), "org.matrix.msc1767.text": a} for a in LUNCH_ANSWERS]
        peer.rpc("send_event", {"room_id": lunch, "type": START, "content": {
            START: {"question": {"org.matrix.msc1767.text": LUNCH, "body": LUNCH, "msgtype": "m.text"},
                    "kind": "org.matrix.msc3381.poll.disclosed", "max_selections": 1, "answers": answers},
            "org.matrix.msc1767.text": LUNCH + "".join(f"\n{i}. {a}" for i, a in enumerate(LUNCH_ANSWERS, 1))}})
        poll = polls.started(enzo, LUNCH)["event_id"]
        ids = {a["org.matrix.msc1767.text"]: a["id"] for a in answers}
        polls.vote(poll, [ids["Soup"]])
        time.sleep(2)
        print(f"Synapse at {hs}: {me} with {enzo} on gomuks, who started a poll in lunch", flush=True)
        look(tmp, args.shots, me, enzo, polls, poll, ids)
        print("PASS: a poll from Element's events shows, takes votes and ends with the same tally on both "
              "devices; a poll started here reads as Element's and ends from its card", flush=True)
    finally:
        if peer:
            peer.close()
        if synapse:
            synapse.terminate()
            synapse.wait(timeout=20)
        shutil.rmtree(tmp, ignore_errors=True)


class Polls:
    """Polls as Enzo's gomuks device sees them, counted as MSC3381 says."""

    def __init__(self, peer, room):
        self.peer, self.room = peer, room

    def events(self, kind):
        seen, out = set(), []
        for e in self.peer.events:
            if e.get("room_id") != self.room or not str(e.get("event_id", "")).startswith("$"):
                continue
            if e["event_id"] in seen:
                continue
            content = e.get("decrypted") if e.get("type") == "m.room.encrypted" else e.get("content")
            if (e.get("decrypted_type") or e.get("type")) == kind and content:
                seen.add(e["event_id"])
                out.append({**e, "content": content})
        return out

    def started(self, sender, question, timeout=60):
        return self.waiting(question, lambda: next((
            e for e in self.events(START) if e["sender"] == sender
            and e["content"][START]["question"]["org.matrix.msc1767.text"] == question), None), timeout)

    def vote(self, poll, answers):
        self.peer.rpc("send_event", {"room_id": self.room, "type": RESPONSE, "content": {
            RESPONSE: {"answers": answers}, "m.relates_to": {"rel_type": "m.reference", "event_id": poll}}})

    def end(self, poll, text):
        self.peer.rpc("send_event", {"room_id": self.room, "type": END, "content": {
            END: {}, "org.matrix.msc1767.text": text,
            "m.relates_to": {"rel_type": "m.reference", "event_id": poll}}})

    def related(self, kind, poll):
        # An encrypted event's m.relates_to stays outside its ciphertext;
        # gomuks gives the target as relates_to either way.
        return [e for e in self.events(kind) if e.get("relates_to") == poll]

    def ballots(self, poll):
        """Each voter's newest answers, up to the poll's end."""
        ends = [e["timestamp"] for e in self.related(END, poll)]
        latest = {}
        for e in sorted(self.related(RESPONSE, poll), key=lambda e: e["timestamp"]):
            if ends and e["timestamp"] > min(ends):
                continue
            latest[e["sender"]] = e["content"][RESPONSE]["answers"]
        return latest

    def tally(self, poll):
        counts = {}
        for answers in self.ballots(poll).values():
            for a in answers:
                counts[a] = counts.get(a, 0) + 1
        return counts

    def waiting(self, label, check, timeout=60):
        """What `check` returns once it's something, as Enzo's device syncs."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if found := check():
                return found
            self.peer.frame(1)
        raise RuntimeError(f"gomuks never saw {label}")


def look(tmp, shots, me, enzo, polls, poll, ids):
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
            im = kde.capture().crop(WINDOW).convert("RGB")
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                im.save(shots / f"{name}.png")
            return im

        def words(rect=MAIN):
            return [(clean(w), x, y) for w, x, y in kde.words(rect)]

        def word(part, rect=MAIN):
            return next(((x, y) for w, x, y in words(rect) if part in w), None)

        def exact(text, rect=MAIN):
            return [(x, y) for w, x, y in words(rect) if w == text]

        def phrase(*parts, rect=MAIN):
            """Where `parts` read in order on one line."""
            ws = words(rect)
            for w, x, y in ws:
                if w != parts[0]:
                    continue
                at, ok = x, True
                for part in parts[1:]:
                    nxt = [(vx, vy) for v, vx, vy in ws if v == part and abs(vy - y) < 8 and 0 < vx - at < 160]
                    if not nxt:
                        ok = False
                        break
                    at = min(nxt)[0]
                if ok:
                    return x, y
            return None

        def settled(label, find, timeout=20):
            """Where `find` reads twice in a row, a moment apart: the timeline
            keeps its bottom in place, so a receipt or line arriving below
            moves everything above it."""
            def still():
                first = find()
                time.sleep(0.8)
                second = find()
                return first if first and second and abs(first[0] - second[0]) < 4 \
                    and abs(first[1] - second[1]) < 4 else None
            return wait_for(label, still, timeout)

        def mine(poll):
            return polls.ballots(poll).get(me)

        def received():
            return sorted({(e.get("decrypted_type") or e.get("type"), e.get("relates_to"),
                            bool(e.get("decrypted")), e.get("decryption_error") or "")
                           for e in polls.peer.events if e.get("sender") == me}, key=str)

        def voted(label, poll, answers):
            try:
                polls.waiting(label, lambda: mine(poll) == answers)
            except RuntimeError:
                shot("missing-" + label.replace(" ", "-"))
                print(f"Expected {answers}, ballots {polls.ballots(poll)}; Enzo's device saw from the test "
                      f"account: {received()}; status words: {[w for w, _, y in words(WINDOW) if y > 860]}",
                      flush=True)
                raise

        tap(kde, *wait_for("lunch in the sidebar", lambda: word("lunch", SIDEBAR), 90))
        wait_for("the poll's question", lambda: phrase("have", "lunch"), 60)
        # Enzo voted; a disclosed poll shows its results only to those who did.
        try:
            wait_for("results hidden until voting", lambda: phrase("vote", "to", "see"), 20)
        finally:
            shot("poll-before-voting")
        assert not exact("100"), "results showed before voting"
        print("UI: Enzo's poll shows its question and answers, 1 vote, its results waiting for a vote", flush=True)

        tap(kde, *settled("Pizza", lambda: word("pizza")))
        voted("the vote for Pizza", poll, [ids["Pizza"]])
        wait_for("the results after voting", lambda: len(exact("50")) >= 2 and phrase("2", "votes"), 30)
        shot("poll-voted")
        tap(kde, *settled("Remove vote", lambda: phrase("remove", "vote")))
        voted("the vote taken back", poll, [])
        wait_for("results hidden again", lambda: phrase("vote", "to", "see"), 30)
        tap(kde, *settled("Soup", lambda: word("soup")))
        voted("the vote for Soup", poll, [ids["Soup"]])
        assert polls.tally(poll) == {ids["Soup"]: 2}, polls.tally(poll)
        wait_for("Soup at 100%", lambda: exact("100") and phrase("2", "votes"), 30)
        print("Matrix: Enzo's device read the votes for Pizza, none, then Soup; both now on Soup", flush=True)

        polls.end(poll, "The poll has ended. Top answer: Soup")
        try:
            wait_for("the final results", lambda: phrase("final", "results") and phrase("poll", "closed"), 30)
        finally:
            shot("poll-ended")
        assert polls.tally(poll) == {ids["Soup"]: 2}
        assert exact("100") and phrase("2", "votes"), "the final tally differs from Enzo's"
        tap(kde, *settled("Pizza", lambda: word("pizza")))
        time.sleep(3)
        assert mine(poll) == [ids["Soup"]], "a vote went out after the poll ended"
        print("UI: once Enzo ended it, the poll shows Enzo's final tally (Soup, 2 votes) and takes no votes",
              flush=True)

        # Create Poll from the composer's +.
        toolbar = Ui(lambda: words(WINDOW), lambda pos: None, lambda: kde.capture().convert("RGB"),
                     width=WINDOW[2], height=WINDOW[3] - TITLE_BAR, origin=(0, TITLE_BAR))
        tap(kde, *toolbar.attach())
        tap(kde, *wait_for("Create Poll in the + menu", lambda: phrase("create", "poll"), 10))
        wait_for("Create a Poll", lambda: phrase("create", "a", "poll", rect=WINDOW), 10)
        kde.type(COLOUR)
        kde.keys(("Down", ENTER), ("Up", ENTER))
        for i, answer in enumerate(COLOURS):
            kde.type(answer)
            if i < len(COLOURS) - 1:
                kde.keys(("Down", ENTER), ("Up", ENTER))  # Enter in the last answer adds another
        tap(kde, *wait_for("Allow multiple answers", lambda: word("multiple", WINDOW), 10))
        time.sleep(0.5)
        shot("create-poll")
        tap(kde, *wait_for("Post", lambda: (exact("post", WINDOW) or [None])[-1], 10))
        wait_for("the dialog closed", lambda: not phrase("create", "a", "poll", rect=WINDOW), 30)

        start = polls.started(me, COLOUR, timeout=90)
        content = start["content"]
        block = content[START]
        assert [a["org.matrix.msc1767.text"] for a in block["answers"]] == list(COLOURS), block
        assert block["max_selections"] == 3, block
        assert block["kind"] == "org.matrix.msc3381.poll.disclosed", block
        assert content["org.matrix.msc1767.text"] == COLOUR + "".join(
            f"\n{i}. {a}" for i, a in enumerate(COLOURS, 1)), content
        colours = {a["org.matrix.msc1767.text"]: a["id"] for a in block["answers"]}
        print("Matrix: Enzo's device read the new poll as Element's: three answers, any number may be picked",
              flush=True)

        polls.vote(start["event_id"], [colours["Crimson"], colours["Cobalt"]])
        wait_for("Enzo's vote", lambda: phrase("1", "vote"), 30)
        tap(kde, *settled("Emerald", lambda: word("emerald")))
        voted("the vote for Emerald", start["event_id"], [colours["Emerald"]])
        tap(kde, *settled("Cobalt", lambda: word("cobalt")))
        voted("Emerald and Cobalt", start["event_id"], [colours["Emerald"], colours["Cobalt"]])
        wait_for("both votes counted", lambda: exact("100") and len(exact("50")) >= 2 and phrase("2", "votes"), 30)
        shot("created-poll-voted")
        tap(kde, *settled("End poll on the card", lambda: phrase("end", "poll")))
        wait_for("End Poll's question", lambda: word("everyone", WINDOW), 10)
        shot("end-poll-dialog")
        # The dialog's End Poll, beside its Cancel, not the card's link under the backdrop.
        cancel = wait_for("Cancel", lambda: word("cancel", WINDOW), 10)
        tap(kde, *next(p for p in exact("end", WINDOW) if abs(p[1] - cancel[1]) < 10 and p[0] > cancel[0]))
        try:
            end = polls.waiting("the end event", lambda: polls.related(END, start["event_id"]), 60)[0]
        finally:
            shot("end-poll-confirmed")
            print("Enzo's device saw from the test account:", received(), flush=True)
        assert end["sender"] == me, end
        assert end["content"]["org.matrix.msc1767.text"] == "The poll has ended. Top answer: Cobalt", end
        wait_for("the final results", lambda: phrase("final", "results") and phrase("poll", "closed"), 30)
        shot("created-poll-ended")
        print("UI: the + menu's Create Poll started it, Enzo's and this account's votes counted, and End poll "
              "ended it with Cobalt on top", flush=True)


if __name__ == "__main__":
    main()
