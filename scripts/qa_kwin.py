"""Private, virtual KWin output for GUI acceptance. Never targets the host session."""
import json
import os
import re
import shutil
import subprocess as sp
import time
from pathlib import Path

from PIL import Image
from qa_native_paste import child, isolated_env, wait_for
from qa_ui import CHAT_X, map_fixture, read, regions


class KWin:
    def __init__(self, stack, tmp, root, backend, *, env=None, screencast=False):
        self.tmp = tmp
        self.stack = stack
        self.env = env = env if env is not None else isolated_env(tmp, backend, "wayland")
        self.synth = shutil.which(os.environ.get("KWIN_INPUTSYNTH", "selenium-webdriver-at-spi-inputsynth"))
        if not self.synth:
            raise RuntimeError("Set KWIN_INPUTSYNTH to KDE's installed selenium-webdriver-at-spi-inputsynth")
        for key in ("QT_QPA_PLATFORM", "QT_IM_MODULE", "LIBGL_ALWAYS_SOFTWARE", "KDE_SESSION_UID",
                    "XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID", "XDG_SESSION_ID", "XDG_SEAT", "XDG_VTNR"):
            env.pop(key, None)
        env.update(XDG_CURRENT_DESKTOP="KDE", KDE_SESSION_VERSION="6", WAYLAND_DISPLAY="kwin-proof",
                   KWIN_COMPOSE="O2", KWIN_XKB_DEFAULT_KEYMAP="1", XKB_DEFAULT_LAYOUT="us",
                   LANG="C.UTF-8", LANGUAGE="en", GTK_A11Y="none", GIO_USE_VFS="local",
                   XDG_DATA_DIRS=str(Path(self.synth).resolve().parent.parent/"share")+":/usr/local/share:/usr/share")
        runtime = Path(env["XDG_RUNTIME_DIR"])
        if screencast:
            env.update(PIPEWIRE_RUNTIME_DIR=str(runtime),PIPEWIRE_REMOTE="pipewire-0",
                       XDG_STATE_HOME=str(tmp/"state"),XDG_SESSION_TYPE="wayland",
                       DBUS_SYSTEM_BUS_ADDRESS="unix:path="+str(runtime/"no-system-bus"))
        env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path="+str(runtime/"bus")
        (Path(env["XDG_CONFIG_HOME"])/"kdeglobals").write_text("[KDE]\nAnimationDurationFactor=0\nSingleClick=false\n")
        def log(name):
            dest=os.getenv("KWIN_QA_DEBUG")
            return stack.enter_context((Path(dest)/(name+".log")).open("w")) if dest else sp.DEVNULL
        stack.enter_context(child(["dbus-daemon","--session","--nofork","--nopidfile","--address="+env["DBUS_SESSION_BUS_ADDRESS"]],env,stderr=log("bus")))
        wait_for("private D-Bus",lambda:(runtime/"bus").is_socket())
        if screencast:
            stack.enter_context(child(["pipewire"],env,stderr=log("pw")))
            wait_for("private PipeWire",lambda:(runtime/"pipewire-0").is_socket())
            stack.enter_context(child(["wireplumber","--profile","policy"],env,stderr=log("wp")))
        self.kw = stack.enter_context(child(["kwin_wayland","--virtual","--width","2100","--height","1000",
                                            "--socket","kwin-proof","--no-lockscreen","--no-global-shortcuts","--no-kactivities"],env))
        def ready():
            assert self.kw.poll() is None, "isolated KWin could not initialize a virtual output"
            r = self.command("qdbus6","org.kde.KWin","/KWin","org.kde.KWin.supportInformation",check=False)
            return r.stdout if r.returncode==0 and "Compositing is active" in r.stdout and "Operation Mode: Wayland" in r.stdout else None
        info=wait_for("active KWin virtual compositor",ready)
        owner=self.command("gdbus","call","--session","--dest","org.freedesktop.DBus",
                           "--object-path","/org/freedesktop/DBus","--method",
                           "org.freedesktop.DBus.GetConnectionUnixProcessID","org.kde.KWin").stdout
        assert re.search(rf"\b{self.kw.pid}\b",owner), "Refusing to drive a compositor not owned by this test"
        # No XWayland server or DISPLAY is started. Both clients use kwin-proof.
        env["QT_QPA_PLATFORM"]="wayland"
        limit=["taskset","-c",",".join(map(str,sorted(os.sched_getaffinity(0))[:4]))] if screencast else []
        # Only KWin and the portal can reach their ordinary PipeWire socket
        # by name. The tested native client must use OpenPipeWireRemote's FD.
        native_env={**env,"PIPEWIRE_REMOTE":"no-ambient-qa-remote"} if screencast else env
        binary=os.getenv("UNTETHERED_QA_CLIENT") or root/"rust-client/target/debug/untethered-iced"
        self.native=stack.enter_context(child([*limit,binary],native_env,stderr=log("native")))
        print(re.search(r"^KWin version: .+$",info,re.M)[0]+" · active virtual Wayland compositor · private D-Bus",flush=True)

    def command(self,*args,check=True):
        return sp.run(args,env=self.env,capture_output=True,text=True,timeout=12,check=check)

    def window(self,pattern):
        r=self.command("kdotool","search","--title",pattern,check=False)
        return next(iter(r.stdout.splitlines()),None) if r.returncode==0 else None

    def position(self,wid,x,y,w,h):
        self.command("kdotool","windowsize",wid,str(w),str(h))
        # Wait for the client to acknowledge the xdg-shell configure. Moving
        # immediately can reapply the old size on an asynchronous Wayland client.
        wait_for("KWin resize",lambda:f"Geometry: {w}x{h}" in self.command("kdotool","getwindowgeometry",wid).stdout)
        self.command("kdotool","windowmove",wid,str(x),str(y))
        def placed():
            info=self.command("kdotool","getwindowgeometry",wid).stdout
            return f"Position: {x},{y}" in info and f"Geometry: {w}x{h}" in info
        wait_for("requested KWin window geometry",placed)

    def actions(self,kind,actions):
        path=self.tmp/"input-actions.json"
        path.write_text(json.dumps({"actions":[{"type":kind,"id":"qa","actions":actions}]}))
        self.command(self.synth,str(path))

    def click(self,x,y):
        self.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pointerDown","button":0},
                                {"type":"pause","duration":80},{"type":"pointerUp","button":0}])

    def keys(self,*keys):
        self.actions("key",[{"type":"key"+direction,"value":key} for direction,key in keys])

    def select_all(self):
        self.keys(("Down","\ue009"),("Down","a"),("Up","a"),("Up","\ue009"))

    def type(self,s):
        self.keys(*[(d,ch) for ch in s for d in ("Down","Up")])

    def drag(self,x,y,dx,dy):
        self.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pointerDown","button":0},
                                {"type":"pause","duration":180},
                                {"type":"pointerMove","origin":"pointer","x":dx-x,"y":dy-y,"duration":700},
                                {"type":"pause","duration":200},{"type":"pointerUp","button":0}])

    def capture(self):
        path=self.tmp/"kwin.png";path.unlink(missing_ok=True)
        self.command("spectacle","-b","-n","-o",str(path))
        if debug:=os.environ.get("KWIN_QA_DEBUG"):
            shutil.copyfile(path,Path(debug)/"kwin-screen.png")
        return Image.open(path).convert("RGB")

    def words(self,rect,psm=11):
        # The shared reader also keeps white labels on filled buttons.
        x,y,_,_=rect
        return [(w,x+wx,y+wy) for w,wx,wy in read(self.capture().crop(rect),psm=psm,dialogs=False)]

    def save_button(self,filename):
        """Where to click to save a received attachment: a file card's name, or
        the Download control that appears over a hovered inline image."""
        hit=next(((x,y) for t,x,y in self.words((CHAT_X,76,1178,940))
                  if t.lower().strip(".,:|")==filename.lower()),None)
        if hit or not filename.lower().endswith(".png"):return hit
        boxes=regions(self.capture(),(CHAT_X,76,1178,930),map_fixture)
        if not boxes:return None
        left,top,right,bottom=max(boxes,key=lambda b:b[3])  # the newest image
        self.actions("pointer",[{"type":"pointerMove","x":(left+right)//2,"y":(top+bottom)//2}])
        time.sleep(.5)
        return right-24,top+24
