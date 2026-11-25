"""Federated ban acceptance on two real Synapse homeservers confined to loopback."""
import json
from contextlib import ExitStack
import subprocess as sp
import sys

import requests
from qa_native_paste import wait_for
from qa_replies import test_replies


def federated_ban(tmp,root,backend,seed,http,free_port,service):
    links=json.loads(sp.check_output(["ip","-j","link","show"]))
    assert {link["ifname"] for link in links}=={"lo"}, "Run in a loopback-only test network namespace"
    ports=set()
    while len(ports)<4:ports.add(free_port())
    port=list(ports)
    names=[f"localhost:{p}" for p in port[:2]]
    urls=[f"http://127.0.0.1:{p}" for p in port[2:]]
    def openssl(*args):
        sp.run(["openssl",*args],cwd=tmp,check=True,stdout=sp.DEVNULL,stderr=sp.DEVNULL)
    openssl("req","-x509","-newkey","rsa:2048","-nodes","-days","1","-subj","/CN=Ban QA CA",
            "-addext","basicConstraints=critical,CA:TRUE","-keyout","ca.key","-out","ca.crt")
    openssl("req","-new","-newkey","rsa:2048","-nodes","-subj","/CN=127.0.0.1","-keyout","tls.key","-out","tls.csr")
    (tmp/"ext").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n")
    openssl("x509","-req","-in","tls.csr","-CA","ca.crt","-CAkey","ca.key","-CAcreateserial",
            "-days","1","-extfile","ext","-out","tls.crt")
    with ExitStack() as stack:
        for i,name in enumerate(names):
            d=tmp/f"synapse-{i}";d.mkdir()
            cmd=[sys.executable,"-m","synapse.app.homeserver","-c","hs.yaml"]
            sp.run([*cmd,"--generate-config","-H",name,"--report-stats","no"],cwd=d,check=True,stdout=sp.DEVNULL)
            (d/"local.yaml").write_text(f"""listeners:
  - port: {port[i]}
    type: http
    tls: true
    bind_addresses: ['127.0.0.1','::1']
    resources: [{{names: [federation]}}]
  - port: {port[2+i]}
    type: http
    tls: false
    bind_addresses: ['127.0.0.1']
    resources: [{{names: [client, media]}}]
public_baseurl: {urls[i]}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: ['{names[1-i]}']
ip_range_whitelist: ['127.0.0.1/32','::1/128']
federation_custom_ca_list: ['{tmp}/ca.crt']
tls_certificate_path: '{tmp}/tls.crt'
tls_private_key_path: '{tmp}/tls.key'
""")
            p=stack.enter_context(service([*cmd,"-c","local.yaml"],d))
            def ready():
                assert p.poll() is None,"Disposable Synapse failed to start"
                try:return requests.get(urls[i]+"/_matrix/client/versions",timeout=3).ok
                except requests.ConnectionError:return False
            wait_for("federated Synapse",ready)
        print(f"Independent loopback Synapse identities: {names[0]} (moderator), {names[1]} (member).",flush=True)
        test_replies(tmp,urls[0],root,backend,seed,http,free_port(),ban=True,peer_hs=urls[1])
