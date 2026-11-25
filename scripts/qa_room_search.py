"""A real cache, cold UI, cross-room decoy and pixel-level reading bookmark."""
import re
import sqlite3
from urllib.parse import quote
from qa_native_paste import wait_for


def check_room_search(tmp,hs,page,room,native,hdr,http,messages,restart,words,locate,click,x,paths):
    native_id="@"+native["username"]+":test.invalid"
    other=page.evaluate("""async uid=>(await window.mxMatrixClientPeg.get().createRoom({
        name:"zz-search-control",preset:"private_chat",invite:[uid],initial_state:[{type:"m.room.encryption",
        state_key:"",content:{algorithm:"m.megolm.v1.aes-sha2"}}]})).room_id""",native_id)
    http("POST",hs+"/_matrix/client/v3/rooms/"+quote(other,safe="")+"/join",headers=hdr,json={})
    page.wait_for_function("([r,u])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(u)?.membership==='join'",arg=[other,native_id])
    parent,reply,edit,neighbor,decoy=page.evaluate("""async ([r,other])=>{
        const c=window.mxMatrixClientPeg.get(), send=async (room,body,extra={})=>
          (await c.sendEvent(room,'m.room.message',{msgtype:'m.text',body,...extra})).event_id;
        const p=await send(r,'Survey briefing');
        const m=await send(r,'Retired glacier path',{'m.relates_to':{'m.in_reply_to':{event_id:p}}});
        const e=await send(r,'* Saffron comet launch',{'m.relates_to':{rel_type:'m.replace',event_id:m},
            'm.new_content':{msgtype:'m.text',body:'Saffron comet launch'}});
        return [p,m,e,await send(r,'Context beacon'),await send(other,'Saffron comet diversion')];
    }""",[room,other])
    gap=messages([f"Ledger sequence {i:03}" for i in range(150)])
    # Both rooms must actually be cached and decrypted on the searching device.
    with sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True) as db:
        def cached(eid,r):
            return db.execute("select json_extract(decrypted,'$.body') from event where event_id=? and room_id=? and type='m.room.encrypted'",(eid,r)).fetchone()
        wait_for("both encrypted rooms indexed on this device",lambda:cached(edit,room)==("* Saffron comet launch",)
                 and cached(decoy,other)==("Saffron comet diversion",) and cached(gap[-1],room)==("Ledger sequence 149",),35)
    restart()
    def point(ws,w):return next(((xx,y) for word,xx,y in ws if word==w and xx>280),None)
    def markers(ws):
        return [(int(num),y) for word,xx,y in ws if word=="ledger" and xx>280 and 68<y<630
                for num,xxx,yy in ws if re.fullmatch(r"\d{3}",num) and xx<xxx<xx+200 and abs(yy-y)<4]
    locate("cold recent page",lambda ws:point(ws,"149"))
    x("mousemove","--window","WIN","690","400","click","--repeat","2","4")
    before=locate("scrolled-away ledger rows",lambda ws: m if len(m:=markers(ws))>=6 and not point(ws,"149") else None)
    anchor,y=before[len(before)//2]
    def tap(w):click(locate(w,lambda ws:point(ws,w)))
    tap("search")
    tap("coverage")
    locate("coverage counts only readable messages from this room",lambda ws:point(ws,"153") and point(ws,"cached"))
    def submit(q):
        x("key","ctrl+a");x("type","--clearmodifiers",q);x("key","Return")
    paths.clear()
    submit("Retired glacier")
    locate("superseded body yields no match",lambda ws:point(ws,"no") and point(ws,"matches"))
    submit("saffron comet")
    def result(ws):
        # Unique body in the search results, not text in the query field.
        if point(ws,"diversion"):raise AssertionError("result escaped its room")
        return point(ws,"launch") if point(ws,"coverage") and point(ws,"match") and not point(ws,"matches") else None
    hit=locate("one edited result in the selected room",result)
    assert not any("/context/" in p or "/event/" in p or "/search" in p for p in paths),paths
    print("Local coverage: 153 readable messages in the room. Both encrypted rooms decrypted; 150 newer messages; retired wording absent; "
          "one current-room result for 'saffron comet', no homeserver search or event fetch.",flush=True)
    click(hit)
    locate("exact edited reply highlighted with context",lambda ws:point(ws,"launch") and point(ws,"edited")
           and point(ws,"beacon") and point(ws,"briefing") and point(ws,"match"))
    assert [p for p in paths if "/context/" in p]==[f"/_matrix/client/v3/rooms/{room}/context/{reply}"],paths
    print(f"Opened base reply {reply} (not edit {edit}); screen showed 'Saffron comet launch' and 'Context beacon'.",flush=True)
    tap("results")
    click(locate("retained search results",result))
    locate("reopened result in context",lambda ws:point(ws,"launch") and point(ws,"match") and point(ws,"beacon"))
    new=messages(["While away one","While away two","While away three"])
    # The bookmark may be the clipped row above the first OCR-readable one.
    # Grow an offscreen row, not the bookmark itself.
    above=len(gap)-45
    assert len(gap)-50<=above<anchor,(above,anchor,before)
    assert above+3<min(n for n,_ in before)
    change=page.evaluate("""async ([r,target])=>{
        const body='Expanded landmark\\nLine two\\nLine three\\nLine four\\nLine five';
        return (await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
            msgtype:'m.text',body:'* '+body,'m.new_content':{msgtype:'m.text',body},
            'm.relates_to':{rel_type:'m.replace',event_id:target}})).event_id
    }""",[room,gap[above]])
    with sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True) as db:
        wait_for("incoming append and above-bookmark edit processed",lambda:db.execute(
            "select count(*) from event where event_id in (?,?) and decrypted is not null",(new[-1],change)).fetchone()[0]==2)
    tap("timeline")
    def restored(ws):
        rows=markers(ws)
        return next(((n,yy) for n,yy in rows if n==anchor and abs(yy-y)<=1),None)
    try:_,yy=locate("same scrolled message at original vertical position",restored)
    except RuntimeError:
        print("Restored viewport:",markers(words()),flush=True);raise
    print(f"Reading position restored: Ledger {anchor:03} at y={y} before search, y={yy} after return, "
          f"despite three new messages and Ledger {above:03} expanding to five lines above it.",flush=True)
