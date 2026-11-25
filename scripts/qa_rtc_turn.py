"""TURN-only acceptance: coturn, two independently firewalled client links, actual ICE stats."""
import json
import re
import secrets
import shlex
import shutil
import subprocess as sp
from contextlib import contextmanager
from urllib.parse import urlsplit

from qa_native_paste import wait_for
from qa_rtc_link import client_link

TURN_IP="192.0.2.3"
TURN_PORT=3478


def configure_turn(tmp,stack,service):
    interfaces=json.loads(sp.check_output(["ip","-j","addr"]))
    assert {i["ifname"] for i in interfaces}=={"lo","rtc-test"}
    assert any(a["local"]=="192.0.2.2" for i in interfaces for a in i["addr_info"])
    sp.run(["ip","addr","add",TURN_IP+"/32","dev","rtc-test"],check=True)
    stack.callback(sp.run,["ip","addr","del",TURN_IP+"/32","dev","rtc-test"],check=True)
    password=secrets.token_hex(24)
    conf=tmp/"coturn.conf"
    conf.write_text(f"""listening-ip={TURN_IP}
relay-ip={TURN_IP}
listening-port={TURN_PORT}
min-port=49160
max-port=49200
realm=rtc.test.invalid
lt-cred-mech
user=qa:{password}
no-cli
no-tls
no-dtls
no-tcp
no-tcp-relay
no-multicast-peers
relay-threads=1
userdb={tmp}/turn.sqlite
pidfile={tmp}/turn.pid
log-file=stdout
denied-peer-ip=0.0.0.0-255.255.255.255
allowed-peer-ip=192.0.2.2
""")
    server=stack.enter_context(service(["turnserver","-c",str(conf)],tmp))
    def ready():
        assert server.poll() is None,"private coturn exited"
        return f"{TURN_IP}:{TURN_PORT}" in sp.check_output(["ss","-Huln"],text=True)
    wait_for("coturn UDP listener",ready)
    with (tmp/"lk.yaml").open("a") as cfg:
        # Under rtc: LiveKit supplies the real TURN credentials in JoinResponse.
        cfg.write(f"""  turn_servers:
    - host: {TURN_IP}
      port: {TURN_PORT}
      protocol: udp
      username: qa
      credential: "{password}"
""")


def relay_links(stack,ports):
    links={role:client_link(stack,ports,role) for role in ("desktop","element")}
    for link in links.values():
        allowed=", ".join(str(int(p)) for p in ports)
        # INPUT and OUTPUT are default-deny for both address families. Local
        # IPC stays local; only explicitly named TCP control ports and coturn
        # UDP are reachable across the veth. Neither side can dial SFU ICE.
        link.inside("nft","-f","-",input=f"""add table inet relaytest
add counter inet relaytest turn_tx
add counter inet relaytest turn_rx
add counter inet relaytest sfu_drop
add chain inet relaytest gate
add chain inet relaytest out {{ type filter hook output priority filter; policy drop; }}
add chain inet relaytest in {{ type filter hook input priority filter; policy drop; }}
add rule inet relaytest out oifname "lo" accept
add rule inet relaytest out ip daddr {link.gateway} tcp dport {{ {allowed} }} accept
add rule inet relaytest out ip daddr {TURN_IP} udp dport {TURN_PORT} jump gate
add rule inet relaytest out ip daddr {TURN_IP} udp dport {TURN_PORT} counter name turn_tx accept
add rule inet relaytest out ip daddr 192.0.2.2 udp dport 18822 counter name sfu_drop drop
add rule inet relaytest out ip daddr 192.0.2.2 tcp dport 18821 counter name sfu_drop drop
add rule inet relaytest in iifname "lo" accept
add rule inet relaytest in meta l4proto tcp ct state established accept
add rule inet relaytest in ip saddr {TURN_IP} udp sport {TURN_PORT} jump gate
add rule inet relaytest in ip saddr {TURN_IP} udp sport {TURN_PORT} counter name turn_rx accept
""")
    return links


def relay_clients(stack,tmp,ports):
    links=relay_links(stack,ports)
    exe=tmp/"element-chromium"
    exe.write_text("#!/bin/sh\nexec "+shlex.join([*links["element"].prefix,shutil.which("chromium")])+' "$@"\n')
    exe.chmod(0o700)
    return links,str(exe)


