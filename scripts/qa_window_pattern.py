"""Two real Wayland windows on the fixture compositor. Change only the selected one.

Prism is 1598x900 so a 1080p-capable share keeps its full width, and a width
that isn't a multiple of 8 makes GStreamer pad every I420 row. Two colour
fields fill the top; a bar sweeps the bottom band at 60 Hz so KWin's screencast
delivers new frames continuously (an idle window yields no PipeWire buffers).
"""
import sys
import time
from PySide6 import QtCore, QtGui, QtWidgets

app=QtWidgets.QApplication([])
BAND=120
class Pattern(QtWidgets.QWidget):
    def __init__(self,title,colors,size,moving=False):
        super().__init__();self.setWindowTitle(title);self.resize(*size);self.colors=colors
        self.start=time.monotonic()
        if moving:
            self.timer=QtCore.QTimer(self);self.timer.setTimerType(QtCore.Qt.TimerType.PreciseTimer)
            self.timer.timeout.connect(self.update);self.timer.start(16)
    def paintEvent(self,_event):
        p=QtGui.QPainter(self)
        w,h=self.width(),self.height()
        if self.colors is None:
            p.fillRect(self.rect(),QtGui.QColor("#16212e"))
            p.scale(w/480,h/270)
            def text(x,y,s,size,color,bold=False):
                f=QtGui.QFont("Sans Serif");f.setPixelSize(size);f.setBold(bold)
                p.setFont(f);p.setPen(QtGui.QColor(color));p.drawText(x,y,s)
            text(24,30,"STUDIO   /   PLAYTEST",12,"#84d9ca",True)
            text(24,71,"Playtest notes",32,"#f2f4fb",True)
            text(24,98,"Saturday session  ·  Round 3",14,"#a8bbcf")
            for i,(title,lines,tint) in enumerate([
                ("Working",["Lobby invites","Audio check"],"#76ceb4"),
                ("To try",["Camera swap","Screen share"],"#d1b0ef"),
                ("Up next",["Team rooms","Next build"],"#e8be81")]):
                x=24+i*148
                p.fillRect(x,126,136,115,QtGui.QColor("#243345"))
                p.fillRect(x,126,136,3,QtGui.QColor(tint));text(x+10,156,title,16,tint,True)
                for j,line in enumerate(lines):text(x+10,189+23*j,line,13,"#dfe9f3")
            p.end();return
        band=BAND if h>=600 else 0
        n=len(self.colors)
        for i,c in enumerate(self.colors):p.fillRect(i*w//n,0,w//n+1,h-band,QtGui.QColor(c))
        if band:
            p.fillRect(0,h-band,w,band,QtGui.QColor("#101010"))
            x=int((time.monotonic()-self.start)*900)%(w+80)-80
            p.fillRect(x,h-band+20,80,band-40,QtGui.QColor("#f0f0f0"))
        p.end()
target=Pattern("Prism",["#eb2477","#0ecc99"],(1598,900),moving=True)
other=Pattern("Decoy",["#ffff00","#ffff00"],(480,270))
target.show();other.show()
def change():
    cmd=sys.stdin.readline().strip()
    if cmd:
        target.colors=None if cmd=="board" else ["#184fea","#f58421"];target.update()
notifier=QtCore.QSocketNotifier(0,QtCore.QSocketNotifier.Type.Read)
notifier.activated.connect(change)
sys.exit(app.exec())
