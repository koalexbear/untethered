"""Find Untethered's Discord-style controls in the OCR-driven acceptance tests.

Header, rail and composer actions are icons with tooltips, and a message's
Reply/Edit/Add Reaction/Report controls appear only in a toolbar while its row
is hovered. These helpers hover a candidate, read the tooltip with the
caller's OCR, and return the point to click.

Construct `Ui` with the caller's primitives:
  words()   -> [(word, x, y)] for a fresh screenshot
  hover(p)  -> move the pointer to p without clicking
  image()   -> the same screenshot as a PIL RGB image (optional; enables
               pixel lookups such as the message toolbar)
Points are in the caller's coordinates. Pass `origin` when the window content
does not start at (0, 0), for example below a KWin title bar.
"""
import io
import re
import subprocess as sp
import time

from PIL import ImageChops, ImageOps

RAIL = 72
SIDEBAR = 240
CHAT_X = RAIL + SIDEBAR  # First column of the conversation; sidebar words are left of it.
HEADER = 48
SEARCH = 150  # Header search box; it widens while room search is open.
SEARCH_OPEN = 220
MEMBERS = 240
TOOLBAR_INSET = 16
BORDER = (0x23, 0x24, 0x28)  # Floating toolbar outline.
CHAT = (0x31, 0x33, 0x38)  # Conversation and dialog background.
SIDE = (0x2B, 0x2D, 0x31)  # Sidebar and dialog footer background.
RAIL_BG = (0x1E, 0x1F, 0x22)
GREEN = (0x23, 0xA5, 0x5A)  # Speaking ring and the Add a Space glyph.


def near(color, ref, tolerance):
    return all(abs(a - b) <= tolerance for a, b in zip(color, ref))


def clean(word):
    return re.sub(r"^\W+|\W+$", "", word).lower()


