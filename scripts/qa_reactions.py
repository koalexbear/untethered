"""Two independently verified clients annotate the base reply, never its replacement."""
import re
from urllib.parse import quote

from qa_native_paste import wait_for
from qa_ui import CHAT_X


def check_reactions(page,room,element_user,get,locate,click,hover,restart,ui):
    parent,reply,edit,decoy=page.evaluate("""async room=>{
        const c=window.mxMatrixClientPeg.get();
        const send=async content=>(await c.sendEvent(room,'m.room.message',content)).event_id;
        const parent=await send({msgtype:'m.text',body:'Orchard note'});
        const rel={'m.in_reply_to':{event_id:parent}};
        const reply=await send({msgtype:'m.text',body:'Pending wording','m.relates_to':rel});
        const edit=await send({msgtype:'m.text',body:'* Juniper update',
            'm.new_content':{msgtype:'m.text',body:'Juniper update'},
            'm.relates_to':{rel_type:'m.replace',event_id:reply}});
        const decoy=await send({msgtype:'m.text',body:'Indigo decoy','m.relates_to':rel});
        return [parent,reply,edit,decoy];
    }""",room)
    key="👍"
    assert len({parent,reply,edit,decoy})==4
    assert all(get("/event/"+quote(eid,safe=""))["type"]=="m.room.encrypted" for eid in (parent,reply,edit,decoy))
    def tile(eid):return page.locator(f'[data-event-id="{eid}"]')
    def pill(eid):return tile(eid).get_by_role("button",name=re.compile(key))
    def element(n,mine):
        tile(reply).get_by_text("Juniper update",exact=True).wait_for()
        def displayed():
            badge=pill(reply)
            return badge.count()==1 and badge.is_visible() and badge.locator('[class*="ButtonCount"]').inner_text()==str(n) and (
                "reactionsRowButtonSelected" in badge.get_attribute("class"))==mine
        wait_for(f"Element count {n} and own={mine} on base reply",displayed)
        assert not any(pill(e).count() for e in (parent,edit,decoy))
    def juniper(ws):
        # The *edited* reply's body row, not another message.
        rows=[(x,y) for w,x,y in ws if w=="juniper" and x>CHAT_X]
        return rows[0] if len(rows)==1 else None
    def chip(ws,count):
        # A reaction pill shows the emoji and its count beneath the message.
        ref=juniper(ws)
        if not ref:return None
        return next(((x,y) for w,x,y in ws if w.strip("()")==str(count) and CHAT_X<x<CHAT_X+300
                     and 5<y-ref[1]<45),None)
    def native(n):return locate(f"Untethered 👍 {n} under Juniper update",lambda ws:chip(ws,n))
    def native_owner(n,mine):
        hover(native(n))
        try:
            verb="remove" if mine else "add"
            def hint(ws):
                return any(w==verb and x>CHAT_X and {"your","reaction"}.issubset(
                    {v for v,xx,yy in ws if xx>x and abs(yy-y)<5}) for w,x,y in ws)
            locate("Untethered tooltip: "+verb+" your reaction",hint)
        finally:hover((65,60))
    def relations(eid):
        return get("/relations/"+quote(eid,safe="")+"/m.annotation/m.reaction","v1")["chunk"]
    def active(n):
        events=relations(reply)
        assert all(e["content"].get("m.relates_to")=={"rel_type":"m.annotation","event_id":reply,"key":key} for e in events)
        assert len(events)==n and len({e["sender"] for e in events})==n,events
        assert not any(relations(eid) for eid in (parent,edit,decoy))
        return {e["sender"]:e["event_id"] for e in events}
    tile(decoy).get_by_text("Indigo decoy",exact=True).wait_for()
    locate("native edit and decoy laid out",lambda ws:any(w=="indigo" for w,_,_ in ws) and any(w=="edited" for w,_,_ in ws))
    row=locate("Juniper update row",juniper)
    click(wait_for("Add Reaction on Juniper update",lambda:ui.message(row,"add reaction"),15))
    # The picker lists quick reactions under "QUICK REACTIONS"; 👍 Like is first.
    label=locate("quick reaction picker",lambda ws:next(((x,y) for w,x,y in ws if w=="quick" and x>CHAT_X),None))
    like=wait_for("Like in reply's reaction picker",lambda:ui.pick([(label[0]-2,label[1]+34)],"like","above"),10)
    click(like)
    native(1);element(1,False)
    single=active(1)
    assert element_user not in single
    pill(reply).click()
    native_owner(2,True);element(2,True)
    both=active(2)
    assert element_user in both and single.items()<=both.items()
    print(f"Both clients show 👍 2 on edited reply {reply}; Synapse found two distinct senders, "
          "zero annotations on its edit, parent or sibling reply.",flush=True)
    restart()
    native_owner(2,True);element(2,True)
    click(native(2))  # Infer which event to redact from the restored Matrix device.
    native_owner(1,False);element(1,True)
    remaining=active(1)
    assert remaining=={element_user:both[element_user]}
    sender,reaction=next(iter(single.items()))
    redacted=get("/event/"+quote(reaction,safe=""))
    because=redacted["unsigned"]["redacted_because"]
    assert because["sender"]==sender and (because["content"].get("redacts") or because.get("redacts"))==reaction
    print(f"Untethered removed its own annotation {reaction}; Element's {both[element_user]} remained.",flush=True)
    restart()
    native_owner(1,False);element(1,True)
    assert active(1)==remaining
    print(f"After reconnect: Untethered 👍 1 ('Add your reaction'), Element 👍 1 (selected), original reply ID {reply}.",flush=True)
