"""Two actual Iced clients on different links: invitations, cleanup and relay-only media."""
import json
import os
import re
import subprocess as sp
import threading
import time
from urllib.parse import quote,urlsplit

from qa_desktop_dm import sfu_participants
from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_rtc_link import outage
from qa_rtc_turn import counters,interrupt_turn,relay_links
from qa_ui import CHAT_X
from qa_voice_room import Desktop,MEMBER

RING="org.matrix.msc4075.rtc.notification"
DECLINE="org.matrix.msc4310.rtc.decline"


def check_native_dm(stack,tmp,hs,name,jwt,root,backend,room,users,headers,secret,http):
    relay=secret is not None
    dirs=(tmp,tmp/"peer")
    # Device IDs come from the clients' sessions, and the call events they
    # decrypted from their QA log (engine/calls.rs).
    events_log=[d/"call-events.jsonl" for d in dirs]
    api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
    ports=[urlsplit(s).port for s in (hs,"https://"+name,jwt)]+[18820]
    links=list(relay_links(stack,ports).values()) if relay else []
    sock=tmp/"qa-pulse"
    desks=[];logs=[];ids=[]
    if relay:
        for h in headers:http("POST",hs+"/_matrix/client/v3/createRoom",headers=h,json={"name":"Sidechat","preset":"private_chat"})
    for i,d in enumerate(dirs):
        session=json.loads((d/"seed/matrix-sdk/session.json").read_text())["session"]
        u,dev=session["user_id"],session["device_id"]
        assert u==users[i]
        ids.append(f"{u}:{dev}")
        env=isolated_env(d,backend,"x11")
        env.update(TOKIO_WORKER_THREADS="2",RAYON_NUM_THREADS="2",GOMAXPROCS="2",
                   PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE=f"Mic{i}.monitor",PULSE_SINK=f"Out{i}")
        start_display(stack,d,env,"x11")
        if i==0:
            pulse=stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
                f"--load=module-native-protocol-unix socket={sock} auth-anonymous=1",
                *(f"--load=module-null-sink sink_name={k}{j} channels=1 rate=48000" for j in range(2) for k in ("Mic","Out"))],env))
            def ready():
                assert pulse.poll() is None
                return sock.is_socket() and "Out1" in sp.run(["pactl","list","short","sinks"],env=env,capture_output=True,text=True).stdout
            wait_for("private audio server",ready)
        env["UNTETHERED_QA_ICE_STATS"]="1"
        env["UNTETHERED_QA_CALL_EVENTS"]=str(events_log[i])
        logs.append(d/"ice.log")
        desks.append(Desktop(stack,root,env,i,prefix=links[i].prefix if relay else [],room="media-proof",
                             log=stack.enter_context(logs[i].open("w"))))

    a,b=desks
    def words(d,dock=False):return {w for w,x,_ in d.words() if (x<CHAT_X if dock else x>CHAT_X)}
    def label(d,text,dock=False):wait_for("on-screen "+text,lambda:text in words(d,dock),12)
    def ready_ui():
        for d in desks:
            # Idle: no call bar or ring, and the header's phone icon is back.
            wait_for("idle DM controls",lambda:not words(d)&{"hang","accept","decline"}
                     and d.ui.header("start voice call"),20)
    def audio(kind):
        return [v for v in json.loads(a.cmd("pactl","-f","json","list",kind))
                if v["properties"].get("application.name")=="Untethered"]
    watcher=stack.enter_context(child(["stdbuf","-oL","pactl","subscribe"],{**a.env,"LC_ALL":"C"},stdout=sp.PIPE,text=True))
    events=[]
    threading.Thread(target=lambda:events.extend(watcher.stdout),daemon=True).start()
    wait_for("subscribed to audio creation events",lambda:a.cmd("pactl","info") and events)
    def starts():return sum("'new' on source-output" in l for l in events)
    def roster():
        return {e["sender"]+":"+e["content"]["device_id"]:e["event_id"] for e in
                http("GET",api+"/state",headers=headers[0]).json() if e["type"]==MEMBER and e["content"].get("application")=="m.call"}
    def messages(i,kind):
        lines=events_log[i].read_text().splitlines() if events_log[i].exists() else []
        return {e["event_id"]:(e["sender"],e["content"]) for e in map(json.loads,lines)
                if e["room_id"]==room and e["type"]==kind and e["event_id"].startswith("$")}
    def history():
        return http("GET",api+"/messages",headers=headers[0],params={"dir":"b","limit":100}).json()["chunk"]
    ready_ui()
    if not relay:
        a.click("voice")
        wait_for("explicit unsupported call-server state",lambda:{"no","supported","matrixrtc","livekit","service","advertised"}<=words(a))
        ready_ui()
        assert not roster() and not starts() and not audio("sink-inputs")
        assert not any(e["type"] in (MEMBER,"org.matrix.msc4143.rtc.slot","m.room.encrypted","m.call.invite",RING) for e in history())
        print("Synapse without RTC discovery: explicit 'No supported MatrixRTC LiveKit service advertised.' "
              "No call membership, ring, media stream or fallback invite; caller ready to retry.",flush=True)
        return

    def dial(callee):
        caller=1-callee
        previous=set(messages(callee,RING));n=starts()
        desks[caller].click("voice")
        wait_for("callee's rendered incoming invite",lambda:{"accept","decline"}<=words(desks[callee]),12)
        r=roster();assert set(r)=={ids[caller]}
        rings=messages(callee,RING)
        [eid]=set(rings)-previous
        u,c=rings[eid]
        assert u==users[caller] and c["m.relates_to"]=={"rel_type":"m.reference","event_id":r[ids[caller]]}
        assert c["m.mentions"]["user_ids"]==[users[callee]] and c["notification_type"]=="ring" and c["lifetime"]==45000
        assert starts()==n and not audio("source-outputs") and len(audio("sink-inputs"))==1
        wire=http("GET",api+"/event/"+quote(eid,safe=""),headers=headers[callee]).json()
        assert wire["type"]=="m.room.encrypted" and wire["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
        return eid,c["sender_ts"]/1000
    def ended():
        wait_for("empty MatrixRTC roster",lambda:not roster(),15)
        ready_ui()
        wait_for("released both endpoints",lambda:not audio("source-outputs") and not audio("sink-inputs"))
    def relay_stats(i,offset):
        matches=re.findall(r"^RTC selected audio (inbound|outbound) local=(\w+) relay_protocol=(\w+) packets=(\d+)$",
                           logs[i].read_text(errors="replace")[offset:],re.M)
        latest={dir:(kind,proto,int(count)) for dir,kind,proto,count in matches}
        return set(latest)=={"inbound","outbound"} and all(k=="Relay" and p=="Udp" and n>0 for k,p,n in latest.values())
    def publications():
        peers=sfu_participants(secret,allow_empty=True)
        if len(peers)!=2:return None
        out={}
        for p in peers:
            tracks=p.get("tracks",[])
            if not tracks:return None
            assert len(tracks)==1 and (tracks[0]["encryption"],tracks[0]["source"])==("GCM","MICROPHONE")
            out[p["identity"]]=(p["sid"],tracks[0]["sid"])
        assert set(out)==set(ids)
        return out
    def hears(left,right):
        for d in desks:d.windows.clear()
        wait_for(f"two decoded speaker monitors: {left}/{right}",lambda:a.hears(set(left)) and b.hears(set(right)),18)
    def memberships():
        return {e["sender"]+":"+e["content"]["device_id"]:(e["event_id"],e["content"]["created_ts"]) for e in
                http("GET",api+"/state",headers=headers[0]).json() if e["type"]==MEMBER and e["content"].get("application")=="m.call"}
    def rejoin(link,seen):
        # Past the 16 s delayed leave: the caller's membership expires. The
        # callee keeps the call (the caller did not hang up); the caller
        # rejoins it with a new membership and key, without ringing again.
        before=memberships()
        with outage(link):
            down=time.monotonic()
            hears([],[])
            wait_for("caller shows Reconnecting",lambda:"reconnecting" in {w for w,_,_ in b.words()},12)
            a.tone=2;b.tone=0
            wait_for("caller's delayed leave fired",lambda:set(memberships())=={ids[0]},25)
            expired=time.monotonic()-down
            wait_for("25-second outage",lambda:time.monotonic()-down>=25,25)
            assert set(memberships())=={ids[0]} and memberships()[ids[0]][1]==before[ids[0]][1]
        restored=time.monotonic()
        def back():
            now=memberships()
            return set(now)==set(ids) and now[ids[1]][1]>before[ids[1]][1]
        wait_for("caller rejoined with a new membership",back,45)
        hears([410],[1030])
        after=memberships()
        assert after[ids[0]][1]==before[ids[0]][1] and after[ids[1]][0]!=before[ids[1]][0]
        assert publications() and set(messages(0,RING))==seen
        print(f"Caller offline for 25 s; its membership expired after {expired:.1f}s. It rejoined {time.monotonic()-restored:.1f}s "
              f"after restoration with membership {after[ids[1]][0]} (created {after[ids[1]][1]-before[ids[1]][1]} ms later); "
              "the callee kept its own; fresh 410/1030 Hz both ways, two GCM microphones, no new ring.",flush=True)
    try:
        cancel,_=dial(1)
        a.click("hang");ended()
        decline,_=dial(0)
        a.click("decline");label(b,"declined");ended()
        [(u,c)]=messages(1,DECLINE).values()
        assert u==users[0] and c["m.relates_to"]=={"rel_type":"m.reference","event_id":decline}
        unanswered,begin=dial(1)
        wait_for("actual ring lifetime elapsed",lambda:"no" in (ws:=words(a)) and "answer" in ws,53)
        elapsed=time.time()-begin
        ended()
        assert 44<=elapsed<=48 and starts()==0,elapsed
        print(f"Caller canceled, callee declined exact invite {decline}, and third call expired unanswered in {elapsed:.1f}s; "
              "0 microphone starts across all three.",flush=True)
        accepted=[]
        for callee in (1,0):
            n=starts(); offsets=[len(f.read_text()) for f in logs];before=[counters(link) for link in links]
            accepted.append(dial(callee)[0]);desks[callee].click("accept")
            wait_for("exact pair of Matrix devices",lambda:set(roster())==set(ids),15)
            tracks=wait_for("two GCM microphones",publications)
            a.tone=0;b.tone=1
            hears([670],[410])
            assert len(audio("source-outputs"))==len(audio("sink-inputs"))==2 and starts()==n+2
            for i in range(2):
                wait_for("active native inbound and outbound relay pairs",lambda:relay_stats(i,offsets[i]),12)
            if callee==1:
                # Change rooms, gate the media path, then recover *new* tones
                # through TURN, on the original tracks with signaling still up.
                a.click("sidechat")
                assert "Sidechat" in a.cmd("xdotool","getwindowname",a.win)
                a.click("mute");hears([670],[])
                a.click("unmute");hears([670],[410])
                a.click("deafen");hears([],[])
                a.click("undeafen");hears([670],[410])
                with interrupt_turn(links[0]):
                    hears([],[])
                    assert set(roster())==set(ids)
                    a.tone=3;b.tone=4
                hears([830],[530])
                assert publications()==tracks
                pt=wait_for("DM in left sidebar",lambda:next(((x,y) for w,x,y in a.words() if w=="media-proof" and x<CHAT_X),None))
                a.cmd("xdotool","mousemove","--window",a.win,*map(str,pt),"click","1")
                wait_for("back in DM",lambda:"media-proof" in a.cmd("xdotool","getwindowname",a.win))
                print("Mute/deafen isolated the expected speaker while browsing Sidechat. Cutting only TURN silenced both; "
                      "fresh 830/530 Hz returned on the original GCM tracks.",flush=True)
            else:
                seen=set(messages(0,RING))
                with outage(links[1]):
                    down=time.monotonic()
                    hears([],[])
                    # While the call stage is closed, a DM call's status is
                    # only in the call dock at the bottom of the sidebar.
                    label(b,"reconnecting",dock=True)
                    a.tone=3;b.tone=4
                    wait_for("six-second full outage",lambda:time.monotonic()-down>=6.5,8)
                    assert set(roster())==set(ids)
                restored=time.monotonic()
                hears([830],[530])
                assert set(roster())==set(ids) and publications() and set(messages(0,RING))==seen
                print(f"Native caller lost its full network link. After restoration both speakers decoded fresh 830/530 Hz "
                      f"in {time.monotonic()-restored:.1f}s; still two devices and no new ring.",flush=True)
                rejoin(links[1],seen)
            for i,link in enumerate(links):
                delta={k:v-before[i][k] for k,v in counters(link).items()}
                assert delta["sfu_drop"]>0 and min(delta["turn_tx"],delta["turn_rx"])>10,delta
                print(f"{ids[i]}: inbound/outbound relay/UDP; {delta['turn_tx']}/{delta['turn_rx']} TURN packets; "
                      f"{delta['sfu_drop']} direct SFU media packets blocked.",flush=True)
            desks[1-callee].click("hang");ended()
            print(f"Accepted {accepted[-1]}: two distinct voices decoded in 12 consecutive 100 ms windows, "
                  "both publications GCM; hangup released both devices.",flush=True)
        assert len({cancel,decline,unanswered,*accepted})==5
        for i in range(2):
            assert set(messages(i,RING))=={cancel,decline,unanswered,*accepted} and len(messages(i,DECLINE))==1
        assert not any(e["type"]=="m.call.invite" for e in history())
    except Exception:
        print("Desktop labels:",[sorted(words(d)) for d in desks],"roster:",roster(),flush=True)
        raise
