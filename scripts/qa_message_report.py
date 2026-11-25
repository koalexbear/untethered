"""Read the actual moderation queue independently of the reporting client."""
import json
import secrets

import requests
import yaml
from qa_native_paste import wait_for
from qa_ui import CHAT_X
from synapse._scripts.register_new_matrix_user import request_registration


def check_reports(tmp,hs,api,room,page,hdr,http,messages,locate,click,write,enter,ui):
    queue=hs+"/_synapse/admin/v1/event_reports"
    reporter=http("GET",hs+"/_matrix/client/v3/account/whoami",headers=hdr).json()["user_id"]
    sender=page.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()")
    assert reporter!=sender
    levels=http("GET",api+"/state/m.room.power_levels",headers=hdr).json()
    assert levels.get("users",{}).get(reporter,levels.get("users_default",0))==0
    denied=requests.get(queue,headers=hdr,timeout=5)
    assert denied.status_code==403 and denied.json()["errcode"]=="M_FORBIDDEN"
    # Admin credentials stay inside this disposable fixture, never in either client.
    password=secrets.token_urlsafe(24)
    request_registration("reviewer",password,hs,yaml.safe_load((tmp/"hs.yaml").read_text())["registration_shared_secret"],admin=True)
    reviewer=http("POST",hs+"/_matrix/client/v3/login",json={"type":"m.login.password",
        "identifier":{"type":"m.id.user","user":"reviewer"},"password":password}).json()
    admin={"Authorization":"Bearer "+reviewer["access_token"]}
    def listing():return http("GET",queue,headers=admin).json()
    assert listing()["total"]==0

    text="Orchid duplicate"
    first,second=messages([text,text])
    assert first!=second
    def points(ws,label):return [(xx,y) for w,xx,y in ws if w==label and xx>CHAT_X]
    def select(label,index,count):
        def target(ws):
            rows=sorted(points(ws,label),key=lambda p:p[1])
            return rows[index] if len(rows)==count else None
        # Report Message is in the toolbar of the hovered row.
        row=locate("selected "+label,target)
        click(wait_for("report on selected "+label,lambda:ui.message(row,"report message"),15))
        locate("report form visible",lambda ws:points(ws,"receives") and points(ws,"cancel"))
    def tap(label):click(locate(label,lambda ws:next(iter(points(ws,label)),None)))
    def confirmed():locate("homeserver accepted report",lambda ws:points(ws,"sent") and not points(ws,"submit"))
    select("orchid",0,2)
    # A blank reason and a discarded draft must not reach the queue.
    enter();write("Discard this reason");tap("cancel")
    assert listing()["total"]==0
    select("orchid",1,2)
    reason="Repeated off-topic invitations"
    write(reason)

    # Change the selected row while its report form is open. The pending
    # draft still refers to the version the reporter actually saw.
    changed="Revised sunflower"
    edit=page.evaluate("""async ([room,target,body])=>(await window.mxMatrixClientPeg.get().sendEvent(room,'m.room.message',{
        msgtype:'m.text',body:'* '+body,'m.new_content':{msgtype:'m.text',body},
        'm.relates_to':{rel_type:'m.replace',event_id:target}})).event_id""",[room,second,changed])
    locate("incoming edit shown beside open report",lambda ws:points(ws,"edited") and points(ws,"revised") and points(ws,"submit"))
    tap("submit");confirmed()
    def inspect(target,why,total):
        result=listing()
        assert result["total"]==len(result["event_reports"])==total,result
        rows=[r for r in result["event_reports"] if r["event_id"]==target]
        assert len(rows)==1
        row=rows[0];detail=http("GET",queue+"/"+str(row["id"]),headers=admin).json()
        expected=dict(room_id=room,event_id=target,user_id=reporter,sender=sender,reason=why)
        for rec in (row,detail):
            assert {k:rec[k] for k in expected}==expected
        event=detail["event_json"]
        assert event["type"]=="m.room.encrypted" and event["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
        assert all(t not in json.dumps(detail) for t in (text,changed,"Discard this reason"))
        print("Synapse admin report:",json.dumps(expected,ensure_ascii=False),flush=True)
    inspect(second,reason,1)
    # A new report on the now-edited text should reference the replacement.
    select("revised",0,1)
    new_reason="Follow-up on edited invitation"
    write(new_reason);enter(2);confirmed()
    inspect(edit,new_reason,2)
    assert {r["event_id"] for r in listing()["event_reports"]}=={second,edit}
    chunk=http("GET",api+"/messages",headers=hdr,params={"dir":"b","limit":100}).json()["chunk"]
    visible=[e for e in chunk if e["type"] in ("m.room.message","m.room.encrypted","m.room.redaction")]
    assert len(visible)==3 and {e["event_id"] for e in visible}=={first,second,edit}
    assert all(e["type"]=="m.room.encrypted" and "ciphertext" in e["content"] for e in visible)
    print("Power-level-0 reporter: admin queue access denied (403). Canceled look-alike: 0 reports. "
          "Selected original and visible edit: 1 report each. Room history: 3 Megolm events, no redactions or report messages.",flush=True)
