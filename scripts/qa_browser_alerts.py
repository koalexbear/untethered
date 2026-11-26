"""Open-tab browser notification proof: actual Chromium, Matrix worker and Dunst."""
import io
import os
import re
import shutil
import subprocess as sp
from contextlib import ExitStack
from urllib.parse import quote

from PIL import Image
from playwright.sync_api import sync_playwright

from qa_kwin_element import element_server,login_element
from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_room_alerts import NotificationDesktop
from qa_ui import CHAT_X,Ui,read
from qa_web_login import gomuks_page


def check_browser_alerts(tmp,hs,web,backend,seed,http,*,dnd=False,reading=False,spaces=False):
    receiver,hdr=seed(tmp,hs)
    other=tmp/"sender";other.mkdir()
    sender,sh=seed(other,hs)
    room=receiver["room"]
    me="@"+receiver["username"]+":test.invalid"
    them="@"+sender["username"]+":test.invalid"
    assert me!=them
    api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
    http("POST",api+"/invite",headers=hdr,json={"user_id":them})
    http("POST",api+"/join",headers=sh,json={})
    with ExitStack() as stack:
        env=isolated_env(tmp,backend,"x11")
        start_display(stack,tmp,env,"x11",1520,1050)
        desk=None if reading else NotificationDesktop(stack,tmp,env)
        element=stack.enter_context(element_server(hs))
        pw=stack.enter_context(sync_playwright())
        exe=os.environ.get("CHROMIUM") or shutil.which("chromium")
        peer_browser=pw.chromium.launch(executable_path=exe,env=env,args=["--disable-dev-shm-usage"])
        stack.callback(peer_browser.close)
        peer_ctx=peer_browser.new_context(viewport={"width":1280,"height":920},locale="en-US")
        peer_ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"),lambda r:r.abort())
        peer=login_element(peer_ctx,element,sender,room)
        # Playwright normally forces each target to act focused. Attach to
        # a headed browser with no default overrides so real tab changes fire.
        profile=tmp/"browser";profile.mkdir()
        opts=["--no-sandbox"] if os.geteuid()==0 else []
        chromium=stack.enter_context(child([exe,*opts,"--ozone-platform=x11","--enable-features=NativeNotifications",
            "--enable-unsafe-swiftshader","--ignore-gpu-blocklist","--disable-dev-shm-usage","--disable-background-networking",
            "--password-store=basic","--disable-sync","--no-first-run","--no-default-browser-check","--remote-debugging-port=0",
            "--window-size=1250,920","--user-data-dir="+str(profile),"about:blank"],env))
        def port():
            assert chromium.poll() is None,"isolated Chrome exited"
            file=profile/"DevToolsActivePort"
            return file.read_text().splitlines()[0] if file.exists() and file.stat().st_size else None
        receiver_browser=pw.chromium.connect_over_cdp("http://127.0.0.1:"+wait_for("Chrome port",port),no_defaults=True)
        stack.callback(receiver_browser.close)
        ctx=receiver_browser.contexts[0]
        page=ctx.pages[0]
        page.on("pageerror",lambda e:print("Receiver page error:",str(e)[:250],flush=True))
        page.set_viewport_size({"width":1180,"height":760})
        # Observe genuine worker output, never substitute permission, visibility,
        # notification APIs, events, or the device's decryption decisions.
        page.add_init_script("""window.qaSeen={};
          window.Worker=class extends Worker{constructor(...args){super(...args);
            this.addEventListener('message',({data:f})=>{
              if(!['sync_complete','events_decrypted'].includes(f.command))return;
              const data=typeof f.data==='string'?JSON.parse(f.data):f.data;
              for(const delta of f.command==='events_decrypted'?[data]:Object.values(data.rooms||{})){
                for(const e of delta.events||[]){
                  if(e.decrypted?.body && e.event_id?.startsWith('$') && !window.qaSeen[e.event_id]){
                    qaSeen[e.event_id]={body:e.decrypted.body,hidden:document.hidden,mentions:e.decrypted['m.mentions'],
                      sender:e.sender,room:e.room_id,notify:!!(e.unread_type&2),live:f.command==='events_decrypted'||
                        (!!data.since&&!data.catchup&&!data.clear_state&&delta.timeline?.some(t=>t.event_rowid===e.rowid))};
                  }
                }
              }
            });
          }}""")
        page.goto(gomuks_page(web),wait_until="commit")
        page.wait_for_function("document.title.includes('Preview')",timeout=45000)
        page.mouse.click(1095,73);page.wait_for_function("document.title.includes('Account')")
        for y,field in [(333,"server"),(391,"username"),(447,"password")]:
            page.mouse.click(510,y);page.keyboard.type(receiver[field],delay=5)
        page.keyboard.press("Enter")
        recovery=page.get_by_label("Recovery key or passphrase")
        recovery.fill(receiver["key"],timeout=45000);recovery.press("Enter")
        page.wait_for_function("document.title.includes('media-proof')",timeout=45000)

        def read_words(raw):
            im=Image.open(io.BytesIO(raw)).convert("RGB")
            return [(w.lower().strip(".,:|()"),x,y) for w,x,y in read(im)]
        def words():return read_words(page.screenshot())
        ui=Ui(words,lambda pos:page.mouse.move(*pos),lambda:Image.open(io.BytesIO(page.screenshot())).convert("RGB"))
        def click_word(label,near=None):
            def find():
                ws=words()
                return next(((x,y) for w,x,y in ws if w==label and x>CHAT_X and
                            (not near or any(v==near and abs(yy-y)<6 for v,_,yy in ws))),None)
            try:point=wait_for(label,find,12)
            except RuntimeError:print("Browser UI:",[w for w,_,_ in words()],flush=True);raise
            page.mouse.click(*point)
        def muted():
            rules=http("GET",hs+"/_matrix/client/v3/pushrules/",headers=hdr).json()["global"]["room"]
            return any(r["rule_id"]==room and r["enabled"] and r["actions"]==[] for r in rules)
        if not reading:
            # The bell icon in the channel header opens room notification settings.
            page.mouse.click(*wait_for("Notification Settings header control",lambda:ui.header("notification settings"),20))
            if not spaces:
                click_word("mute","all")
                wait_for("Matrix room muted by the browser",muted)
        assert page.evaluate("Notification.permission")=="default"
        other_tab=ctx.new_page()
        other_tab.set_content("<title>Foreground control</title><p>Reading another tab</p>")
        def hide():
            other_tab.bring_to_front()
            try:page.wait_for_function("document.visibilityState==='hidden' && !document.hasFocus()",polling=150,timeout=8000)
            except Exception:
                print("Tab visibility/focus:",[p.evaluate("[document.visibilityState,document.hasFocus()]") for p in (page,other_tab)],flush=True);raise
            assert other_tab.evaluate("!document.hidden")
        def send(body,target,hidden,expected,in_room=room):
            eid=peer.evaluate("""async ([r,b,u])=>(await window.mxMatrixClientPeg.get().sendEvent(r,'m.room.message',{
                body:b,msgtype:'m.text','m.mentions':u?{user_ids:[u]}:{}})).event_id""",[in_room,body,target])
            wire=http("GET",hs+"/_matrix/client/v3/rooms/"+quote(in_room,safe="")+"/event/"+quote(eid,safe=""),headers=hdr).json()
            assert wire["type"]=="m.room.encrypted" and wire["content"]["algorithm"]=="m.megolm.v1.aes-sha2"
            page.wait_for_function("id=>!!window.qaSeen[id]",arg=eid,polling=200,timeout=35000)
            seen=page.evaluate("id=>window.qaSeen[id]",eid)
            assert seen.get("live"),"expected incremental sync or fresh decryption"
            assert (seen["body"],seen["room"],seen["sender"],seen["hidden"],seen["notify"])==(body,in_room,them,hidden,expected),seen
            assert seen["mentions"].get("user_ids",[])==([target] if target else [])
            return eid
        hide()
        if reading:
            from qa_read_state import check_read_state
            check_read_state(stack,tmp,hs,backend,room,me,hdr,http,page,peer,words)
            return
        assert desk.count()==0
        if not dnd and not spaces:send("Before permission",me,True,True)
        page.bring_to_front()
        click_word("enable","browser")
        def screen_words():
            return read_words(sp.check_output(["import","-window","root","png:-"],env=env,timeout=6))
        def allow():
            ws=screen_words()
            # Inspect the real Chromium chrome, not a page-injected dialog.
            buttons=[(x,y) for w,x,y in ws if w=="allow" and y<420
                     and not any(v in ("don’t","don't","not") and 0<x-xx<85 and abs(yy-y)<5 for v,xx,yy in ws)]
            return buttons[-1] if buttons else None
        try:button=wait_for("browser permission bubble",allow,10)
        except RuntimeError:print("Permission UI:",[(w,x,y) for w,x,y in screen_words() if y<420],flush=True);raise
        desk.run("xdotool","mousemove",*map(str,button),"click","1")
        page.wait_for_function("Notification.permission==='granted'",timeout=8000)
        print("Clicked Enable browser notifications and allowed the real Chromium site prompt on the disposable origin.",flush=True)
        hide()
        assert desk.count()==0
        if spaces:
            from qa_space_alerts import check_spaces,companion
            second=companion(stack,tmp,backend)
            def restart():
                page.bring_to_front();page.reload()
                page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
            def space_click(x,y):
                page.mouse.click(x,y);page.mouse.move(650,440)
            check_spaces(hs,peer,me,hdr,http,room,words,space_click,page.bring_to_front,hide,
                         restart,lambda r,b,u,n:send(b,u,True,n,r),desk,"Chromium",second_device=second,ui=ui)
            return
        if dnd:
            from qa_account_dnd import verify_dnd
            verify_dnd(stack,tmp,hs,backend,hdr,http,me,room,page,peer,words,click_word,send,hide,desk)
            return
        send("Quiet orchard",None,True,False)
        send("Another recipient",them,True,False)
        mention=send("Browser beacon",me,True,True)
        def still_hidden():assert page.evaluate("document.hidden && !document.hasFocus()")
        desk.popup("Browser beacon",appname="Chromium",activate=True,on_visible=still_hidden)
        assert desk.count()==1
        print(f"Hidden Untethered tab: decrypted ordinary and other-user mention, no alerts. Direct mention "
              f"{mention}: one visible Chromium notification while the receiver was still hidden.",flush=True)
        page.wait_for_function("!document.hidden && document.hasFocus()",polling=100,timeout=8000)
        send("Already visible",me,False,True)
        page.reload();page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
        assert page.evaluate("Notification.permission")=="granted"
        hide()
        send("Muted after reload",None,True,False)
        assert muted() and desk.count()==1 and desk.run(desk.ctl,"count","history")=="1"
        assert not any(t in desk.log.read_text() for t in ("Before permission","Quiet orchard","Another recipient","Already visible","Muted after reload"))
        print("Popup action returned focus to Untethered. After reload and another hidden ordinary message: "
              "1 total Notify call, 1 archived popup, mute rule retained.",flush=True)
