"""Drive the setup form, then make real membership and posting attempts as the invited user."""
from urllib.parse import quote
import requests

from qa_native_paste import wait_for
from qa_ui import CHAT_X


def check_setup(hs,http,admin,member,peer,locate,click,write,restart,paths,ui):
    api=hs+"/_matrix/client/v3"
    me=http("GET",api+"/account/whoami",headers=admin).json()["user_id"]
    other=peer.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()")
    assert me!=other
    def rooms():return set(http("GET",api+"/joined_rooms",headers=admin).json()["joined_rooms"])
    initial=rooms()
    def state(room,ty,key=""):
        return http("GET",api+"/rooms/"+quote(room,safe="")+"/state/"+ty+"/"+quote(key,safe=""),headers=admin).json()
    def point(word,area):
        return locate(word,lambda ws:next(((x,y) for w,x,y in ws if w==word and area[0]<x<area[1]),None))
    def tap(word,area=(CHAT_X,1180)):click(point(word,area))
    def submit():
        click(locate("Create button",lambda ws:next(((x,y) for w,x,y in ws if w=="create" and x>CHAT_X
            and any(v=="cancel" and abs(yy-y)<6 for v,_,yy in ws)),None)))
    def menu(label):
        # Space name → dropdown → Invite People / Create Channel.
        click(wait_for("space menu "+label,lambda:ui.space_menu(label,click),15))
    side=(70,CHAT_X)
    def new_id(name,previous):
        added=wait_for("one new "+name,lambda:rooms()-previous)
        assert len(added)==1
        room=added.pop();assert state(room,"m.room.name")=={"name":name}
        return room
    # The green + at the end of the server rail creates a space.
    click(wait_for("Add a Space",ui.add_space,15));point("cancel",(CHAT_X,1180));write("Hearth")
    tap("invite");write(other)
    submit()
    space=new_id("Hearth",initial)
    point("hearth",side)
    assert state(space,"m.room.create")["type"]=="m.space" and state(space,"m.room.join_rules")["join_rule"]=="invite"
    assert state(space,"m.room.member",other)["membership"]=="invite"
    created={space}
    def channel(name,voice):
        menu("create channel")
        point("cancel",(CHAT_X,1180));write(name)
        if voice:tap("voice")
        submit()
        r=new_id(name,initial|created);created.add(r)
        if not voice:
            point("exists",(CHAT_X,1180))
            before_retry=len(paths)
            assert requests.get(api+"/rooms/"+quote(space,safe="")+"/state/m.space.child/"+quote(r,safe=""),
                                headers=admin,timeout=6).status_code==404
            tap("retry")
            wait_for("repaired parent child link",lambda:requests.get(api+"/rooms/"+quote(space,safe="")+
                "/state/m.space.child/"+quote(r,safe=""),headers=admin,timeout=6).ok)
            assert rooms()==initial|created and len(paths)==before_retry
            print("First link write rejected by test gateway. Retry linked the existing room; no second createRoom POST.",flush=True)
        point(name.lower(),side)
        assert state(space,"m.space.child",r)=={"via":["test.invalid"]}
        assert state(r,"m.space.parent",space)=={"canonical":True,"via":["test.invalid"]}
        assert state(r,"m.room.join_rules")=={"join_rule":"restricted","allow":[{"type":"m.room_membership","room_id":space}]}
        assert state(r,"m.room.history_visibility")["history_visibility"]=="joined"
        assert state(r,"m.room.encryption")["algorithm"]=="m.megolm.v1.aes-sha2"
        assert state(r,"m.room.create").get("type")==("org.matrix.msc3417.call" if voice else None)
        if voice:
            levels=state(r,"m.room.power_levels")
            assert all(levels["events"][t]==0 for t in ("org.matrix.msc3401.call.member","org.matrix.msc4143.rtc.slot"))
        return r
    text=channel("Workshop",False);voice=channel("Lounge",True)
    def join(room):
        return peer.evaluate("""async r=>{try{return {id:(await window.mxMatrixClientPeg.get().joinRoom(r,{viaServers:['test.invalid']})).roomId}}
            catch(e){return {status:e.httpStatus,code:e.errcode}}}""",room)
    for r in (text,voice):
        res=join(r)
        assert res=={"status":403,"code":"M_FORBIDDEN"},res
    assert state(space,"m.room.member",other)["membership"]=="invite"
    print("Invite-only Hearth created. Element was invited but not joined: both linked channels denied join with HTTP 403 M_FORBIDDEN.",flush=True)
    # An administrator can also invite by ID *after* creation, rather than
    # needing to recreate the space or link the rooms again.
    peer.evaluate("async r=>await window.mxMatrixClientPeg.get().leave(r)",space)
    wait_for("original invitation declined",lambda:state(space,"m.room.member",other)["membership"]=="leave")
    menu("invite people");point("cancel",(CHAT_X,1180));write(other);tap("send")
    wait_for("new invitation from the space form",lambda:state(space,"m.room.member",other)["membership"]=="invite")
    tap("close")
    assert join(space)=={"id":space}
    for r in (text,voice):
        assert join(r)=={"id":r}
        assert state(r,"m.room.member",other)["membership"]=="join"
        events=http("GET",api+"/rooms/"+quote(r,safe="")+"/messages",params={"dir":"b","limit":80},headers=admin).json()["chunk"]
        assert not any(e["type"]=="m.room.member" and e.get("state_key")==other and e["content"]["membership"]=="invite" for e in events)
    assert state(space,"m.room.member",other)["membership"]=="join"
    # The member remains an ordinary member, not a space moderator.
    denied=requests.put(api+"/rooms/"+quote(space,safe="")+"/state/m.space.child/"+quote(text,safe=""),headers=member,json={},timeout=8)
    assert denied.status_code==403 and denied.json()["errcode"]=="M_FORBIDDEN"
    assert state(space,"m.space.child",text)["via"]==["test.invalid"]
    tap("workshop",side)
    eid=peer.evaluate("""async r=>(await window.mxMatrixClientPeg.get().sendTextMessage(r,'Amber arrival')).event_id""",text)
    wire=http("GET",api+"/rooms/"+quote(text,safe="")+"/event/"+quote(eid,safe=""),headers=admin).json()
    assert wire["type"]=="m.room.encrypted" and wire["sender"]==other and wire["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
    point("amber",(CHAT_X,1180))
    restart()
    # All rooms may now initially select Lounge. Select the sole space icon
    # and verify its server-backed list, not names in a local creation draft.
    def selected(ws):
        if any(w=="hearth" and 70<x<CHAT_X and y<48 for w,x,y in ws):return True
        click((38,155));return False
    locate("reopened Hearth",selected)
    point("lounge",side);tap("workshop",side);point("arrival",(CHAT_X,1180))
    assert rooms()==initial|created and sum(p.endswith("/createRoom") for p in paths)==4,paths
    print(f"Joined space {space}; Element joined text and voice with no channel invites. "
          f"Encrypted send {eid} visible in Workshop after restart; member's unlink was denied. Exactly 3 new rooms.",flush=True)
