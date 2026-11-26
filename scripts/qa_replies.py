"""Two device UI test. The duplicate originals have the same author and exact text."""
import io
import json
import os
import re
import shutil
import subprocess as sp
from contextlib import ExitStack
from urllib.parse import quote,unquote

from PIL import Image
from playwright.sync_api import sync_playwright
from qa_kwin_element import element_server,login_element
from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_ui import CHAT_X,Ui,read
from qa_upload_cancel import slow_gateway


def screen_words(raw):
    # The shared reader also finds labels on filled buttons and in open dialogs.
    im=Image.open(io.BytesIO(raw)).convert("RGB")
    return [(w.lower().strip(".,:|()"),x,y) for w,x,y in read(im)]


def test_replies(tmp,hs,root,backend,seed,http,port,*,rich=False,reactions=False,room_search=False,alerts=False,permissions=False,ban=False,kick=False,report=False,client_web=None,mentions=False,peer_hs=None,spaces=False,setup=False):
    paths=[]
    def observe(method,path):
        if room_search or method=="GET" and "/context/" in path or setup and method=="POST" and path.endswith("/createRoom"):paths.append(unquote(path))
    refused=[]
    def refuse_link(method,path):
        if method=="PUT" and "/state/m.space.child/" in unquote(path) and not refused:
            refused.append(path);return True
        return False
    with ExitStack() as stack:
        proxy,_=stack.enter_context(slow_gateway(hs,port,observe_path=observe,reject_once=refuse_link if setup else None))
        native,hdr=seed(tmp,proxy)
        remote=tmp/"peer";remote.mkdir()
        bob,bh=seed(remote,peer_hs or hs)
        room=native["room"];api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
        uid=(http("GET",peer_hs+"/_matrix/client/v3/account/whoami",headers=bh).json()["user_id"]
             if peer_hs else "@"+bob["username"]+":test.invalid")
        if report:
            # The reporter joins the sender's room as an ordinary member.
            http("POST",api+"/leave",headers=hdr,json={})
            room=bob["room"];api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
            reporter=http("GET",hs+"/_matrix/client/v3/account/whoami",headers=hdr).json()["user_id"]
            http("POST",api+"/invite",headers=bh,json={"user_id":reporter})
            http("POST",api+"/join",headers=hdr,json={})
        elif kick:
            http("PUT",api+"/state/m.room.join_rules",headers=hdr,json={"join_rule":"public"})
        else:
            http("POST",api+"/invite",headers=hdr,json={"user_id":uid})
        member_api=(peer_hs or hs)+"/_matrix/client/v3/rooms/"+quote(room,safe="")
        if not report:http("POST",member_api+"/join",headers=bh,json={})
        web=stack.enter_context(element_server(peer_hs or hs,server_name=uid.split(":",1)[1]));pw=stack.enter_context(sync_playwright())
        flags=["--enable-unsafe-swiftshader","--ignore-gpu-blocklist"] if client_web else []
        browser=pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                   args=["--disable-dev-shm-usage",*flags]);stack.callback(browser.close)
        ctx=browser.new_context(viewport={"width":1280,"height":920},locale="en-US")
        ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"),lambda route:route.abort())
        page=login_element(ctx,web,bob,room)
        def messages(texts):
            return page.evaluate("""async ([room,texts])=>{
                const c=window.mxMatrixClientPeg.get(),ids=[];
                for(const text of texts)ids.push((await c.sendTextMessage(room,text)).event_id);
                return ids;
            }""",[room,texts])
        def tile(eid):return page.locator(f'[data-event-id="{eid}"]')
        def record(body):
            return page.evaluate("""([room,body])=>window.mxMatrixClientPeg.get().getRoom(room)?.getLiveTimeline().getEvents()
                .filter(e=>e.getContent().body?.endsWith(body)&&e.getId()?.startsWith('$')&&e.replyEventId)
                .map(e=>({id:e.getId(),target:e.replyEventId}))""",[room,body])
        def locate(what,fn):
            try:return wait_for(what,lambda:fn(words()),15)
            except RuntimeError:
                print("Client visible words:",[w for w,_,_ in words()],flush=True);raise
        if client_web:
            from qa_message_report import check_reports
            from qa_web_login import login_client
            reporter_ctx=browser.new_context(viewport={"width":1180,"height":760})
            client=reporter_ctx.new_page()
            login_client(client,client_web,native)
            def words():return screen_words(client.screenshot())
            ui=Ui(words,lambda pos:client.mouse.move(*pos),
                  lambda:Image.open(io.BytesIO(client.screenshot())).convert("RGB"))
            if setup:
                from qa_community_setup import check_setup
                def restart():
                    client.reload()
                    client.wait_for_function("document.title.includes('Untethered') && !document.title.includes('Preview') && !document.title.includes('Account')",timeout=45000)
                check_setup(hs,http,hdr,bh,page,locate,lambda pos:client.mouse.click(*pos,delay=60),
                            lambda s:client.keyboard.type(s,delay=5),restart,paths,ui)
                return
            if mentions:
                from qa_mentions_inbox import check_inbox
                def restart():
                    nonlocal client
                    client.close()
                    client=reporter_ctx.new_page()
                    client.goto(client_web,wait_until="commit")
                    client.wait_for_function("document.title.includes('media-proof')",timeout=45000)
                check_inbox(hs,page,room,hdr,http,restart,locate,lambda pos:client.mouse.click(*pos),paths)
                return
            def enter(n=1):
                for _ in range(n):client.keyboard.press("Enter")
            check_reports(tmp,hs,api,room,page,hdr,http,messages,locate,lambda pos:client.mouse.click(*pos),
                          lambda s:client.keyboard.type(s,delay=5),enter,ui)
            return
        env=isolated_env(tmp,backend,"x11")
        if spaces:start_display(stack,tmp,env,"x11",1820,980)
        else:start_display(stack,tmp,env,"x11")
        if alerts or spaces:
            from qa_room_alerts import NotificationDesktop
            desktop=NotificationDesktop(stack,tmp,env)
        native_life=stack.enter_context(ExitStack())
        def x(*args):return sp.check_output(["xdotool",*args],env=env,text=True,timeout=6).strip()
        def launch(title="Untethered.*media-proof"):
            p=native_life.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
            def visible():
                assert p.poll() is None
                r=sp.run(["xdotool","search","--onlyvisible","--name",title],
                         env=env,text=True,capture_output=True)
                return r.stdout.strip() if r.returncode==0 else None
            wid=wait_for("native synced window",visible,30)
            x("windowmove",wid,"0","0","windowsize",wid,"1180","760","windowfocus",wid)
            return wid
        win=launch()
        def words():
            return screen_words(sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6))
        def click(pos):x("mousemove","--window",win,*map(str,pos),"click","1")
        def capture():
            return Image.open(io.BytesIO(sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6))).convert("RGB")
        ui=Ui(words,lambda pos:x("mousemove","--window",win,*map(str,pos)),capture)
        def lines(words,label):return [(x,y) for v,x,y in words if v==label and x>CHAT_X]
        if setup:
            from qa_community_setup import check_setup
            def restart_setup():
                nonlocal win
                native_life.close();win=launch("Untethered.*(Lounge|Workshop|media-proof)")
            check_setup(hs,http,hdr,bh,page,locate,click,lambda s:x("type","--clearmodifiers",s),restart_setup,paths,ui)
            return
        if spaces:
            from qa_space_alerts import check_spaces,native_sender
            def restart():
                nonlocal win
                native_life.close();win=launch()
            me="@"+native["username"]+":test.invalid"
            def space_click(xx,y):
                click((xx,y));x("mousemove","--window",win,"650","440")
            def foreground():
                x("windowraise",win);desktop.focus(win)
            def background():
                x("windowraise",desktop.other,"mousemove","--window",desktop.other,"70","25")
                desktop.focus(desktop.other)
            check_spaces(hs,page,me,hdr,http,room,words,space_click,foreground,
                background,restart,native_sender(tmp,hs,page,hdr,http),desktop,"Untethered",ui=ui)
            return
        if rich:
            from qa_rich_text import check_rich
            check_rich(page,room,locate,click,lambda pos:x("mousemove","--window",win,*map(str,pos)),
                       lambda k:x("key","--clearmodifiers",k),capture,ui.composer())
            return
        if mentions:
            from qa_mentions_inbox import check_inbox
            def restart():
                nonlocal win
                native_life.close()
                win=launch()
            check_inbox(hs,page,room,hdr,http,restart,locate,click,paths)
            return
        if report:
            from qa_message_report import check_reports
            check_reports(tmp,hs,api,room,page,hdr,http,messages,locate,click,lambda s:x("type","--clearmodifiers",s),
                          lambda n=1:x("key",*(["Return"]*n)),ui)
            return
        if permissions or ban or kick:
            def restart_native():
                nonlocal win
                native_life.close()
                win=launch()
            if ban or kick:
                from qa_room_removal import check_removal
                check_removal(page,api,room,peer_hs,hdr,bh,http,locate,click,restart_native,
                              lambda *args:x(*[win if arg=="WIN" else arg for arg in args]),ui,kick=kick)
            else:
                from qa_room_permissions import check_permissions
                check_permissions(page,api,room,native,hdr,bh,http,locate,click,restart_native,
                                  lambda *args:x(*[win if arg=="WIN" else arg for arg in args]),ui)
            return
        if alerts:
            from qa_room_alerts import check_room_alerts
            def restart_native():
                nonlocal win
                native_life.close()
                win=launch()
            check_room_alerts(tmp,hs,page,room,native,hdr,http,desktop,locate,click,restart_native,lambda:win,ui)
            return
        if room_search:
            from qa_room_search import check_room_search
            def restart_native():
                nonlocal win
                native_life.close()
                win=launch()
            check_room_search(tmp,hs,page,room,native,hdr,http,messages,restart_native,words,locate,click,
                              lambda *args:x(*[win if arg=="WIN" else arg for arg in args]),paths,ui)
            return
        if reactions:
            from qa_reactions import check_reactions
            def restart():
                nonlocal win
                native_life.close()
                page.reload()
                win=launch()
            def hover(pos):
                x("mousemove","--window",win,*map(str,pos))
            def react_click(pos):
                click(pos)
                hover((65,60))  # Keep chip tooltips off the message during OCR.
            def room_get(suffix,version="v3"):
                return http("GET",f"{hs}/_matrix/client/{version}/rooms/{quote(room,safe='')}"+suffix,headers=hdr).json()
            check_reactions(page,room,uid,room_get,
                            locate,react_click,hover,restart,ui)
            return
        first,_,second,_=messages(["Beacon original","Cobalt waypoint","Beacon original","Amber waypoint"])
        def first_original(ws):
            originals=sorted(lines(ws,"beacon"),key=lambda p:p[1])
            return len(originals)==2 and originals[0]
        # Message actions appear in the toolbar of the hovered row.
        row=locate("first of two identical messages",first_original)
        click(wait_for("Reply on first of two identical messages",lambda:ui.message(row,"reply"),15))
        locate("native reply draft",lambda ws:lines(ws,"replying"))
        # Element drafts its reply to the other event using its public Reply button.
        tile(second).hover()
        tile(second).get_by_role("button",name="Reply",exact=True).click()
        page.locator(".mx_ReplyPreview").wait_for()
        # Fill the gap using actual encrypted events rather than mock timeline rows.
        gaps=messages([f"History marker {i:03}" for i in range(80)])
        assert len(set(gaps))==80
        nbody="Northern response";ebody="Southern response"
        x("type","--clearmodifiers",nbody);x("key","Return")
        nr=wait_for("Element decoded native reply",lambda:record(nbody))
        assert len(nr)==1 and nr[0]["target"]==first,nr
        editor=page.locator('[contenteditable=true]:visible').last
        editor.fill(ebody);editor.press("Enter")
        er=wait_for("Element confirmed outgoing reply",lambda:record(ebody))
        assert len(er)==1 and er[0]["target"]==second,er
        locate("both original replies on the native timeline",lambda ws:lines(ws,"northern") and lines(ws,"southern"))
        def edit_native(word,body):
            row=locate("native message "+word,lambda ws:(rows:=lines(ws,word)) and rows[-1])
            click(wait_for("native edit action for "+word,lambda:ui.message(row,"edit"),15))
            # The edit field shows "Escape to cancel · Enter to save edit".
            locate("editable message",lambda ws:lines(ws,"escape"))
            x("key","ctrl+a");x("type","--clearmodifiers",body);x("key","Return")
            tile(nr[0]["id"]).get_by_text(body,exact=True).wait_for()
            locate("native saved revision",lambda ws:lines(ws,body.split()[0].lower()) and not lines(ws,"escape"))
        def replacing(eid):
            return page.evaluate("([r,id])=>window.mxMatrixClientPeg.get().getRoom(r).findEventById(id)?.replacingEventId()",[room,eid])
        # Both native saves must replace the stable reply, not chain edits.
        edit_native("northern","Interim wording")
        interim=wait_for("Element aggregated first edit",lambda:replacing(nr[0]["id"]))
        edit_native("interim","Polar revision")
        final=wait_for("Element aggregated second edit",lambda:(v if (v:=replacing(nr[0]["id"])) and v!=interim else None))
        # ArrowUp in Element's empty composer edits the user's last message.
        editor.click();editor.press("ArrowUp")
        editbox=page.locator(".mx_EditMessageComposer [contenteditable=true]")
        editbox.wait_for()
        assert ebody in editbox.inner_text()
        editbox.fill("Meridian correction")
        page.locator(".mx_EditMessageComposer").get_by_role("button",name="Save",exact=True).click()
        tile(er[0]["id"]).get_by_text("Meridian correction",exact=True).wait_for()
        incoming=wait_for("Element edit confirmed",lambda:(v if (v:=replacing(er[0]["id"])) and v.startswith("$") else None))
        def edited_rows(ws):
            if not all(len(lines(ws,w))==1 for w in ("polar","meridian")):return False
            if any(lines(ws,w) for w in ("northern","southern","interim")):return False
            return all(any(abs(yy-lines(ws,w)[0][1])<7 for _,yy in lines(ws,"edited")) for w in ("polar","meridian"))
        locate("both edited native timeline rows",edited_rows)
        print("Live: two native saves edited the same reply; Element edited its own reply. "
              "Both timelines show Polar revision and Meridian correction in place.",flush=True)
        # Check every encrypted message; distinguish replacements from duplicates.
        timeline=http("GET",api+"/messages",headers=hdr,params={"dir":"b","limit":100}).json()["chunk"]
        wire=[e for e in timeline if e["type"] in ("m.room.encrypted","m.room.message")]
        assert len(wire)==89 and len({e["event_id"] for e in wire})==89
        replacements={e["event_id"]:e["content"]["m.relates_to"]["event_id"] for e in wire
                      if e["content"].get("m.relates_to",{}).get("rel_type")=="m.replace"}
        assert replacements=={interim:nr[0]["id"],final:nr[0]["id"],incoming:er[0]["id"]},replacements
        assert set([first,second,nr[0]["id"],er[0]["id"],*gaps])<={e["event_id"] for e in wire}
        assert all(e["type"]=="m.room.encrypted" and e["content"]["algorithm"]=="m.megolm.v1.aes-sha2" for e in wire)
        assert not any(t in json.dumps(wire) for t in (nbody,ebody,"Beacon original","Polar revision","Meridian correction","Interim wording"))
        print(f"Element decrypted Untethered's reply → {first}; Element replied to the look-alike → {second}. "
              "Synapse: 89 distinct Megolm messages, including 3 replacements targeting the 2 reply IDs.",flush=True)
        def jump_above(body):
            # The quoted line above a reply ("@sender Beacon original") opens it.
            def find(ws):
                b=lines(ws,body)
                if not b:return None
                quotes=[(x,y) for x,y in lines(ws,"beacon") if 0<b[-1][1]-y<70]
                return max(quotes,key=lambda p:p[1],default=None)
            click(locate("original link for "+body,find))
        def check_marker(marker):
            locate("highlighted original in its actual history",lambda ws:
                   lines(ws,"original") and lines(ws,"older") and lines(ws,"beacon") and
                   any(0<y-lines(ws,"beacon")[0][1]<54 for _,y in lines(ws,marker)))
        def latest():
            click(locate("return to recent timeline",lambda ws:next(iter(lines(ws,"latest")),None)))
            locate("reply timeline restored",edited_rows)
        paths.clear()
        jump_above("meridian");check_marker("amber");latest()
        assert not paths,"loaded original should not require a context request"
        # End the native transport AND UI, then reopen its existing device profile.
        native_life.close()
        page.reload()
        tile(nr[0]["id"]).wait_for()
        win=launch()
        locate("both edited replies on cold native timeline",edited_rows)
        live=page.evaluate("r=>window.mxMatrixClientPeg.get().getRoom(r).getLiveTimeline().getEvents().map(e=>e.getId())",room)
        assert first not in live and second not in live,(len(live),live.index(first) if first in live else None)
        for eid,body in ((nr[0]["id"],"Polar revision"),(er[0]["id"],"Meridian correction")):
            tile(eid).get_by_text(body,exact=True).wait_for()
        assert record("Polar revision")==nr and record("Meridian correction")==er
        print("Cold restart: both clients rendered the two final texts on the original reply IDs; both reply targets unchanged.",flush=True)
        # Drive the rendered quote, not a permalink to the answer. For the
        # second click navigate back to the *reply*, not its distant original.
        for reply,target in [(nr[0],first),(er[0],second)]:
            if target==second:page.goto(web+"/#/room/"+room+"/"+quote(reply["id"],safe=""))
            tile(reply["id"]).locator("blockquote").get_by_text("Beacon original",exact=True).click()
            page.wait_for_function("""id=>{const el=document.querySelector('[data-event-id="'+CSS.escape(id)+'"]');
                if(!el || !(el.className.includes('stateHighlighted') || el.className.includes('stateSelected') ||
                    decodeURIComponent(location.hash).endsWith('/'+id)))return false;
                const r=el.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}""",arg=target)
            print(f"Reloaded Element: edited reply {reply['id']} highlighted {target}; "
                  f"original absent from initial {len(live)} live events.",flush=True)
        for body,target,marker in [("meridian",second,"amber"),("polar",first,"cobalt")]:
            paths.clear()
            jump_above(body)
            check_marker(marker)
            assert paths==[f"/_matrix/client/v3/rooms/{room}/context/{target}"],paths
            print(f"Restarted Untethered: fetched exact /context/{target}; original 'Beacon original' "
                  f"followed by '{marker.title()} waypoint' on screen.",flush=True)
            if target==second:
                click(locate("earlier context",lambda ws:next(iter(lines(ws,"preceding")),None)))
                locate("both originals in preceding context",lambda ws:len(lines(ws,"beacon"))==2 and lines(ws,"cobalt"))
            latest()
