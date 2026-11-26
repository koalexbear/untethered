"""Measure local mixer changes at all three clients' actual speaker monitors."""
from statistics import median

from qa_native_paste import wait_for
from qa_ui import CHAT_X

# The mixer is in the call dock at the bottom of the sidebar. Each peer has a
# line with its name, gain and Reset button, and its slider on the line below:
# the slider's centre is this far below the name. Arrow keys move a slider
# only while the pointer is over it.
SLIDER_BELOW = 24


def check_volume(desks):
    alice,_,_=desks
    alice.click("mixer")

    def row(name):
        words=[(w,x,y) for w,x,y in alice.words() if x<CHAT_X]
        for w,_,y in words:
            if w!=name:continue
            line={v:x for v,x,yy in words if abs(yy-y)<=5}
            if "reset" in line:
                for value,x in line.items():
                    if value.isdecimal():
                        return int(value),(x,y+SLIDER_BELOW),(line["reset"],y)
        return None

    def at(name,percent):
        return (r:=row(name)) and r[0]==percent and r

    def set_gain(name,old,new,reset=False):
        _,slider,button=wait_for(f"{name} slider at {old}%",lambda:at(name,old),8)
        x,y=button if reset else slider
        alice.cmd("xdotool","windowfocus",alice.win,"mousemove","--window",alice.win,str(x),str(y))
        if reset:alice.cmd("xdotool","click","1")
        else:alice.cmd("xdotool","key","--repeat",str(abs(new-old)//5),"--delay","65",
                       "Down" if new<old else "Up")
        wait_for(f"{name} slider at {new}%",lambda:at(name,new),8)

    def fresh():
        for d in desks:d.windows.clear()

    def levels(windows):
        return [[median(frame[j] for frame in frames) for j in range(3)] for frames in windows]

    wait_for("both peers at 100% in Alice's mixer",lambda:at("bruno",100) and at("clara",100),8)
    fresh()
    def baseline():
        windows=[list(d.windows) for d in desks]
        if all(len(frames)==12 and all(all(amp<200 if i==j else 6000<amp<8000
               for j,amp in enumerate(f[:3])) for f in frames) for i,frames in enumerate(windows)):
            return levels(windows)
    reference=wait_for("unattenuated six-path reference",baseline,12)

    def measure(bruno,clara):
        fresh()  # No pre-adjustment samples in any receiver's measurement.
        def steady():
            windows=[list(d.windows) for d in desks]
            for i,frames in enumerate(windows):
                if len(frames)!=12:return None
                for f in frames:
                    for j,amp in enumerate(f):
                        gain=([0,bruno,clara][j] if i==0 and j<3 else 100)/100
                        if j>=3 or i==j or gain==0:
                            if amp>160:return None
                        elif abs(amp/reference[i][j]-gain)>max(.02,.05*gain):
                            return None
            return levels(windows)
        measured=wait_for(f"six independent levels with Alice's Bruno/Clara at {bruno}%/{clara}%",steady,12)
        def pair(i,a,b):return f"{measured[i][a]:.0f}/{measured[i][b]:.0f}"
        print(f"Alice's mixer {bruno}%/{clara}%: Alice hears Bruno/Clara {pair(0,1,2)}, "
              f"Bruno hears Alice/Clara {pair(1,0,2)}, Clara hears Alice/Bruno {pair(2,0,1)} "
              "(median PCM amplitudes; 12 consecutive 100 ms windows checked).",flush=True)

    measure(100,100)
    set_gain("bruno",100,25)
    measure(25,100)
    set_gain("clara",100,150)
    measure(25,150)
    set_gain("clara",150,100,reset=True)
    set_gain("bruno",25,0)
    measure(0,100)
    set_gain("bruno",0,100,reset=True)
    measure(100,100)
    alice.click("mixer")