def counters(link):
    values=json.loads(link.inside("nft","-j","list","counters","inet","relaytest").stdout)["nftables"]
    return {x["counter"]["name"]:x["counter"]["packets"] for x in values if "counter" in x}


@contextmanager
def interrupt_turn(link):
    # Only the allow-listed TURN flow is gated. SFU and Matrix signaling stay up.
    link.inside("nft","add","rule","inet","relaytest","gate","counter","drop")
    try:yield
    finally:link.inside("nft","flush","chain","inet","relaytest","gate")


def verify_turn(links,widget,speaker,new_tone,native_log,roster,publications,users):
    def routes():
        return widget.evaluate("window.qaIceRoutes()")
    def browser_relay():
        r=routes()
        return r if {s["direction"] for s in r}=={"inbound","outbound"} and all(
            s.get("local")=="relay" and s.get("relayProtocol")=="udp" and s.get("state")=="succeeded"
            and urlsplit(s.get("url","").replace("turn:","turn://",1)).hostname==TURN_IP for s in r) else None
    pairs=wait_for("selected Element TURN pairs for active inbound and outbound audio",browser_relay,10)
    print("Element selected ICE: "+", ".join(f"{r['direction']}={r['local']}/{r['relayProtocol']}" for r in pairs),flush=True)
    def native_relay():
        rows=re.findall(r"^RTC selected audio (inbound|outbound) local=(\w+) relay_protocol=(\w+) packets=(\d+)$",
                        native_log.read_text(errors="replace"),re.M)
        latest={direction:(kind,proto,int(packets)) for direction,kind,proto,packets in rows}
        return ({d:count for d,(k,p,count) in latest.items()}
                if set(latest)=={"inbound","outbound"} and all((k,p)==("Relay","Udp") for k,p,_ in latest.values()) else None)
    initial=wait_for("native selected ICE pairs from libwebrtc transport stats",native_relay,12)
    print("Untethered selected ICE: inbound=relay/udp, outbound=relay/udp.",flush=True)
    before={role:counters(link) for role,link in links.items()}
    # Counterfactual: remove TURN (but not the control channels). Neither
    # original tone can arrive over direct ICE as an accidental fallback.
    stats=lambda:widget.evaluate("window.qaAudioStats()")
    wait_for("both original tones before gating TURN",lambda:stats()["run480"]>=12 and speaker()[1]>=12,8)
    with interrupt_turn(links["desktop"]):
        spec,monitor=stats()["spectra"],speaker()[0]
        def silent():
            s=stats();n,_,quiet=speaker()
            return min(s["quiet"],s["spectra"]-spec,quiet,n-monitor)>=10
        wait_for("both tones stopped without the relay",silent,7)
        assert sorted(roster())==sorted(users) and len(publications())==2
        new_tone.set()
    start_spec,start_monitor=stats()["spectra"],speaker()[0]
    def recovered():
        s=stats();n,tone,_=speaker()
        return min(s["run730"],s["spectra"]-start_spec,tone,n-start_monitor)>=12
    wait_for("fresh bidirectional relayed tones",recovered,15)
    assert browser_relay()
    wait_for("fresh native counts on selected relays",lambda:(r:=native_relay()) and all(r[d]>n for d,n in initial.items()),10)
    assert sorted(roster())==sorted(users)
    tracks=publications()
    assert len(tracks)==2 and {u.rsplit(":",1)[0] for u in tracks}==set(users)
    assert all(t==["GCM"] for t in tracks.values())
    for role,link in links.items():
        counts=counters(link)
        tx=counts["turn_tx"]-before[role]["turn_tx"];rx=counts["turn_rx"]-before[role]["turn_rx"]
        assert min(tx,rx)>10 and counts["sfu_drop"]>0
        print(f"{role}: +{tx} sent / +{rx} received packets via coturn; blocked {counts['sfu_drop']} packets to direct SFU media ports.",flush=True)
    print("Blocking only TURN silenced both ends; restoring it delivered fresh 730 Hz at Element and 960 Hz at Untethered for 1.2 s, both microphone publications GCM.",flush=True)
