"""Composer autocomplete against Element Web: `@`, `#` and `:` typed in the
native composer, in its message editor and in a reply, read back from
Element's decrypted events and its rendered, highlighted tiles."""
import os
import time
from pathlib import Path

from qa_native_paste import wait_for
from qa_ui import CHAT_X


def check_composer(page,room,uid,click,x,locate,ui,capture,messages):
    shots=os.environ.get("COMPOSER_QA_SHOTS")
    def shot(name):
        if shots:
            Path(shots).mkdir(parents=True,exist_ok=True)
            capture().save(Path(shots)/f"{name}.png")
    def lines(ws,label):return [(xx,y) for v,xx,y in ws if v==label and xx>CHAT_X]
    # The popup floats just above the composer, below the header's room name.
    def popup(label):return lambda ws:[p for p in lines(ws,label) if p[1]>300]
    def event(marker,edit=False):
        found=page.evaluate("""([room,marker,edit])=>window.mxMatrixClientPeg.get().getRoom(room)?.getLiveTimeline().getEvents()
            .filter(e=>e.getId()?.startsWith('$')&&!e.isDecryptionFailure()&&e.getType()==='m.room.message'
                &&(edit?e.isRelation('m.replace'):!e.isRelation('m.replace'))&&e.getOriginalContent().body?.includes(marker))
            .map(e=>({id:e.getId(),content:e.getOriginalContent(),reply:e.replyEventId}))""",[room,marker,edit])
        return found[0] if len(found)==1 else None
    def highlighted(eid,pills,by=None):
        """Element highlights the tile: its push rules give the event (or the
        edit `by` that replaced it) the highlight tweak, which is what
        Element's tile and room badge use; and it draws `pills` pills."""
        tile=page.locator(f'[data-event-id="{eid}"]')
        tile.wait_for(timeout=15000)
        wait_for(f"Element's highlight on {by or eid}",lambda:page.evaluate("""([room,id])=>{
            const c=window.mxMatrixClientPeg.get(),e=c.getRoom(room)?.findEventById(id);
            return !!(e&&c.getPushActionsForEvent(e)?.tweaks?.highlight)}""",[room,by or eid]),15)
        wait_for(f"{pills} Element pill(s) in {eid}",lambda:tile.locator(".mx_Pill").count()==pills,15)

    user_link=f'<a href="https://matrix.to/#/{uid}">Mira</a>'
    room_href=f"https://matrix.to/#/{room}?via=test.invalid"

    # 1. A new message: a mention by Tab, a channel by Enter, emoji both ways.
    click(ui.composer())
    x("type","--clearmodifiers","Ping @mi")
    locate("member popup listing Mira",popup("mira"))
    shot("1-members")
    x("key","Tab")
    x("type","--clearmodifiers","see #med")
    locate("channel popup listing media-proof",popup("media-proof"))
    shot("2-channels")
    x("key","Return")
    x("type","--clearmodifiers","party :tad")
    # OCR may read the colons of ":tada:" as letters ("stada").
    locate("emoji popup listing :tada:",lambda ws:[(xx,y) for w,xx,y in ws if "tada" in w and xx>CHAT_X and y>300])
    shot("3-emoji")
    x("key","Tab")
    x("type","--clearmodifiers","then :wave: done")
    shot("4-draft")
    x("key","Return")
    sent=wait_for("Element decrypted the native mention",lambda:event("then"),30)
    c=sent["content"]
    assert c["body"]==f"Ping Mira see {room_href} party 🎉 then 👋 done",c["body"]
    assert c["m.mentions"]=={"user_ids":[uid]},c
    assert c["format"]=="org.matrix.custom.html" and user_link in c["formatted_body"],c
    assert f'<a href="{room_href}">#media-proof</a>' in c["formatted_body"],c
    highlighted(sent["id"],2)
    print("Element: @mi+Tab sent a Mira pill in m.mentions and highlighted the tile; "
          "#med+Enter a matrix.to room pill; :tad+Tab and :wave: their emoji.",flush=True)

    # 1b. The emoji button: search the picker, Enter inserts at the cursor.
    x("type","--clearmodifiers","Liftoff  now")
    x("key","Left","Left","Left","Left")
    click(ui.at(ui.width-40-46,ui.height-52))  # Left of the send arrow.
    time.sleep(.5)
    x("type","--clearmodifiers","rocket")
    # The picker's search field floats above the composer, not in it.
    locate("rocket in the picker search",lambda ws:[p for p in lines(ws,"rocket") if 300<p[1]<ui.height-90])
    shot("4b-picker")
    x("key","Return")
    time.sleep(.3)
    x("key","Return")
    picked=wait_for("Element decrypted the picker's emoji",lambda:event("Liftoff"),30)
    assert picked["content"]["body"]=="Liftoff 🚀 now",picked
    assert picked["content"]["m.mentions"]=={},picked
    print("Emoji button: the picker's search inserted 🚀 at the cursor.",flush=True)

    # 2. An edit that adds a mention notifies through the replacement.
    x("type","--clearmodifiers","Quiet note");x("key","Return")
    base=wait_for("Element decrypted the edit target",lambda:event("Quiet note"),30)
    assert base["content"]["m.mentions"]=={},base
    row=locate("native edit target",lambda ws:next(iter(sorted(lines(ws,"quiet"),key=lambda p:-p[1])),None))
    click(wait_for("Edit on the quiet note",lambda:ui.message(row,"edit"),15))
    time.sleep(.5)
    x("key","End")
    x("type","--clearmodifiers"," for @mi")
    locate("member popup in the message editor",popup("mira"))
    shot("5-edit")
    x("key","Tab");x("key","Return")
    edit=wait_for("Element decrypted the native edit",lambda:event("for Mira",edit=True),30)
    new=edit["content"]["m.new_content"]
    assert edit["content"]["m.relates_to"]=={"rel_type":"m.replace","event_id":base["id"]},edit
    assert new["body"]=="Quiet note for Mira" and user_link in new["formatted_body"],new
    assert new["m.mentions"]=={"user_ids":[uid]},new
    assert edit["content"]["m.mentions"]=={"user_ids":[uid]},"an added mention notifies"
    highlighted(base["id"],1,by=edit["id"])
    print("Element: the edit added a Mira pill; m.mentions on the replacement and its new content.",flush=True)

    # 3. A reply keeps its text literal around the pill.
    [hello]=messages(["Element asks a question"])
    row=locate("Element's question",lambda ws:next(iter(lines(ws,"asks")),None))
    click(wait_for("Reply on Element's question",lambda:ui.message(row,"reply"),15))
    locate("native reply draft",lambda ws:lines(ws,"replying"))
    x("type","--clearmodifiers","*literal* @mi")
    locate("member popup in a reply",popup("mira"))
    x("key","Tab")
    x("type","--clearmodifiers","answered")
    x("key","Return")
    reply=wait_for("Element decrypted the native reply",lambda:event("answered"),30)
    rc=reply["content"]
    assert reply["reply"]==hello,reply
    assert rc["body"]=="*literal* Mira answered",rc
    assert rc["formatted_body"]==f"*literal* {user_link} answered",rc
    assert rc["m.mentions"]=={"user_ids":[uid]},rc
    highlighted(reply["id"],1)
    print("Element: the reply mentioned Mira with literal text and a pill.",flush=True)
