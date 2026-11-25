"""Two joined accounts: exercise target-specific membership UI and independent server auth."""
from urllib.parse import quote,unquote,urlsplit
import re
import requests
from qa_native_paste import wait_for
from qa_ui import CHAT_X


def check_removal(page,api,room,peer_hs,admin,member,http,locate,click,restart,x,ui,*,kick=False):
    hs=api.split("/_matrix/")[0]
    moderator=http("GET",hs+"/_matrix/client/v3/account/whoami",headers=admin).json()["user_id"]
    peer=page.evaluate("()=>window.mxMatrixClientPeg.get().getSafeUserId()")
    assert moderator!=peer
    member_url=api+"/state/m.room.member/"+quote(peer,safe="")
    def current():
        return http("GET",member_url,headers=admin).json()
    assert current()["membership"]=="join"
    levels=http("GET",api+"/state/m.room.power_levels",headers=admin).json()
    action="kick" if kick else "ban"
    assert levels.get("users",{}).get(peer,levels.get("users_default",0))<levels.get(action,50)
    # Public at the time of the failed join: a lack of invitation cannot
    # explain this refusal. Switch back to invite-only before lifting the ban.
    def joins(rule):
        http("PUT",api+"/state/m.room.join_rules",headers=admin,json={"join_rule":rule})
        assert http("GET",api+"/state/m.room.join_rules",headers=admin).json()["join_rule"]==rule
    if kick:
        assert http("GET",api+"/state/m.room.join_rules",headers=admin).json()["join_rule"]=="public"
    else:joins("public")
    http("PUT",(peer_hs or hs)+"/_matrix/client/v3/profile/"+quote(peer,safe="")+"/displayname",headers=member,json={"displayname":"Bruno"})
    wait_for("joined target has room display name",lambda:current().get("displayname")=="Bruno",15)
    def point(ws,word,min_x):return next(((xx,y) for w,xx,y in ws if w==word and xx>min_x),None)
    def tap(word,min_x=910):click(locate(word,lambda ws:point(ws,word,min_x)))
    def people():
        # The member list icon in the channel header.
        click(wait_for("Member List header control",lambda:ui.header("member list"),20))
    def selected(kind):
        locate("selected "+kind,lambda ws:any(abs(y-yy)<6 for w,xx,y in ws if xx>910 and w=="membership"
                                               for v,xxx,yy in ws if xxx>xx and v==kind))
    def observed(kind):
        return wait_for("Synapse membership="+kind,lambda:s if (s:=current())["membership"]==kind else None)
    def synced(kind):
        page.wait_for_function("([r,k])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMyMembership()===k",arg=[room,kind],timeout=20000)
    def join(status,code=None):
        def is_join(r):
            path=unquote(urlsplit(r.url).path)
            return r.request.method=="POST" and (path.endswith("/join/"+room) or path.endswith("/rooms/"+room+"/join"))
        with page.expect_response(is_join) as wire:
            result=page.evaluate("""async ([r,via])=>{
              try{return {room:(await window.mxMatrixClientPeg.get().joinRoom(r,{viaServers:[via]})).roomId}}
              catch(e){return {code:e.errcode}}
            }""",[room,moderator.split(":",1)[1]])
        response=wire.value
        assert response.status==status,(response.status,response.json(),result)
        if status==403:
            assert response.json()["errcode"]==result["code"]==code,(response.json(),result)
            return response.json()
        else:
            assert response.json()["room_id"]==result["room"]==room
    def is_send(r):return r.request.method=="PUT" and f"/rooms/{room}/send/m.room.encrypted/" in unquote(urlsplit(r.url).path)
    def send(body):
        with page.expect_response(is_send) as wire:
            eid=page.evaluate("async ([r,b])=>(await window.mxMatrixClientPeg.get().sendTextMessage(r,b)).event_id",[room,body])
        assert wire.value.status==200 and wire.value.json()["event_id"]==eid
        page.locator(f'[data-event-id="{eid}"]').get_by_text(body,exact=True).wait_for()
        a,b=body.lower().split()
        locate("Untethered decrypted "+body,lambda ws:any(abs(y-yy)<6 for w,xx,y in ws if w==a and xx>CHAT_X
            for v,xxx,yy in ws if v==b and xx<xxx<xx+200))
        # A send is acknowledged by the sender's server before federation
        # necessarily reaches the admin's server; inspect the latter after receipt.
        event=http("GET",api+"/event/"+quote(eid,safe=""),headers=admin).json()
        assert event["sender"]==peer and event["type"]=="m.room.encrypted" and "ciphertext" in event["content"]
        return eid
    baseline=send("Arrival beacon")
    denied=requests.post((peer_hs or hs)+"/_matrix/client/v3/rooms/"+quote(room,safe="")+"/"+action,headers=member,json={"user_id":moderator},timeout=10)
    assert denied.status_code==403
    people();tap("bruno");selected("joined")
    reason="Room conduct test"
    tap("reason");x("type","--clearmodifiers",reason)
    if kick:
        # Once it sees `leave`, Element may decline to encrypt another event.
        # Hold a fresh, genuine encrypted PUT before it reaches the network,
        # then let Synapse authorize it only after the kick is established.
        held=[]
        page.route(re.compile(r"/send/m\.room\.encrypted/"),lambda route:held.append(route),times=1)
        page.evaluate("""r=>{
            const c=window.mxMatrixClientPeg.get();
            window.qaKickSend=c.sendTextMessage(r,'Outside message').catch(e=>{
                if(e.event?.status==='not_sent')c.cancelPendingEvent(e.event);
                return {errcode:e.errcode};
            });
        }""",room)
        route=wait_for("fresh Megolm send ready",lambda:held[0] if held else page.evaluate("false"))
        assert route.request.method=="PUT" and route.request.post_data_json["algorithm"]=="m.megolm.v1.aes-sha2"
        tap("kick")
        left=observed("leave");assert left.get("reason")==reason
        selected("left");synced("leave")
        with page.expect_response(is_send) as wire:
            route.continue_()
        result=page.evaluate("window.qaKickSend")
        assert wire.value.status==403 and wire.value.json()["errcode"]==result["errcode"]=="M_FORBIDDEN",(wire.value.status,wire.value.json())
        plain=requests.put(api+"/send/m.room.message/after-kick",headers=member,json={"msgtype":"m.text","body":"Outside plain"},timeout=10)
        assert plain.status_code==403 and plain.json()["errcode"]=="M_FORBIDDEN",plain.text
        chunk=http("GET",api+"/messages",headers=admin,params={"dir":"b","limit":100}).json()["chunk"]
        assert {e["event_id"] for e in chunk if e["type"] in ("m.room.message","m.room.encrypted")}=={baseline}
        print("Kick: Synapse membership=leave with the supplied reason. Encrypted and plain member sends while out "
              "returned HTTP 403 M_FORBIDDEN; only the pre-kick message exists in history.",flush=True)
        join(200);observed("join");synced("join");selected("joined")
        page.goto(page.url.split("#")[0]+"#/room/"+room)
        eid=send("Public return")
        chunk=http("GET",api+"/messages",headers=admin,params={"dir":"b","limit":100}).json()["chunk"]
        assert {e["event_id"] for e in chunk if e["type"] in ("m.room.message","m.room.encrypted")}=={baseline,eid}
        transitions=[(e["sender"],e["content"]["membership"]) for e in reversed(chunk)
                     if e["type"]=="m.room.member" and e.get("state_key")==peer]
        assert transitions[-2:]==[(moderator,"leave"),(peer,"join")] and all(k in ("join","leave") for _,k in transitions),transitions
        assert http("GET",api+"/state/m.room.join_rules",headers=admin).json()["join_rule"]=="public"
        assert http("GET",api+"/state/m.room.power_levels",headers=admin).json()==levels
        print(f"Public rejoin HTTP 200, zero invite or ban events for the member. Post-rejoin Megolm send HTTP 200 {eid}; "
              "Element and Untethered displayed 'Public return'.",flush=True)
        return
    tap("ban")
    banned=observed("ban");assert banned.get("reason")==reason
    synced("ban");error=join(403,"M_FORBIDDEN" if peer_hs else "M_BAD_STATE")
    assert "ban" in error["error"].lower(),error
    assert current()["membership"]=="ban"
    print(f"Public join rule confirmed. Synapse refused the banned Element member's rejoin: HTTP 403 {error['errcode']} ({error['error']}).",flush=True)
    restart()
    locate("restored pre-ban message",lambda ws:point(ws,"arrival",CHAT_X)) # Header has finished loading.
    people()
    # Ban state may not include a display name; select the row under the
    # roster's BANNED heading.
    click(locate("Banned roster row",lambda ws:(lambda p:p and (p[0],p[1]+32))(point(ws,"banned",910))))
    selected("banned")
    joins("invite")
    tap("unban");observed("leave");selected("left")
    join(403,"M_FORBIDDEN") # An unban has not silently created an invitation.
    tap("invite");observed("invite");synced("invite")
    join(200);observed("join");synced("join")
    page.goto(page.url.split("#")[0]+"#/room/"+room)
    selected("joined")
    eid=send("Returned lantern")
    timeline=http("GET",api+"/messages",headers=admin,params={"dir":"b","limit":100}).json()["chunk"]
    assert {e["event_id"] for e in timeline if e["type"] in ("m.room.encrypted","m.room.message")}=={baseline,eid}
    transitions=[e for e in reversed(timeline) if e["type"]=="m.room.member" and e["state_key"]==peer]
    assert [(e["sender"],e["content"]["membership"]) for e in transitions[-4:]]==[
        (moderator,"ban"),(moderator,"leave"),(moderator,"invite"),(peer,"join")]
    print("Cold restart restored the Banned row. Unban → leave (invite-only rejoin still 403); "
          "Iced Invite → invite; same Element account rejoined with HTTP 200.",flush=True)
    print(f"Post-rejoin Megolm send HTTP 200 {eid}; Element and Untethered both displayed 'Returned lantern'.",flush=True)
