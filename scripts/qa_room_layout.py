"""Drive Alice's rendered grid/focus controls with two independent portal publishers."""
import io
import json
import re
import subprocess as sp

from PIL import Image, ImageChops
from qa_desktop_dm import sfu_participants
from qa_native_paste import wait_for
from qa_room_camera import camera_source, present, start_camera


def pattern_box(im,pair):
    # Find two filled side-by-side fields. Count their actual occupied pixels
    # so a caption, control, or stray matching pixel cannot establish tile size.
    fields=[]
    for color in pair:
        channels=[c.point([255 if abs(v-target)<29 else 0 for v in range(256)])
                  for c,target in zip(im.split(),color)]
        mask=ImageChops.darker(ImageChops.darker(*channels[:2]),channels[2])
        fields.append(mask.tobytes())
    # Name badges sit over the bottom edge, and controls may share a
    # palette color. Require two solid, adjacent fields across >=45 rows
    # at the same coordinates rather than bounding every matching pixel.
    w=im.width;run=re.compile(b"\xff{32,}");bands={}
    for y in range(0,im.height,3):
        rows=[[m.span() for m in run.finditer(data[y*w:(y+1)*w])] for data in fields]
        for l,m in rows[0]:
            for n,r in rows[1]:
                if abs(n-m)>max(6,(r-l)//55) or abs((r-n)-(m-l))>8:continue
                key=l,r
                if key not in bands or bands[key][1]<y-3:bands[key]=[y,y]
                else:bands[key][1]=y
                if y-bands[key][0]>=45:
                    # Complete this uninterrupted band for accurate height.
                    x1=(l+m)//2;x2=(n+r)//2;end=y+1
                    while end<im.height and fields[0][end*w+x1] and fields[1][end*w+x2]:end+=1
                    return l,bands[key][0],r,end
    return None


def check_layout(stack,desks,users,secret,original_mics,publications):
    alice,*publishers=desks
    pairs=[
        [((31,215,123),(220,40,178)),((244,117,24),(40,83,242))],
        [((27,212,236),(236,218,26)),((223,43,47),(155,97,229))],
    ]
    palettes=[p+[p[0][::-1],p[1][::-1]] for p in pairs]
    def videos():
        tracks={}
        for p in sfu_participants(secret):
            for t in p.get("tracks",[]):
                if t.get("type")!="VIDEO":continue
                uid=p["identity"].rsplit(":",1)[0]
                assert t["source"]=="CAMERA" and t["encryption"]=="GCM" and uid not in tracks
                # Simulcast (V2): three layers, so Alice's thumbnail costs little.
                layers=sorted((l.get("width",0),l.get("height",0)) for l in t.get("layers",[]))
                assert t.get("simulcast") and layers==[(320,180),(640,360),(1280,720)],t
                tracks[uid]=t["sid"]
        return tracks
    updates=[]
    assert len({p.kde.env["PIPEWIRE_RUNTIME_DIR"] for p in publishers})==2
    for peer,user,palette in zip(publishers,users[1:],palettes):
        assert not present(peer.kde)
        updates.append(camera_source(stack,peer.kde,palette))
        start_camera(peer,lambda:user in videos())
    original=videos()
    assert set(original)==set(users[1:]) and len(set(original.values()))==2

    def screen():
        data=sp.check_output(["import","-window",alice.win,"png:-"],env=alice.env,timeout=6)
        im=Image.open(io.BytesIO(data)).convert("RGB")
        # Mask the room sidebar, which has independent speaking indicators.
        im.paste((0,0,0),(0,0,313,im.height))
        return im
    def state(form,phase):
        def matches():
            im=screen()
            boxes=[pattern_box(im,p[phase]) for p in palettes]
            if not all(boxes):return None
            a,b=boxes
            wa,wb=a[2]-a[0],b[2]-b[0]
            if form=="grid":
                if min(wa,wb)<240 or abs(wa-wb)>6 or abs(a[1]-b[1])>4:return None
                if not (a[2]<b[0] or b[2]<a[0]):return None
            else:
                big,small=(a,b) if form=="Bruno" else (b,a)
                if big[2]-big[0]<440 or small[2]-small[0]>162 or big[3]>=small[1]:return None
            return boxes
        boxes=wait_for(f"Alice renders {form}, source patterns {phase}",matches,20)
        assert videos()==original and publications(3)==original_mics
        print(f"Alice {form} / pattern {phase}: "+", ".join(
            f"{name} {r-l}×{b-t}" for name,(l,t,r,b) in zip(("Bruno","Clara"),boxes)),flush=True)
        return boxes
    def show(phase):
        for update in updates:update(phase)
    def click(box):
        l,t,r,b=box
        alice.cmd("xdotool","mousemove","--window",alice.win,str((l+r)//2),str((t+b)//2),"click","1")

    # Calls start docked in the sidebar; Alice watches on the stage.
    alice.open_stage()
    grid=state("grid",0)
    im=screen()
    for name,(l,t,r,b) in zip(("bruno","clara"),grid):
        # Read the name chip itself; full-screen OCR skips chips over video.
        chip=im.crop((l-2,b-58,l+140,b+85))
        ch=[c.point([255 if abs(v-target)<=1 else 0 for v in range(256)]) for c,target in zip(chip.split(),(32,33,37))]
        bounds=ImageChops.darker(ImageChops.darker(*ch[:2]),ch[2]).getbbox()
        assert bounds,"missing participant badge"
        buf=io.BytesIO();chip.crop(bounds).resize(((bounds[2]-bounds[0])*4,(bounds[3]-bounds[1])*4)).save(buf,"PNG")
        got=sp.run(["tesseract","stdin","stdout","--psm","7"],input=buf.getvalue(),capture_output=True).stdout.decode()
        assert name in got.lower(),(name,got)
    for d in desks:d.audit=[]
    show(1)
    grid=state("grid",1)
    click(grid[0])
    state("Bruno",1)
    show(2)
    _,thumbnail=state("Bruno",2)
    click(thumbnail)
    state("Clara",2)
    show(3)
    state("Clara",3)
    alice.click("grid")
    grid=state("grid",3)
    # Re-select Clara and withdraw her publication. Focus must fall back to
    # the grid without losing Bruno's still-live tile.
    click(grid[1])
    state("Clara",3)
    for i,d in enumerate(desks):
        audit=d.audit
        d.audit=None
        assert len(audit)>=12
        assert all(all(amp>500 if j!=i and j<3 else amp<200 for j,amp in enumerate(frame))
                   for _,frame in audit),f"audio interrupted at client {i+1}"
        print(f"Client {i+1} heard both peers in {len(audit)}/{len(audit)} consecutive 100 ms windows across layout switches.",flush=True)
    publishers[1].click("leave")
    pair={u:original_mics[u] for u in users[:2]}
    wait_for("Clara's camera and microphone leave the SFU",lambda:videos()=={users[1]:original[users[1]]} and publications(2)==pair,15)
    def reader_gone(peer):
        kde=peer.kde
        graph=json.loads(kde.command("pw-dump").stdout)
        camera=next(n["id"] for n in graph if n.get("type")=="PipeWire:Interface:Node"
                    and n["info"]["props"].get("node.name")=="qa-room-camera")
        return (not any(n.get("type")=="PipeWire:Interface:Link" and n["info"]["output-node-id"]==camera for n in graph)
                and kde.command("pgrep","-P",str(kde.native.pid),"-x","gst-launch-1.0",check=False).returncode==1)
    wait_for("Clara's camera capture released on leave",lambda:reader_gone(publishers[1]),12)
    alice.tone=3;publishers[0].tone=4
    wait_for("fresh pair audio after departure",lambda:all(d.hears(h) for d,h in zip(desks,({830},{530},set()))),12)
    updates[0](0)
    def fallback():
        im=screen()
        return ((r:=pattern_box(im,palettes[0][0])) and 200<r[2]-r[0]<500 and not pattern_box(im,palettes[1][3]) and
                "focus" not in {w for w,x,_ in alice.words() if x>312} and
                videos()=={users[1]:original[users[1]]})
    wait_for("focused departure restores live grid",fallback,15)
    publishers[0].click("stop")
    wait_for("last capture and tile cleared",lambda:not videos() and not pattern_box(screen(),palettes[0][0]) and reader_gone(publishers[0]),15)
    assert publications(2)==pair
    print("Clara left while focused: capture released; Alice's grid rendered fresh Bruno pixels on his original track. "
          "Remaining pair decoded new 830/530 Hz tones; last camera off cleared the wall.",flush=True)
