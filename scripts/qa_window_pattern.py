"""Two real Wayland windows on the fixture compositor. Change only the selected one."""
import sys
from PySide6 import QtCore, QtGui, QtWidgets

app=QtWidgets.QApplication([])
class Pattern(QtWidgets.QWidget):
    def __init__(self,title,colors):
        super().__init__();self.setWindowTitle(title);self.resize(480,270);self.colors=colors
    def paintEvent(self,_event):
        p=QtGui.QPainter(self)
        if self.colors is None:
            p.fillRect(self.rect(),QtGui.QColor("#16212e"))
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
        for i,c in enumerate(self.colors):p.fillRect(i*240,0,240,270,QtGui.QColor(c))
        p.end()
target=Pattern("Prism",["#eb2477","#0ecc99"])
other=Pattern("Decoy",["#ffff00","#ffff00"])
target.show();other.show()
def change():
    cmd=sys.stdin.readline().strip()
    if cmd:
        target.colors=None if cmd=="board" else ["#184fea","#f58421"];target.update()
notifier=QtCore.QSocketNotifier(0,QtCore.QSocketNotifier.Type.Read)
notifier.activated.connect(change)
sys.exit(app.exec())
