"""Web file chooser → Element → browser Save, including a rejected caption-only attempt."""
import io
import itertools
import subprocess as sp

from PIL import Image
from qa_kwin_element import element_return,check_plain_objects
from qa_native_paste import wait_for
from qa_replies import screen_words
from qa_web_login import login_client


def browser_roundtrip(tmp,hs,browser,fixture,element,room,alice,bob,hdr,uploads,gate,events,original,sources,url):
    ctx=browser.new_context(viewport={"width":1180,"height":760},accept_downloads=True)
    page=ctx.new_page();login_client(page,url,fixture)
    def words():return screen_words(page.screenshot())
    def point(label,fn):
        try:return wait_for(label,lambda:fn(words()),15)
        except RuntimeError:print("Browser labels:",[w for w,_,_ in words()],flush=True);raise
    def pick(*paths):
        # The attach button stays aligned with the editor, whose top moves
        # when caption wrapping or a notice changes the composer layout.
        im=Image.open(io.BytesIO(page.screenshot())).convert("RGB")
        def inside(y):
            r,g,b=im.getpixel((302,y))
            return 95<r<190 and max(r,g,b)-min(r,g,b)<22
        runs=[ys for yes,group in itertools.groupby(range(620,742),key=inside) if yes and len(ys:=list(group))>=18]
        if not runs:raise AssertionError(("Attach button not painted",words()))
        y=runs[-1][len(runs[-1])//2]
        with page.expect_file_chooser(timeout=8000) as picker:page.mouse.click(312,y,delay=85)
        picker.value.set_files([str(p) for p in paths])
    def skipped(ws):
        line={t for t,x,y in ws if x>280 and y>440}
        return {"oversize.bin","65537","65536-byte"}<=line
    caption="Browser boundary proof"
    point("room composer rendered",lambda ws:any(w=="message" and x>280 and y>615 for w,x,y in ws))
    page.mouse.click(545,690);page.keyboard.type(caption)
    point("caption entered",lambda ws:any(t=="boundary" and y>600 for t,_,y in ws))
    pick(sources/"Oversize.bin")
    point("named rejection at 65,537 bytes",skipped)
    page.mouse.click(545,690);page.keyboard.press("Enter")
    page.mouse.click(1139,691)
    point("rejected-only draft keeps caption",lambda ws:skipped(ws) and any(t=="boundary" and y>600 for t,_,y in ws))
    assert not uploads and not events()
    print("Chromium rejected a sole 65,537-byte file. Enter and Send did not publish its caption.",flush=True)
    pick(sources/"Map.png",sources/"Oversize.bin",sources/"Plan.pdf")
    point("two valid files staged beside rejection",lambda ws:skipped(ws) and sum(w=="ready" for w,_,_ in ws)==2)
    assert not uploads and not events()
    page.mouse.click(1139,691)
    wait_for("first real browser upload",lambda:uploads)
    assert not events()
    gate.set()
    wait_for("two plain attachments",lambda:len(events())>=2)
    outbound=events()
    assert len(outbound)==len(uploads)==2 and {e["sender"] for e in outbound}=={alice}
    assert all(e["type"]=="m.room.message" for e in outbound)
    [first]=[e["content"] for e in outbound if e["content"].get("filename")=="Map.png"]
    assert first["body"]==caption
    element.wait_for_function("""([r,ids])=>ids.every(id=>window.mxMatrixClientPeg.get().getRoom(r)
        ?.findEventById(id)?.getContent()?.url?.startsWith('mxc://'))""",arg=[room,[e["event_id"] for e in outbound]])
    returned=element_return(tmp,element,original,next(e["event_id"] for e in outbound if e["content"]["msgtype"]=="m.image"))
    wait_for("two Element return events",lambda:len(events())>=4,30)
    check_plain_objects(hs,hdr,events(),original,alice,bob,uploads)
    page.reload()
    page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
    def button(filename):
        # Query the painted gray Save buttons, not a same-name message body.
        im=Image.open(io.BytesIO(page.screenshot())).convert("RGB")
        def gray(y):
            rgb=im.getpixel((351,y));return 95<rgb[0]<190 and max(rgb)-min(rgb)<22
        for flag,g in itertools.groupby(range(58,669),key=gray):
            ys=list(g)
            if not flag or not 19<=len(ys)<=38:continue
            crop=im.crop((350,ys[0],578,ys[-1]+1)).resize((684,3*len(ys)))
            buf=io.BytesIO();crop.save(buf,"PNG")
            line=sp.check_output(["tesseract","stdin","stdout","--psm","7"],input=buf.getvalue(),stderr=sp.DEVNULL).decode().lower()
            if "save" in line and filename.lower() in line:return (402,ys[len(ys)//2])
    for name,path in returned.items():
        pos=wait_for("browser Save "+path.name,lambda:button(path.name),20)
        with page.expect_download() as dl:page.mouse.click(*pos)
        assert dl.value.suggested_filename==path.name
        dest=tmp/("browser-saved-"+name);dl.value.save_as(dest)
        assert dest.read_bytes()==original[name]
    assert len(events())==len(uploads)==4
    print(f"Browser after reload: Save downloaded Element’s returned {len(original['Map.png'])}-byte PNG "
          "and exact-limit 65,536-byte PDF byte-for-byte.",flush=True)
