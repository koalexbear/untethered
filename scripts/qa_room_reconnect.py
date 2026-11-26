"""Voice-room recovery: one client (Bruno) on its own network link.

(a) Bruno's link goes down for 6.5 s, (b) only the SFU's ports are blocked for
6.5 s, (c) his link goes down for 25 s, past the 16 s delayed leave. Alice and
Clara keep hearing each other throughout (a) and (b); afterwards every path
carries fresh tones. In (a) and (b) Bruno keeps his original membership; in
(c) he rejoins with a new one, everyone's sender key index advances, and he
is in the roster once.
"""
import re
import time
from pathlib import Path

from qa_native_paste import wait_for
from qa_rtc_link import outage, sfu_block
from qa_ui import CHAT_X
from qa_voice_room import TONES

LEASE = 16


def key_indices(log):
    """Each sender key index a client started using (UNTETHERED_QA_RECOVERY)."""
    return [int(n) for n in re.findall(r"^RTC sender key index=(\d+)$",
                                       Path(log).read_text(errors="replace"), re.M)]


def recoveries(log):
    """The client's lost LiveKit sessions and rejoins, in order."""
    return re.findall(r"^RTC call (lost=\w+|rejoined fresh=\w+)$", Path(log).read_text(errors="replace"), re.M)


def check_reconnect(desks, users, names, link, logs, member_events, members, publications, audio_counts):
    """`member_events()`: the active call memberships in the homeserver's
    state, as (sender, event ID, created_ts)."""
    alice, bruno, clara = desks

    def memberships():
        return {sender: (event, created) for sender, event, created in member_events()}

    def heard(timeout):
        """All three speakers decode exactly the others' current tones."""
        for d in desks:
            d.windows.clear()
        expected = [{TONES[o.tone] for o in desks if o is not d} for d in desks]
        wait_for("fresh tones on all six paths " + "/".join(str(sorted(e)) for e in expected),
                 lambda: all(d.hears(e) for d, e in zip(desks, expected)), timeout)

    def shows(d, word):
        return word in {w for w, _, _ in d.words()}

    def outage_case(label, cut, seconds, during, after):
        """Cut Bruno off, switch his tone while away and the others' after."""
        original = memberships()[users[1]]
        seen = [recoveries(log) for log in logs]
        for d in (alice, clara):
            d.audit = []
        start = time.monotonic()
        expired = None
        with cut():
            # Alice and Clara hear only each other while Bruno is away.
            wait_for("Bruno silent at Alice and Clara",
                     lambda: alice.hears({TONES[clara.tone]}) and clara.hears({TONES[alice.tone]}), 12)
            wait_for("Bruno shows Reconnecting", lambda: shows(bruno, "reconnecting"), 12)
            bruno.tone = during
            if seconds > LEASE:
                wait_for("Bruno's delayed leave fired", lambda: users[1] not in memberships(), seconds)
                expired = time.monotonic() - start
                assert members() == sorted(users[::2])
            wait_for(f"{seconds}-second {label}", lambda: time.monotonic() - start >= seconds, seconds + 1)
            if seconds < LEASE:
                assert memberships()[users[1]][1] == original[1], "Bruno's membership changed during a short outage"
        restored = time.monotonic()
        audits = [(d.audit, TONES[o.tone]) for d, o in ((alice, clara), (clara, alice))]
        alice.audit = clara.audit = None
        for audit, tone in audits:
            windows = [amps[TONES.index(tone)] for t, amps in audit if start <= t <= restored]
            if seconds < LEASE:
                # Every 100 ms window, from the cut until the link returned.
                assert windows and min(windows) > 500, (label, tone, min(windows, default=None))
        alice.tone, clara.tone = after
        heard(60 if seconds > LEASE else 30)
        recovered = time.monotonic() - restored
        now = memberships()[users[1]]
        tracks = wait_for("three encrypted microphones at the SFU", lambda: publications(3), 10)
        wait_for("two playback streams and one capture per client", lambda: audio_counts() == [(1, 2)] * 3, 10)
        assert members() == sorted(users)
        # Only Bruno recovers; a short outage needs no rejoin at all.
        changes = [recoveries(log)[len(before):] for log, before in zip(logs, seen)]
        expected = ["lost=Lease", "rejoined fresh=true"] if seconds > LEASE else []
        assert changes == [[], expected, []], changes
        return original, now, tracks, recovered, expired, len(audits[0][0])

    # (a) Bruno's link goes down for 6.5 s; the SDK resumes on his membership.
    original, now, tracks, recovered, _, windows = outage_case(
        "full outage", lambda: outage(link), 6.5, 4, (3, 1))
    assert now[1] == original[1] and not shows(bruno, "reconnecting")
    print(f"(a) Bruno offline 6.5 s: Alice and Clara heard each other in all {windows} windows of 100 ms. "
          f"All six paths carried fresh tones {recovered:.1f}s after restoration, on Bruno's original membership "
          f"(created {original[1]}).", flush=True)

    # (b) Only the SFU is unreachable; Matrix heartbeats keep the membership.
    original, now, tracks, recovered, _, windows = outage_case(
        "SFU block", lambda: sfu_block(link), 6.5, 0, (2, 4))
    assert now[1] == original[1] and not shows(bruno, "reconnecting")
    print(f"(b) SFU ports blocked 6.5 s (Matrix up): Alice and Clara heard each other in all {windows} windows. "
          f"Fresh tones on all six paths after {recovered:.1f}s, on Bruno's original membership.", flush=True)

    # (c) Past the delayed leave: Bruno rejoins with a new membership and key.
    before = [key_indices(log)[-1] for log in logs]
    original, now, tracks, recovered, expired, _ = outage_case(
        "outage", lambda: outage(link), 25, 3, (1, 0))
    after = [key_indices(log)[-1] for log in logs]
    assert now[0] != original[0] and now[1] > original[1], (original, now)
    assert all(a > b for a, b in zip(after, before)), (before, after)
    assert [sender for sender, _, _ in member_events()].count(users[1]) == 1 and len(set(tracks)) == 3
    for d in (alice, clara):
        # One roster row: the reader's passes may each find the same word.
        ys = sorted(y for w, x, y in d.words() if x < CHAT_X and w == names[1].lower())
        rows = [y for i, y in enumerate(ys) if i == 0 or y - ys[i - 1] > 10]
        assert len(rows) == 1, ys
    print(f"(c) Bruno offline 25 s: his membership expired after {expired:.1f}s. He rejoined with membership {now[0]} "
          f"(created {now[1] - original[1]} ms later); sender key indices {before} -> {after}. "
          f"Fresh tones on all six paths {recovered:.1f}s after restoration; one Bruno in the roster and at the SFU.",
          flush=True)
    return tracks
