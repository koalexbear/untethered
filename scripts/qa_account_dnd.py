"""Use two distinct receivers, their real desktop notification servers and a separate sender."""
import json
import sqlite3
import subprocess as sp
from contextlib import ExitStack, closing
from pathlib import Path
from urllib.parse import quote

from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_replies import screen_words
from qa_room_alerts import NotificationDesktop


def verify_dnd(stack,tmp,hs,backend,hdr,http,me,room,page,peer,words,click_word,send,hide,browser_desk):
    root=Path(__file__).resolve().parents[1]
    folder=tmp/"desktop";folder.mkdir()
    env=isolated_env(folder,backend,"x11")
    env["UNTETHERED_PROFILE"]=str(tmp/"seed")
    # Bare Xvfb has no window manager to restack an existing notification
    # window above a newly launched GUI. Keep the popup area unobstructed.
    start_display(stack,folder,env,"x11",1820,980)
    desk=NotificationDesktop(stack,folder,env)
    life=stack.enter_context(ExitStack())
    def launch():
        proc=life.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
        def visible():
            assert proc.poll() is None
            return sp.run(["xdotool","search","--all","--onlyvisible","--pid",str(proc.pid),"--name","Untethered.*media-proof"],
                          text=True,env=env,capture_output=True).stdout.strip()
        win=wait_for("native receiver in encrypted room",visible,35)
        desk.run("xdotool","windowmove",win,"0","0","windowsize",win,"1180","760");desk.focus(win)
        return win
    win=launch()
    def native_words():return screen_words(sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6))
    def locate(label,read=words,*,sidebar=False):
        def find():
            return next(((x,y) for w,x,y in read() if w==label and (70<x<280 if sidebar else x>280)),None)
        try:return wait_for("visible "+label,find,15)
        except RuntimeError:print("Visible receiver labels:",[w for w,_,_ in read()],flush=True);raise
    def click(x,y):desk.run("xdotool","mousemove","--window",win,str(x),str(y),"click","1")
    def rules():return http("GET",hs+"/_matrix/client/v3/pushrules/",headers=hdr).json()
    def split(doc):
        from copy import deepcopy
        doc=deepcopy(doc)
        [master]=[r for r in doc["global"]["override"] if r["rule_id"]==".m.rule.master"]
        return master.pop("enabled"),doc
    before,others=split(rules())
    assert before is False
    def confirmed(on):
        wait_for("only master-rule enabled flag changed",lambda:split(rules())==(on,others))
    second=peer.evaluate("""async me=>(await window.mxMatrixClientPeg.get().createRoom({name:"zz-control",preset:"private_chat",
        invite:[me],initial_state:[{type:"m.room.encryption",state_key:"",content:{algorithm:"m.megolm.v1.aes-sha2"}}]})).room_id""",me)
    http("POST",hs+"/_matrix/client/v3/rooms/"+quote(second,safe="")+"/join",headers=hdr,json={})
    peer.wait_for_function("([r,me])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(me)?.membership==='join'",arg=[second,me])
    def deliver(body,target,notify,r=room):
        assert page.evaluate("document.hidden && !document.hasFocus()") and desk.run("xdotool","getwindowfocus")==desk.other
        eid=send(body,target,True,notify,r)
        with closing(sqlite3.connect((tmp/"seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True)) as db:
            def check():
                row=db.execute("SELECT decrypted,unread_type FROM event WHERE event_id=? AND room_id=? AND type='m.room.encrypted'",(eid,r)).fetchone()
                if row and row[0]:
                    c=json.loads(row[0])
                    assert (c["body"],c["m.mentions"].get("user_ids",[]),bool(row[1]&2))==(body,[target] if target else [],notify)
                    return True
            wait_for("desktop decrypted "+body,check,20)
        if notify:
            for d,app in ((desk,"Untethered"),(browser_desk,"Chromium")):
                try:d.popup(body,appname=app,on_visible=(lambda:still_hidden(page)) if app=="Chromium" else None)
                except RuntimeError:
                    print(app,"expected",body,"bus count",d.count(),"displayed",d.run(d.ctl,"count","displayed"),
                          "waiting",d.run(d.ctl,"count","waiting"),"logged body",body in d.log.read_text(),flush=True)
                    raise
    def counts(n):
        for d in (desk,browser_desk):
            assert d.count()==n and int(d.run(d.ctl,"count","history"))==n
    desk.focus(desk.other)
    deliver("First lantern",me,True)
    counts(1)
    desk.focus(win);click(150,728)
    click(*locate("pause",native_words))
    confirmed(True);locate("resume",native_words)
    page.bring_to_front();page.mouse.click(150,728);locate("resume")
    desk.focus(desk.other);hide()
    deliver("Silent sapphire",me,False);deliver("Quiet garden",None,False,second)
    counts(1)
    # Restart the receiver process/engine; reload the other receiver's worker.
    life.close();win=launch();locate("disturb",native_words,sidebar=True)
    page.bring_to_front();page.reload()
    page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
    page.mouse.click(150,728);locate("resume")
    hide();desk.focus(desk.other)
    deliver("Muted cobalt",me,False)
    counts(1)
    # DND suppresses delivery, not the independently indexed direct mentions.
    desk.focus(win);click(*locate("mentions",native_words,sidebar=True))
    locate("sapphire",native_words);locate("cobalt",native_words)
    page.bring_to_front();click_word("resume");locate("pause");confirmed(False)
    click(150,728);locate("pause",native_words)
    desk.focus(desk.other);hide()
    deliver("Restored beacon",me,True);deliver("Ordinary meadow",None,True,second)
    deliver("Muted room baseline",None,False)
    counts(3)
    for d in (desk,browser_desk):
        assert all(s not in d.log.read_text() for s in ("Silent sapphire","Quiet garden","Muted cobalt","Muted room baseline"))
    print("DND enabled on Linux, persisted on both restarted receivers, disabled from Chromium. "
          "Both decrypted all controls; silenced mentions stayed in the inbox. "
          "Each notification server received exactly 3 popups: baseline, restored mention and restored ordinary message. "
          "All Matrix push rules including the room mute restored exactly.",flush=True)


def still_hidden(page):
    assert page.evaluate("document.hidden && !document.hasFocus()")
