"""Drive Alice's Right Ctrl key on a private X server and measure all three outputs."""
import time

from qa_native_paste import child, wait_for


def check_ptt(stack,desks,audio_counts,same_tracks):
    alice,bruno,clara=desks
    # Both peers recorded every speaker window from before the first join.
    for peer in (bruno,clara):
        assert peer.audit and max(f[0] for _,f in peer.audit)<200,"Alice audible before first press"
        peer.audit=[]

    stack.enter_context(child(["xmessage","-title","PTT focus target","-geometry","220x80+10+480",
                               "Isolated focus test"],alice.env))
    target=wait_for("private focus target",lambda:alice.cmd("xdotool","search","--onlyvisible","--name",
                    "^PTT focus target$",check=False).strip())
    def focus(win):
        alice.cmd("xdotool","windowfocus","--sync",win)
        assert int(alice.cmd("xdotool","getwindowfocus").strip())==int(win)
    def key(down):alice.cmd("xdotool","keydown" if down else "keyup","Control_R")
    def edge(down,stage,tone):
        # Bound end-to-end response as observed by each speaker monitor,
        # including input delivery, jitter buffer and the 100 ms analysis window.
        before=time.monotonic()
        key(down)
        check(stage,tone if down else None)
        band={410:0,530:3}[tone]
        ms=[]
        for peer in (bruno,clara):
            after=[(t,f[band]) for t,f in peer.audit if t>=before]
            # First window after the last mismatch: a brief hole followed by
            # more speech must not count as a successful release.
            start=1+max((i for i,(_,amp) in enumerate(after) if
                         (amp<500 if down else amp>=200)),default=-1)
            assert len(after)-start>=11,(stage,after)
            ms.append(round(1000*(after[start][0]-before)))
        assert max(ms)<800,(stage,ms)
        print(f"  Key {'down' if down else 'up'} to stable speaker windows: Bruno {ms[0]} ms, Clara {ms[1]} ms.",flush=True)
    def check(stage,tone=None,deaf=False):
        for d in desks:d.windows.clear()
        sent={tone} if tone else set()
        expect=(set() if deaf else {670,1030},{1030}|sent,{670}|sent)
        wait_for(stage,lambda:all(d.hears(hz) for d,hz in zip(desks,expect)),10)
        assert audio_counts()==[(int(tone is not None),1),(1,1),(1,1)] and same_tracks()
        print(f"{stage}: Bruno heard {sorted(expect[1])} Hz; Clara heard {sorted(expect[2])} Hz; "
              f"Alice mic streams: {int(tone is not None)}.",flush=True)

    focus(alice.win)
    try:
        check("Joined with PTT armed")
        edge(True,"Right Ctrl held",410)
        alice.tone=3
        check("Fresh input during same hold",530)
        edge(False,"Right Ctrl released with 530 Hz still at input",530)
        edge(True,"Second press",530)
        focus(target)
        check("Focus lost before key-up")
        focus(alice.win)  # Key repeat on refocus must not reopen capture.
        check("Focus returned with Right Ctrl still held")
        focus(target)
        key(False)  # Delivered to the other window.
        focus(alice.win)
        check("Focus returned without a press")
        alice.click("mute")
        key(True)
        check("Right Ctrl cannot override manual mute")
        alice.click("unmute")
        check("Unmute during hold still needs a fresh press")
        key(False)
        alice.tone=0
        key(True)
        check("Fresh press after unmute",410)
        alice.click("deafen")
        check("Deafen during hold",deaf=True)
        alice.click("undeafen")
        check("Undeafen still needs a fresh press")
        key(False)
        key(True)
        check("Fresh press after undeafen",410)
        key(False)
        check("Last release")
        alice.click("push-to-talk")
        check("Open mic restored",410)
    finally:
        key(False)
    for peer,band,who in ((bruno,2,"Clara → Bruno"),(clara,1,"Bruno → Clara")):
        audit=peer.audit
        peer.audit=None
        minimum=min(f[band] for _,f in audit)
        assert minimum>500,(who,len(audit),minimum)
        print(f"{who}: audible in all {len(audit)} consecutive 100 ms windows across key, "
              f"focus, mute and deafen transitions (minimum tone amplitude {minimum:.0f}).",flush=True)
