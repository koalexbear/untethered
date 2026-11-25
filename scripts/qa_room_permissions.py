"""An Iced admin, a joined Element member and real homeserver authorization."""
import copy
from urllib.parse import unquote,urlsplit

import requests
from qa_native_paste import wait_for


def check_permissions(page,api,room,native,admin,member,http,locate,click,restart,x):
    mod="@"+native["username"]+":test.invalid"
    peer=page.evaluate("()=>window.mxMatrixClientPeg.get().getSafeUserId()")
    assert peer!=mod
    endpoint=api+"/state/m.room.power_levels"
    def state():return http("GET",endpoint,headers=admin).json()
    def write(content):return http("PUT",endpoint,headers=admin,json=content).json()["event_id"]
    base=state()
    assert base.get("users",{}).get(peer,base.get("users_default",0))==0
    # Exercise explicit, inherited, stricter and unknown permissions. An
    # implementation that replaces the room with a canned preset must fail.
    base.pop("events_default",None)
    base["events"].update({"m.room.encrypted":0,"m.room.message":0,"m.reaction":0,"m.sticker":70,"org.example.audit":80})
    base["org.example.room_policy"]={"label":"Retain this extension","version":3}
    write(base)
    def point(ws,label):return next(((xx,y) for w,xx,y in ws if w==label and xx>280),None)
    def tap(w):click(locate(w,lambda ws:point(ws,w)))
    def notice(w):locate(w,lambda ws:point(ws,w))
    def timeline():
        return {e["event_id"]:e for e in http("GET",api+"/messages",headers=admin,params={"dir":"b","limit":100}).json()["chunk"]
                if e["type"] in ("m.room.encrypted","m.room.message")}
    def visible(event_id,body):
        event=http("GET",api+"/event/"+event_id,headers=admin).json()
        assert event["type"]=="m.room.encrypted" and event["sender"]==peer and "ciphertext" in event["content"]
        page.locator(f'[data-event-id="{event_id}"]').get_by_text(body,exact=True).wait_for()
        first,second=body.lower().split()
        locate("decrypted "+body,lambda ws:any(any(w==second and x<xx<x+160 and abs(y-yy)<5 for w,xx,yy in ws)
                                              for word,x,y in ws if word==first and x>280))

    def send(body,status):
        def is_send(response):
            return response.request.method=="PUT" and f"/rooms/{room}/send/m.room.encrypted/" in unquote(urlsplit(response.url).path)
        with page.expect_response(is_send) as reply:
            result=page.evaluate("""async ([r,text])=>{
                const c=window.mxMatrixClientPeg.get();
                try{return await c.sendTextMessage(r,text)}catch(e){
                    if(e.event?.status==='not_sent')c.cancelPendingEvent(e.event);
                    return {errcode:e.errcode};
                }
            }""",[room,body])
        wire=reply.value
        assert wire.status==status,(wire.status,wire.json())
        if status==403:
            assert wire.json()["errcode"]==result["errcode"]=="M_FORBIDDEN",(wire.json(),result)
        else:
            assert wire.json()["event_id"]==result["event_id"]
            visible(result["event_id"],body)
        return result
    baseline=send("Baseline lantern",200)["event_id"]
    tap("permissions")
    tap("make")
    def read_locked():
        p=state()
        return p if p.get("events_default")==50 else None
    locked=wait_for("homeserver read-only power levels",read_locked,15)
    notice("restore")
    expected=copy.deepcopy(base)
    expected["events_default"]=50
    expected["events"].update({"m.room.message":50,"m.room.encrypted":50,"m.reaction":50})
    assert {k:v for k,v in locked.items() if k!="com.untethered.posting_restore.v1"}==expected,locked
    denied=send("Blocked member message",403)
    plain=requests.put(api+"/send/m.room.message/plain-test",headers=member,json={"msgtype":"m.text","body":"Denied plain message"},timeout=10)
    assert plain.status_code==403 and plain.json()["errcode"]=="M_FORBIDDEN"
    assert set(timeline())=={baseline}
    print("Element member: encrypted baseline HTTP 200; same room after Make read-only: "
          f"HTTP 403 {denied['errcode']}. Plain text also 403 M_FORBIDDEN. No denied message in Synapse history.",flush=True)
    # A member also cannot undo the policy via the normal Matrix state API.
    refused=requests.put(endpoint,headers=member,json=base,timeout=10)
    assert refused.status_code==403 and refused.json()["errcode"]=="M_FORBIDDEN" and state()==locked
    x("mousemove","--window","WIN","620","709","click","1")
    x("type","--clearmodifiers","Moderator bulletin");x("key","Return")
    page.get_by_text("Moderator bulletin",exact=True).wait_for()
    after_admin=timeline()
    assert len(after_admin)==2 and any(e["sender"]==mod and e["type"]=="m.room.encrypted" for e in after_admin.values())
    # Another authorized device changes an unrelated rule and one of the
    # locked rules. Restoration must keep both current values.
    locked["events"]["org.example.audit"]=85
    locked["events"]["m.reaction"]=65
    write(locked)
    restart()
    tap("permissions");tap("restore")
    restored=copy.deepcopy(base)
    restored["events"]["org.example.audit"]=85
    restored["events"]["m.reaction"]=65
    wait_for("exact policy restored without reverting another admin change",lambda:state()==restored,15)
    notice("make")
    delivered=send("Restored delivery",200)["event_id"]
    assert set(timeline())==set(after_admin)|{delivered}
    print("Member's permission rewrite: HTTP 403 M_FORBIDDEN; administrator posted while locked. "
          "Restore after restart preserved roles, stricter sticker level and later changes to audit (85) and reactions (65).",flush=True)
    print(f"Restored member send: HTTP 200 {delivered}; original Megolm event shown in Element and "
          "decrypted as 'Restored delivery' in Untethered.",flush=True)
