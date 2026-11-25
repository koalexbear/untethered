"""Private client networks for RTC tests.

Use only within run_element.sh's disposable outer namespace. Each client's
veth is its only non-loopback path. Preserve the test services' advertised
URLs without sharing the services' loopback interface.
"""
import json
import os
import subprocess as sp
from contextlib import contextmanager
from dataclasses import dataclass


@dataclass
class ClientLink:
    prefix: list
    server_if: str
    gateway: str

    def inside(self,*args,**kw):
        return sp.run([*self.prefix,*args],check=True,capture_output=True,text=True,timeout=5,**kw)


def client_link(stack, ports, role):
    def run(*args, **kw):
        return sp.run(args, check=True, capture_output=True, text=True, timeout=5, **kw)

    interfaces=json.loads(run("ip","-j","addr").stdout)
    assert role in ("desktop","element")
    assert {i["ifname"] for i in interfaces} <= {"lo","rtc-test","qa-desktop-gw","qa-element-gw"}
    assert any(a["local"]=="192.0.2.2" for i in interfaces for a in i["addr_info"])
    ns=sp.Popen(["unshare","-n","sh","-ec","ip link set lo up; echo ready; exec cat"],
                stdin=sp.PIPE, stdout=sp.PIPE, text=True)
    def stop():
        ns.terminate(); ns.wait(timeout=5)
    stack.callback(stop)
    assert ns.stdout.readline().strip()=="ready"
    assert os.stat(f"/proc/{ns.pid}/ns/net").st_ino!=os.stat("/proc/self/ns/net").st_ino
    prefix=["nsenter","-t",str(ns.pid),"-n","--"]
    def inside(*args, **kw):return run(*prefix,*args,**kw)
    iface=f"qa-{role}-gw"
    nic=f"qa-{role}"
    gateway,local=("198.51.100.1","198.51.100.2") if role=="desktop" else ("198.51.100.5","198.51.100.6")
    run("ip","link","add",iface,"type","veth","peer","name",nic,"netns",str(ns.pid))
    stack.callback(run,"ip","link","del",iface)
    for execute,dev,ip in [(run,iface,gateway),(inside,nic,local)]:
        execute("ip","addr","add",ip+"/30","dev",dev)
        execute("ip","link","set",dev,"up")
        execute("sysctl","-qw",f"net.ipv4.conf.{dev}.route_localnet=1")
    inside("ip","route","add","192.0.2.0/24","via",gateway)
    ports=", ".join(str(int(p)) for p in ports)
    table=f"qa_rtc_{role}"
    run("nft","add","table","ip",table)
    stack.callback(run,"nft","delete","table","ip",table)
    run("nft","-f","-",input=f"""add chain ip {table} pre {{ type nat hook prerouting priority dstnat; }}
add rule ip {table} pre iifname "{iface}" ip daddr {gateway} tcp dport {{ {ports} }} dnat to 127.0.0.1
""")
    inside("nft","-f","-",input=f"""add table ip qa_rtc
add chain ip qa_rtc out {{ type nat hook output priority dstnat; }}
add chain ip qa_rtc post {{ type nat hook postrouting priority srcnat; }}
add rule ip qa_rtc out ip daddr 127.0.0.1 tcp dport {{ {ports} }} dnat to {gateway}
add rule ip qa_rtc post oifname "{nic}" ip saddr 127.0.0.1 snat to {local}
""")
    return ClientLink(prefix,iface,gateway)


def desktop_network(stack, ports):
    link=client_link(stack,ports,"desktop")
    return link.prefix,lambda:outage(link)


@contextmanager
def outage(link):
    # Remove connectivity in both directions and reset the SFU socket, as on
    # a lost access network. Don't wait for the kernel TCP timeout.
    endpoint=["dst","127.0.0.1","dport","=",":18820"]
    assert link.inside("ss","-Htn","state","established",*endpoint).stdout.strip()
    sp.run(["ip","link","set",link.server_if,"down"],check=True)
    try:
        assert link.inside("ss","-HKtn","state","established",*endpoint).stdout.strip()
        yield
    finally:
        sp.run(["ip","link","set",link.server_if,"up"],check=True)