def tesseract(im, psm=11):
    """[(word, x, y)] for `im`, OCR'd at twice its size; centres in `im` pixels."""
    buf = io.BytesIO()
    im.resize((im.width * 2, im.height * 2)).save(buf, "PNG")
    tsv = sp.run(["tesseract", "stdin", "stdout", "--psm", str(psm), "tsv"], input=buf.getvalue(),
                 capture_output=True, check=True).stdout.decode()
    return [(s[11], (int(s[6]) + int(s[8]) // 2) // 2, (int(s[7]) + int(s[9]) // 2) // 2)
            for line in tsv.splitlines() if len(s := line.split("\t")) == 12 and s[0] == "5" and s[11].strip()]


def map_fixture(c):
    """Pixels of the pink and teal halves of the Map.png test image."""
    r, g, b = c
    return r > 209 and g < 54 and 94 < b < 130 or r < 40 and g > 180 and 140 < b < 180


def read(im, origin=(0, 0), psm=11, *, dialogs=True):
    """OCR a screenshot of the client: [(word, x, y)] in `im` pixels.

    Besides the ordinary pass, it reads bright text on its own, which keeps
    labels on filled buttons (blurple, red, green, grey) that a whole-window
    threshold otherwise treats as background, and any open dialog."""
    found, seen = [], set()

    def add(words):
        for w, x, y in words:
            key = (clean(w), x // 8, y // 8)
            if key[0] and key not in seen:
                seen.add(key)
                found.append((w, x, y))

    add(tesseract(im, psm))
    add(tesseract(ImageOps.grayscale(im).point(lambda v: 0 if v > 170 else 255), psm))
    if dialogs and psm == 11:
        add(modal_words(im, origin))
    return found


def _mask(im, color):
    bands = [band.point(lambda v, c=c: 255 if v == c else 0) for band, c in zip(im.split(), color)]
    return ImageChops.darker(ImageChops.darker(bands[0], bands[1]), bands[2])


def modal_words(im, origin=(0, 0)):
    """Words inside an open dialog, or [] when none is open.

    The dimmed backdrop leaves three brightness levels on screen, and a
    whole-window threshold merges the dialog's background with its text.
    Reading the dialog body and footer separately avoids that."""
    ox, oy = origin
    if im.getpixel((ox + 4, im.height - 6)) == RAIL_BG or im.getpixel((ox + 36, oy + 4)) == RAIL_BG:
        return []  # No backdrop over the rail: nothing is dimmed.
    body = _mask(im, CHAT).getbbox()
    if not body or body[2] - body[0] < 200 or body[3] - body[1] < 100:
        return []
    found = []
    regions = [(body, 11)]
    below = _mask(im.crop((body[0], body[3], body[2], min(im.height, body[3] + 120))), SIDE).getbbox()
    if below:
        foot = (body[0], body[3] + below[1], body[2], body[3] + below[3])
        regions += [(foot, 11), (foot, 7)]
    seen = set()
    for (left, top, right, bottom), psm in regions:
        for w, x, y in tesseract(im.crop((left, top, right, bottom)), psm):
            key = (clean(w), (left + x) // 6, (top + y) // 6)
            if key not in seen:
                seen.add(key)
                found.append((w, left + x, top + y))
    return found


def regions(im, box, match, *, min_height=24):
    """Bounding boxes (left, top, right, bottom) of solid areas inside `box`
    whose pixels satisfy match((r, g, b)), such as an inline test image."""
    left, top, right, bottom = box
    boxes = []
    for y in range(top, bottom, 2):
        x = left
        while x < right:
            if not match(im.getpixel((x, y))):
                x += 1
                continue
            start = x
            while x < right and match(im.getpixel((x, y))):
                x += 1
            if x - start < 20:
                continue
            for b in boxes:
                if b[3] >= y - 4 and start < b[2] and x > b[0]:
                    b[0], b[2], b[3] = min(b[0], start), max(b[2], x), y
                    break
            else:
                boxes.append([start, y, x, y])
    return [tuple(b) for b in boxes if b[3] - b[1] >= min_height]


class Ui:
    def __init__(self, words, hover, image=None, *, width=1180, height=760, origin=(0, 0), settle=0.45):
        self.words, self.hover, self.image = words, hover, image
        self.width, self.height = width, height
        self.ox, self.oy = origin
        self.settle = settle

    def at(self, x, y):
        """A window-content point in the caller's coordinates."""
        return (self.ox + x, self.oy + y)

    # Tooltips -------------------------------------------------------------

    def tooltip(self, pos, side):
        """Hover `pos` and return the words of the tooltip shown beside it."""
        self.hover(pos)
        time.sleep(self.settle)
        x, y = pos
        left, top, right, bottom = {
            "above": (x - 160, y - 80, x + 160, y - 8),
            "below": (x - 160, y + 8, x + 160, y + 80),
            "right": (x + 14, y - 16, x + 320, y + 16),
            "left": (x - 320, y - 16, x - 14, y + 16),
        }[side]
        return {clean(w) for w, wx, wy in self.words() if left < wx < right and top < wy < bottom}

    def pick(self, candidates, label, side):
        """The first candidate whose tooltip contains every word of `label`."""
        want = {clean(w) for w in label.split()}
        for pos in candidates:
            if want <= self.tooltip(pos, side):
                return pos
        return None

    # Channel header -------------------------------------------------------

    def header_slots(self, *, signed_in=True):
        """Centres of the header icons from the right, for both search widths."""
        slots = []
        widths = (SEARCH, SEARCH_OPEN) if signed_in else (None,)
        for search in widths:
            right = self.width - 16 - (search + 8 if search else 0)
            for i in range(8):
                p = self.at(right - 16 - 40 * i, HEADER // 2)
                if p not in slots:
                    slots.append(p)
        return slots

    def header(self, label, *, signed_in=True):
        """Point for a header icon such as "Notification Settings",
        "Posting Permissions", "Member List", "Copy Conversation",
        "Start Voice Call" or "Start Video Call"."""
        return self.pick(self.header_slots(signed_in=signed_in), label, "below")

    def search_box(self):
        """The header search box (it shows the word "Search")."""
        return self.at(self.width - 16 - SEARCH // 2, HEADER // 2)

    # Message toolbar ------------------------------------------------------

    def toolbar(self, row, *, members=False):
        """Hover a message row (any point on its text) and return the centres
        of its toolbar buttons, left to right. Requires `image`."""
        self.hover(row)
        time.sleep(self.settle)
        im = self.image()
        rights = [self.width - TOOLBAR_INSET, self.width - MEMBERS - TOOLBAR_INSET]
        if members:
            rights.reverse()
        for right in rights:
            for edge in (right - 1, right - 2):
                x = self.ox + edge
                top = max(self.oy + HEADER, row[1] - 110)
                bottom = min(self.oy + self.height - 1, row[1] + 40)
                run = []
                for y in range(top, bottom):
                    if near(im.getpixel((x, y)), BORDER, 6):
                        run.append(y)
                    elif len(run) >= 18:
                        break
                    else:
                        run = []
                if len(run) < 18:
                    continue
                cy = (run[0] + run[-1]) // 2 + 1
                left = next((xx for xx in range(x - 3, x - 200, -1) if near(im.getpixel((xx, cy)), BORDER, 6)), None)
                if left is None:
                    continue
                n = round((x - left - 3) / 32)
                if n < 1:
                    continue
                return [(left + 18 + 32 * i, cy) for i in range(n)]
        return []

    def message(self, row, label, *, members=False):
        """Point for a hovered message's "Add Reaction", "Reply", "Edit" or
        "Report Message" control. `row` is a point on the message text."""
        buttons = self.toolbar(row, members=members)
        if label.lower() in ("edit", "report message", "report"):
            buttons = buttons[::-1]  # The last control when present.
        return self.pick(buttons, label, "above")

    # Sidebar, rail and composer -------------------------------------------

    def space_menu(self, label, click):
        """Open the space dropdown from its name and return `label`'s point:
        "Invite People", "Create Channel" or "Notification Settings"."""
        first = clean(label.split()[0])

        def item():
            return next(((x, y) for w, x, y in self.words()
                         if clean(w) == first and self.ox + RAIL < x < self.ox + CHAT_X
                         and self.oy + HEADER < y < self.oy + HEADER + 200), None)

        # Clicking the name again would close a menu that is already open.
        if hit := item():
            return hit
        click(self.at(RAIL + 80, HEADER // 2))
        for _ in range(20):
            time.sleep(0.3)
            if hit := item():
                return hit
        return None

    def add_space(self):
        """The green Add a Space button at the end of the server rail."""
        if self.image:
            im = self.image()
            ys = [y for y in range(self.oy + 60, self.oy + self.height - 60)
                  if any(near(im.getpixel((self.ox + x, y)), GREEN, 30) for x in range(26, 46))]
            if ys:
                return self.at(36, (ys[0] + ys[-1]) // 2 - self.oy)
        return self.pick([self.at(36, 158 + 56 * i) for i in range(12)], "add a space", "right")

    def settings(self):
        """Your name and status at the bottom left; opens User Settings."""
        return self.at(150, self.height - 26)

    def composer(self):
        """A point in the message field."""
        return self.at(CHAT_X + 200, self.height - 52)

    def attach(self):
        """The + button that opens the file picker."""
        return self.at(CHAT_X + 16 + 16 + 12, self.height - 52)

    def send(self):
        """The send arrow at the right of the composer."""
        return self.at(self.width - 40, self.height - 52)

    def cancel_reply(self, replying_y):
        """The × at the end of the "Replying to" bar, given that bar's y."""
        return (self.ox + self.width - 40, replying_y)

    def speaking(self, im, name_y, *, minimum=40):
        """Whether a voice participant row, identified by its name's y, has the
        green speaking ring around its avatar."""
        hits = 0
        for y in range(name_y - 16, name_y + 17):
            for x in range(RAIL + 30, RAIL + 72):
                if near(im.getpixel((self.ox + x, y)), GREEN, 32):
                    hits += 1
        return hits >= minimum

    def picture_download(self, box):
        """Hover an inline image (left, top, right, bottom) and return its
        Download control at the top right."""
        left, top, right, bottom = box
        self.hover(((left + right) // 2, (top + bottom) // 2))
        time.sleep(self.settle)
        return (right - 24, top + 24)
