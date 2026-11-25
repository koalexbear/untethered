"""Real read-marker and marked-unread account data; one recipient, two devices."""
import sqlite3
import subprocess as sp
import time
from contextlib import ExitStack,closing
from pathlib import Path
from urllib.parse import quote

import requests
from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_replies import screen_words
from qa_ui import CHAT_X


def check_read_state(stack,tmp,hs,backend,room,me,hdr,http,web,peer,web_words):
    root=Path(__file__).resolve().parents[1]
    folder=tmp/"reader";folder.mkdir()
    env=isolated_env(folder,backend,"x11");env["UNTETHERED_PROFILE"]=str(tmp/"seed")
    start_display(stack,folder,env,"x11")
    stack.enter_context(child(["xmessage","-title","Focus control","-geometry","150x70+10+800","Other task"],env))
    def x(*args):return sp.check_output(["xdotool",*map(str,args)],env=env,text=True,timeout=6).strip()
    other_window=wait_for("focus control",lambda:sp.run(["xdotool","search","--name","^Focus control$"],
                                                     env=env,text=True,capture_output=True).stdout.strip())
    life=stack.enter_context(ExitStack())
    def launch():
        p=life.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
        def ready():
            assert p.poll() is None
            return sp.run(["xdotool","search","--all","--pid",str(p.pid),"--onlyvisible","--name","Untethered.*media-proof"],
                          env=env,text=True,capture_output=True).stdout.strip()
        win=wait_for("verified native room",ready,35)
        x("windowmove",win,0,0,"windowsize",win,1180,760)
        return win
    win=launch();x("windowfocus","--sync",other_window)
    peer_id=peer.evaluate("window.mxMatrixClientPeg.get().getSafeUserId()")
    peer.evaluate("async ()=>await window.mxMatrixClientPeg.get().setDisplayName('Cedar')")
    def native_words():return screen_words(sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6))
    def native_click(pos):x("mousemove","--window",win,*pos,"click",1)
    def find(label,check,read=native_words):
        try:return wait_for(label,lambda:check(read()),15)
        except RuntimeError:print(label,"visible words:",read(),flush=True);raise
    def word(ws,s):return next(((xx,y) for w,xx,y in ws if w==s and xx>CHAT_X),None)
    def new_tag(ws):
        # The unread divider is a red line with a NEW tag at its right end.
        return next(((xx,y) for w,xx,y in ws if w=="new" and xx>1000),None)
    def phrase(ws,first,second):
        return next(((xx,y) for w,xx,y in ws if w==first and xx>CHAT_X for v,x2,y2 in ws
                     if v==second and 0<x2-xx<100 and abs(y2-y)<4),None)
    def marker(r,key):
        uri=f"{hs}/_matrix/client/v3/user/{quote(me,safe='')}/rooms/{quote(r,safe='')}/account_data/{key}"
        res=requests.get(uri,headers=hdr,timeout=4)
        if res.status_code==404:return None
        res.raise_for_status();return res.json()
    def receipts(r):
        with closing(sqlite3.connect((tmp/"homeserver.db").as_uri()+"?mode=ro",uri=True)) as db:
            return dict(db.execute("SELECT receipt_type,event_id FROM receipts_linearized WHERE room_id=? AND user_id=? "
                                   "AND thread_id IS NULL",(r,me)))
    def exact(eid):
        wait_for("Synapse private and fully-read markers at "+eid,lambda:
            receipts(room).get("m.read.private")==eid and marker(room,"m.fully_read")=={"event_id":eid})
    def send(r,body,target=False):
        eid=peer.evaluate("""async ([r,body,u])=>(await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
            msgtype:'m.text',body,'m.mentions':u?{user_ids:[u]}:{}})).event_id""",[r,body,me if target else None])
        e=http("GET",hs+"/_matrix/client/v3/rooms/"+quote(r,safe="")+"/event/"+quote(eid,safe=""),headers=hdr).json()
        assert e["type"]=="m.room.encrypted" and e["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
        web.wait_for_function("([id,b])=>qaSeen[id]?.body===b",arg=[eid,body],polling=150,timeout=30000)
        return eid
    def hold(expected):
        # Check continuously across multiple 750 ms reading checks.
        end=time.monotonic()+2
        while time.monotonic()<end:
            assert receipts(room).get("m.read.private")==expected
            web.wait_for_timeout(125)
    other=peer.evaluate("""async u=>(await window.mxMatrixClientPeg.get().createRoom({name:'Orchard',preset:'private_chat',
        invite:[u],initial_state:[{type:'m.room.encryption',state_key:'',content:{algorithm:'m.megolm.v1.aes-sha2'}}]})).room_id""",me)
    http("POST",hs+"/_matrix/client/v3/rooms/"+quote(other,safe="")+"/join",headers=hdr,json={})
    peer.wait_for_function("([r,u])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(u)?.membership==='join'",arg=[other,me])
    initial=receipts(room)
    private=initial.get("m.read.private")
    peer.evaluate("""async r=>{const c=window.mxMatrixClientPeg.get();for(let i=0;i<55;i++)await c.sendTextMessage(r,'Ledger '+i)}""",room)
    first=send(room,"Sunrise marker",True);orchard=send(other,"Orchard control",True)
    find("decrypted latest text",lambda ws:word(ws,"sunrise"))
    assert web.evaluate("document.hidden && !document.hasFocus()") and x("getwindowfocus")==other_window
    hold(private)
    # Move away from the bottom before refocusing the native window.
    x("mousemove","--window",win,690,375,"click","--repeat",8,4)
    find("scrolled ledger, newest not visible",lambda ws:word(ws,"ledger") and not word(ws,"sunrise"))
    x("windowfocus","--sync",win);hold(private)
    x("mousemove","--window",win,690,375,"click","--repeat",23,5)
    find("visible native tail",lambda ws:word(ws,"sunrise"))
    exact(first)
    print("Unfocused native, hidden tab and focused-but-scrolled-up timeline sent no receipt. "
          f"Reading the native tail marked {first} privately on Synapse.",flush=True)
    x("windowfocus","--sync",other_window)
    # Put the web receiver in search *before* the new event exists. Time
    # spent locating/clicking Search is real reading time on a visible tab.
    web.bring_to_front()
    web.mouse.click(*find("web search button",lambda ws:word(ws,"search"),web_words))
    find("web search active",lambda ws:word(ws,"coverage"),web_words)
    second=send(room,"Amber unread",True)
    def divider(ws):
        d=new_tag(ws);a=phrase(ws,"amber","unread");s=phrase(ws,"sunrise","marker")
        return d and a and s and s[1]<d[1]<a[1]
    find("unread boundary between exact old and new messages",divider)
    hold(first)
    web.mouse.click(*find("return from search",lambda ws:word(ws,"return"),web_words))
    exact(second)
    find("both devices saw web receipt",lambda ws:phrase(ws,"mark","unread"))
    # Set a reminder on web; even an actively focused receiver must retain it.
    web.mouse.click(*find("web Mark unread",lambda ws:phrase(ws,"mark","unread"),web_words))
    wait_for("Synapse manual flag",lambda:marker(room,"m.marked_unread")=={"unread":True})
    find("native saw manual unread",lambda ws:phrase(ws,"mark","read"))
    life.close();win=launch()
    x("windowfocus","--sync",win);web.reload();web.wait_for_function("document.title.includes('media-proof')",timeout=45000)
    third=send(room,"Violet reminder")
    for read in (native_words,web_words):find("reminder survived restart",lambda ws:phrase(ws,"mark","read") and word(ws,"violet"),read)
    hold(second)
    web.mouse.click(*find("explicitly mark read",lambda ws:phrase(ws,"mark","read"),web_words))
    exact(third);wait_for("manual flag cleared",lambda:marker(room,"m.marked_unread")=={"unread":False})
    for read in (native_words,web_words):
        find("clean room on both clients",lambda ws:phrase(ws,"mark","unread"),read)
        find("other room highlight badge retained",lambda ws:any(w=="orchard" and xx<CHAT_X and any(
            x2<CHAT_X and x2>xx and abs(y-y2)<6 and "1" in v for v,x2,y2 in ws) for w,xx,y in ws),read)
    assert receipts(other).get("m.read.private")!=orchard and marker(other,"m.fully_read")!={"event_id":orchard}
    assert receipts(room).get("m.read")==initial.get("m.read")
    visible=peer.evaluate("""([r,u])=>window.mxMatrixClientPeg.get().getRoom(r).getEventReadUpTo(u)""",[room,me])
    assert visible not in (first,second,third)
    print(f"Web auto-read {second}. Manual unread persisted on Linux + web across restart; explicit web read "
          f"advanced both devices to {third}. Orchard retained its highlight; no public receipt reached Element.",flush=True)
    web.mouse.click(150,728)
    web.mouse.click(*find("opt into public receipts",lambda ws:phrase(ws,"share","receipts"),web_words))
    find("public sharing enabled",lambda ws:phrase(ws,"keep","receipts"),web_words)
    def pref(value):
        with closing(sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True)) as db:
            wait_for("native synced read receipt preference",lambda:db.execute("SELECT json_extract(content,'$.public') "
                "FROM account_data WHERE type='com.untethered.read_receipts.v1' AND user_id=?",(me,)).fetchone()==(int(value),))
    pref(True)
    shared=send(room,"Public receipt")
    wait_for("Synapse received opted-in public receipt",lambda:receipts(room).get("m.read")==shared)
    peer.wait_for_function("""([r,u,e])=>window.mxMatrixClientPeg.get().getRoom(r).getEventReadUpTo(u)===e""",
                           arg=[room,me,shared],timeout=15000)
    web.mouse.click(*find("return to private receipts",lambda ws:phrase(ws,"keep","receipts"),web_words))
    pref(False)
    again=send(room,"Private again");exact(again)
    assert receipts(room)["m.read"]==shared and peer.evaluate(
        "([r,u])=>window.mxMatrixClientPeg.get().getRoom(r).getEventReadUpTo(u)",[room,me])==shared
    x("windowfocus","--sync",other_window)
    base=send(room,"Earlier draft")
    edit=peer.evaluate("""async ([r,id])=>(await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
      msgtype:'m.text',body:'* Revised honey','m.new_content':{msgtype:'m.text',body:'Revised honey'},
      'm.relates_to':{rel_type:'m.replace',event_id:id}})).event_id""",[room,base])
    find("the newest message has an applied edit",lambda ws:phrase(ws,"revised","honey") and word(ws,"edited"))
    hold(again)
    x("windowfocus","--sync",win);exact(edit)
    x("windowfocus","--sync",other_window)
    last=send(room,"After edit")
    def after_edit(ws):
        a=phrase(ws,"revised","honey");d=new_tag(ws);m=phrase(ws,"after","edit")
        return a and d and m and a[1]<d[1]<m[1]
    find("divider follows a receipt on an aggregated edit",after_edit)
    assert receipts(room).get("m.read")==shared
    x("windowfocus","--sync",win);exact(last)
    # A public receipt from the independent Element reader must be rendered
    # on the exact native-authored message. No further peer posts are sent
    # that might produce an implicit receipt for their own messages.
    web.reload();web.wait_for_function("document.title.includes('media-proof')",timeout=45000)
    native_click((535,696));x("type","--clearmodifiers","Reader checkpoint");x("key","Return")
    def delivered():
        return peer.evaluate("""r=>window.mxMatrixClientPeg.get().getRoom(r).getLiveTimeline().getEvents()
            .find(e=>e.getContent().body==='Reader checkpoint' && e.getId().startsWith('$'))?.getId()""",room)
    posted=wait_for("Element decrypted native message",delivered)
    peer.evaluate("""async ([r,id])=>{const c=window.mxMatrixClientPeg.get();await c.sendReadReceipt(c.getRoom(r)
        .findEventById(id),'m.read',true)}""",[room,posted])
    with closing(sqlite3.connect((tmp/"homeserver.db").as_uri()+"?mode=ro",uri=True)) as db:
        wait_for("real public receipt from Element",lambda:db.execute("SELECT event_id FROM receipts_linearized "
            "WHERE room_id=? AND user_id=? AND receipt_type='m.read' AND thread_id IS NULL",(room,peer_id)).fetchone()==(posted,))
    for read in (native_words,web_words):
        def seen(ws):
            at=phrase(ws,"reader","checkpoint");label=phrase(ws,"seen","by")
            return at and label and 0<label[1]-at[1]<40 and any(w=="cedar" and abs(yy-label[1])<5 for w,_,yy in ws)
        find("Element receipt on Reader checkpoint",seen,read)
    print(f"Opt-in reached Element at {shared}; switching back to private left the public receipt there. "
          f"Private marker advanced to edit {edit} with the divider before the next post. "
          f"Both Untethered devices rendered Element’s Seen by on {posted}.",flush=True)
