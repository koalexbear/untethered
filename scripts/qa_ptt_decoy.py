"""A focusable Wayland window that logs every key it receives, one per line:
"press <Qt key> <text>". The push-to-talk fixture keeps it focused while it
holds the portal shortcut, then checks the shortcut never arrived here."""
import sys
from PySide6 import QtWidgets

app = QtWidgets.QApplication([])
log = open(sys.argv[1], "a", buffering=1)


class Decoy(QtWidgets.QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("PTT decoy")
        self.resize(400, 200)

    def keyPressEvent(self, event):
        log.write(f"press {event.key():#x} {event.text()!r}\n")

    def keyReleaseEvent(self, event):
        log.write(f"release {event.key():#x}\n")


window = Decoy()
window.show()
sys.exit(app.exec())
