"""Check the rendered inbox before/after restart and each actual /context target."""
from urllib.parse import quote

from qa_ui import CHAT_X


def check_inbox(hs,peer,one,hdr,http,restart,locate,click,paths):
    me=http("GET",hs+"/_matrix/client/v3/account/whoami",headers=hdr).json()["user_id"]
    them=peer.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()")
    assert me!=them
    two=peer.evaluate("""async u=>(await window.mxMatrixClientPeg.get().createRoom({
        name:'Orchard',preset:'private_chat',invite:[u],initial_state:[{type:'m.room.encryption',state_key:'',
        content:{algorithm:'m.megolm.v1.aes-sha2'}}]})).room_id""",me)
    def api(r):return hs+"/_matrix/client/v3/rooms/"+quote(r,safe="")
    http("POST",api(two)+"/join",headers=hdr,json={})
    peer.wait_for_function("([r,u])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(u)?.membership==='join'",arg=[two,me])

    def points(ws,w,left=False):
        return [(xx,y) for word,xx,y in ws if word==w and ((70<xx<CHAT_X) if left else xx>CHAT_X)]
    def tap_inbox():click(locate("global Mentions sidebar",lambda ws:next(iter(points(ws,"mentions",True)),None)))
    tap_inbox()
    locate("empty mentions surface",lambda ws:points(ws,"inbox") and points(ws,"no"))
    ids=peer.evaluate("""async ([a,b,me,other])=>{
      const c=window.mxMatrixClientPeg.get(), sent=[];
      const send=async(r,body,mentions={},extra={})=>{
        const id=(await c.sendEvent(r,'m.room.message',{msgtype:'m.text',body,'m.mentions':mentions,...extra})).event_id;
        sent.push([r,id]);return id;
      };
      const direct={user_ids:[me]};
      const edit=(r,base,t,m)=>send(r,'* '+t,m,{'m.new_content':{msgtype:'m.text',body:t,'m.mentions':m},
        'm.relates_to':{rel_type:'m.replace',event_id:base}});
      const hitA=await send(a,'Beacon notice',direct);
      await send(a,'Amber marker');await send(a,'Literal '+me);await send(a,'Wrong recipient',{user_ids:[other]});
      const hitB=await send(b,'Beacon notice',direct);
      await send(b,'Indigo marker');await send(b,'Everyone listening',{room:true});
      const base=await send(b,'Previous wording');await send(b,'Jade marker');
      const replacement=await edit(b,base,'Violet revision',direct);
      const gone=await send(a,'Removed address',direct);await edit(a,gone,'Withdrawn address',{});
      return {sent, hitA,hitB,base,replacement};
    }""",[one,two,me,them])
    assert len(ids["sent"])==len({id for _,id in ids["sent"]})==12
    for room,id in ids["sent"]:
        e=http("GET",api(room)+"/event/"+quote(id,safe=""),headers=hdr).json()
        assert e["sender"]==them and e["type"]=="m.room.encrypted" and e["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
    def entries(ws):
        if not (points(ws,"inbox") and points(ws,"3") and len(points(ws,"beacon"))==2 and len(points(ws,"violet"))==1):
            return None
        if any(points(ws,w) for w in ("literal","wrong","everyone","withdrawn","previous","removed","marker","ledger")):return None
        hits={}
        for r,byline in [(one,"media-proof"),(two,"orchard")]:
            for _,y in points(ws,byline):
                for word in ("beacon","violet"):
                    for xx,yy in points(ws,word):
                        if 5<yy-y<33:hits[(r,word)]=(xx,yy)
        return hits if set(hits)=={(one,"beacon"),(two,"beacon"),(two,"violet")} else None
    locate("live, exactly three cross-room mentions",entries)
    print("Live inbox: two same-text direct mentions and one edit-added mention; ordinary, literal ID, "
          "other-recipient, room-wide and edit-removed mentions excluded.",flush=True)
    # On a new UI, none of the targets should be in either initial 50-row page.
    # All 130 filler events match the coarse local query (they carry
    # user_ids), but are addressed to the sender. The cold inbox must page
    # past them, not stop at the first 128 candidates or list their bodies.
    last=peer.evaluate("""async rooms=>{
        const c=window.mxMatrixClientPeg.get();return await Promise.all(rooms.map(async r=>{
            let id;for(let i=0;i<65;i++)id=(await c.sendEvent(r,'m.room.message',{msgtype:'m.text',
                body:'Ledger '+i,'m.mentions':{user_ids:[c.getSafeUserId()]}})).event_id;return id;
        }))
    }""",[one,two])
    assert all(id.startswith("$") for id in last) and len(set(last))==2
    # Cold proof must not race the pre-restart receiver's acquisition of
    # either room. Verify the 65 newer events on each server timeline, too.
    for room,final in zip((one,two),last):
        chunk=http("GET",api(room)+"/messages",headers=hdr,params={"dir":"b","limit":65}).json()["chunk"]
        assert len(chunk)==len({e["event_id"] for e in chunk})==65 and chunk[0]["event_id"]==final
        assert all(e["type"]=="m.room.encrypted" and e["content"]["algorithm"]=="m.megolm.v1.aes-sha2" for e in chunk)
    for channel in ("media-proof","orchard"):
        # Coverage counts locally readable base messages, not server events:
        # five original messages plus sixty-five filler per room.
        click(locate("channel "+channel,lambda ws:next(iter(points(ws,channel,True)),None)))
        click(locate("room search",lambda ws:next(iter(points(ws,"search")),None)))
        click(locate("local coverage",lambda ws:next(iter(points(ws,"coverage")),None)))
        locate("70 messages decrypted in "+channel,lambda ws:points(ws,"70") and points(ws,"cached"))
    print("Before restart: 70 readable messages on this device in each room; 130 newer non-recipient messages.",flush=True)
    restart()
    paths.clear();tap_inbox();locate("persisted inbox after restart",entries)
    assert not paths,paths
    for room,w,neighbor,target in [(one,"beacon","amber",ids["hitA"]),(two,"beacon","indigo",ids["hitB"]),
                                   (two,"violet","jade",ids["base"])]:
        hits=locate("inbox before target selection",entries)
        paths.clear();click(hits[(room,w)])
        locate("highlighted target and correct neighbor",lambda ws:points(ws,"older") and points(ws,"original")
               and points(ws,w) and points(ws,neighbor))
        assert paths==[f"/_matrix/client/v3/rooms/{room}/context/{target}"],paths
        print(f"Cold inbox selected {room} → {target}; original and {neighbor.title()} marker visible.",flush=True)
        tap_inbox()
    locate("return shows exactly those same three mentions",entries)
    http("POST",api(two)+"/leave",headers=hdr,json={})
    locate("open inbox drops both entries from the departed room",lambda ws:points(ws,"inbox") and points(ws,"1")
           and len(points(ws,"beacon"))==1 and not points(ws,"violet") and not points(ws,"orchard"))
    print("Left Orchard while viewing the inbox: 3 entries became 1; only the other room remained.",flush=True)
