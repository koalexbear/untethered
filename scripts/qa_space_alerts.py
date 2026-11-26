"""Real space hierarchy, encrypted child rooms, Matrix rules and visible notifications."""
import base64
import subprocess as sp
from pathlib import Path
from urllib.parse import quote

import qa_events
from qa_events import use_profile
from qa_ui import CHAT_X
from qa_native_paste import child,isolated_env,start_display,wait_for

PREFIX="com.untethered.space_alerts.v1"


def native_sender(tmp,hs,page,hdr,http):
    def send(room,body,target,notify):
        eid=page.evaluate("""async ([r,b,u])=>(await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
            body:b,msgtype:'m.text','m.mentions':u?{user_ids:[u]}:{}})).event_id""",[room,body,target])
        wire=http("GET",hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")+"/event/"+quote(eid,safe=""),headers=hdr).json()
        assert wire["type"]=="m.room.encrypted" and wire["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
        e=wait_for("native decrypted "+body,lambda:qa_events.event(tmp/"seed",room,eid))
        c=e["content"]
        assert e["encrypted"] and (c["body"],c["m.mentions"].get("user_ids",[]),e["notify"])==(body,[target] if target else [],notify),e
        return eid
    return send


def companion(stack,tmp,backend):
    """A separately signed-in receiver sharing this web account's push rules."""
    from qa_room_alerts import NotificationDesktop
    d=tmp/"companion";d.mkdir()
    env=use_profile(isolated_env(d,backend,"x11"),tmp/"seed")
    start_display(stack,d,env,"x11",1820,980)
    desk=NotificationDesktop(stack,d,env)
    p=stack.enter_context(child([Path(__file__).resolve().parents[1]/"rust-client/target/debug/untethered-iced"],env))
    def ready():
        assert p.poll() is None
        return sp.run(["xdotool","search","--all","--onlyvisible","--pid",str(p.pid),"--name","Untethered.*media-proof"],
                      env=env,text=True,capture_output=True).stdout.strip()
    win=wait_for("companion Linux receiver",ready,35)
    desk.run("xdotool","windowmove",win,"0","0","windowsize",win,"1180","760")
    desk.focus(desk.other)
    def decoded(room,eid,body,notify):
        def evaluated():
            e=qa_events.event(tmp/"seed",room,eid)
            return e and e["encrypted"] and (e["content"].get("body"),e["notify"])
        assert wait_for("companion decoded and evaluated "+body,evaluated)==(body,notify)
    return desk,decoded


def check_spaces(hs,peer,me,hdr,http,room,words,click,focus,hide,restart,send,desk,app,second_device=None,*,ui):
    api=hs+"/_matrix/client/v3"
    def create(name,space=False):
        r=peer.evaluate("""async ([name,u,space])=>(await window.mxMatrixClientPeg.get().createRoom({
            name,invite:[u],preset:'private_chat',creation_content:space?{type:'m.space'}:{},
            initial_state:space?[]:[{type:'m.room.encryption',state_key:'',content:{algorithm:'m.megolm.v1.aes-sha2'}}]})).room_id""",[name,me,space])
        http("POST",api+"/rooms/"+quote(r,safe="")+"/join",headers=hdr,json={})
        peer.wait_for_function("([r,u])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMember(u)?.membership==='join'",arg=[r,me])
        return r
    def link(parent,kid,on):
        peer.evaluate("""async ([p,k,on])=>await window.mxMatrixClientPeg.get().sendStateEvent(p,'m.space.child',
            on?{via:['test.invalid']}:{},k)""",[parent,kid,on])
    parent,nested=create("Garden",True),create("Beds",True)
    child,control=create("Orchard"),create("Outside")
    link(nested,child,True);link(parent,nested,True);link(parent,room,True)
    def rules():return http("GET",api+"/pushrules/",headers=hdr).json()["global"]
    # An unrelated user-defined underride (not just server defaults) must
    # survive creation, updates and cleanup of the generated rules.
    http("PUT",api+"/pushrules/global/underride/qa.keep",headers=hdr,json={
        "conditions":[{"kind":"event_property_is","key":"type","value":"qa.never"}],"actions":[]})
    baseline=rules()
    def rid(r):return PREFIX+"."+base64.urlsafe_b64encode(r.encode()).decode().rstrip("=")
    def settled(mapping):
        def matches():
            actual={r["rule_id"]:r for r in rules()["underride"] if r["rule_id"].startswith(PREFIX+".")}
            return set(actual)=={rid(k) for k in mapping} and all(actual[rid(k)]["enabled"] and actual[rid(k)]["actions"]==v and
                 actual[rid(k)]["conditions"]==[{"kind":"event_property_is","key":"room_id","value":k}] for k,v in mapping.items())
        try:wait_for("derived Matrix underrides",matches,18)
        except RuntimeError:
            print("Underrides:",rules()["underride"][:4],"UI:",words(),flush=True);raise
    def pos(label,bounds,ws):
        return next(((x,y) for w,x,y in ws if w.lower().strip(".,:|")==label and bounds[0]<x<bounds[1]),None)
    def find(label,bounds=(70,CHAT_X)):
        try:return wait_for("UI "+label,lambda:pos(label,bounds,words()),12)
        except RuntimeError:print("Visible UI:",words(),flush=True);raise
    def tap(label,bounds=(70,CHAT_X)):click(*find(label,bounds))
    def space_alerts():
        # Space name → dropdown → Notification Settings opens the sidebar panel.
        click(*wait_for("space Notification Settings",lambda:ui.space_menu("notification settings",lambda p:click(*p)),15))
    def room_alerts():
        # The bell icon in the channel header.
        click(*wait_for("room Notification Settings",lambda:ui.header("notification settings"),20))
    def space_option(m):
        # Three equal-height buttons above the help paragraph. Anchor to the
        # painted final button, independent of wrapping/current-value text.
        # On native, OCR sometimes merges "All" with a rail glyph at its left.
        def geometry():
            ws=words()
            button_rows=[y for w,x,y in ws if 70<x<CHAT_X and w=="mute"]
            help_rows=[y for w,x,y in ws if 70<x<CHAT_X and w=="includes"]
            if not button_rows or not help_rows:return None
            last=max(button_rows)
            if not 12<min(help_rows)-last<38:return None
            return 120,last-{"quiet":0,"all":30,"default":60}[m]
        click(*wait_for("space option "+m,geometry,12))
    def select_space(name):
        focus()
        # The rail contains exactly these two joined spaces, in room-ID
        # order. Check the rendered *name* after clicking the icon; full
        # screen OCR often ignores isolated one-letter glyphs.
        i=sorted([parent,nested]).index({"Garden":parent,"Beds":nested}[name])
        def selected():
            if any(w==name.lower() and 70<x<CHAT_X and y<45 for w,x,y in words()):return True
            click(38,150+i*52)
        wait_for("selected space "+name,selected,18)
    count=0
    def deliver(r,b,u,n):
        nonlocal count
        hide()
        eid=send(r,b,u,n)
        if second_device:second_device[1](r,eid,b,n)
        if n:
            try:desk.popup(b,appname=app)
            except RuntimeError:
                print("expected",b,"bus calls",desk.count(),"displayed",desk.run(desk.ctl,"count","displayed"),
                      "logged expected body",b in desk.log.read_text(),flush=True);raise
            count+=1
            if second_device:second_device[0].popup(b)
        assert desk.count()==count
        if second_device:assert second_device[0].count()==count
    select_space("Garden")
    find("orchard");find("media-proof")  # direct and nested channels in parent sidebar
    space_alerts();space_option("quiet");settled({room:[],child:[]})
    deliver(room,"Quiet direct",None,False);deliver(child,"Quiet nested",None,False)
    deliver(child,"Mention beacon",me,True);deliver(control,"Outside garden",None,True)
    focus();tap("media-proof")
    if not pos("mute",(CHAT_X,1180),words()):room_alerts()
    tap("all",(CHAT_X,1180))
    wait_for("room override",lambda:any(r["rule_id"]==room and r["actions"]==["notify"] for r in rules()["room"]))
    deliver(room,"Room exception",None,True)
    restart();select_space("Garden");space_alerts()
    assert http("GET",api+"/user/"+quote(me,safe="")+"/rooms/"+quote(parent,safe="")+"/account_data/"+PREFIX,
                headers=hdr).json()=={"mode":"quiet"}
    deliver(child,"Quiet after restart",None,False)
    select_space("Beds")
    if not pos("mute",(70,CHAT_X),words()):space_alerts()
    space_option("all");settled({room:[],child:["notify"]})
    if app=="Untethered":
        # Separate the change of notification policy from the instant of
        # switching keyboard focus away from the currently selected channel.
        # The app still receives background traffic while looking at its inbox.
        tap("mentions");find("inbox",(CHAT_X,1180))
    deliver(child,"Nearer space wins",None,True)
    master=api+"/pushrules/global/override/.m.rule.master/enabled"
    def dnd(on):
        http("PUT",master,headers=hdr,json={"enabled":on})
        assert http("GET",master,headers=hdr).json()=={"enabled":on}
        focus()
        if on:find("disturb")
        else:wait_for("DND off in receiver footer",lambda:any(w=="online" and x<CHAT_X and y>650 for w,x,y in words()))
    dnd(True);deliver(child,"DND override",me,False);dnd(False)
    link(nested,child,False);settled({room:[]})
    deliver(child,"Removed child",None,True)
    link(parent,child,True);settled({room:[],child:[]})
    deliver(child,"Newly linked",None,False)
    select_space("Garden")
    if not pos("mute",(70,CHAT_X),words()):space_alerts()
    space_option("default");settled({})
    deliver(child,"Default restored",None,True)
    after=rules();exception=next(r for r in after["room"] if r["rule_id"]==room)
    assert exception["actions"]==["notify"] and exception["enabled"]
    after["room"].remove(exception);assert after==baseline
    for receiver in ([desk,second_device[0]] if second_device else [desk]):
        assert count==6 and receiver.run(receiver.ctl,"count","history")=="6"
        assert all(s not in receiver.log.read_text() for s in ("Quiet direct","Quiet nested","Quiet after restart","Newly linked","DND override"))
    print(f"{app}: muted direct and nested rooms, mention and outside room notified, channel override won, "
          "restart preserved mute. Nearer space, last-child removal and new child reconciled. "
          "6 visible popups; restoring the parent left unrelated push rules unchanged.",flush=True)
