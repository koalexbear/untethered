"""Sign a disposable account into an actual 1180×760 Untethered canvas."""
import io

from PIL import Image

from qa_native_paste import wait_for
from qa_ui import read


def field(page,caption):
    """The input under a sign-in caption (HOMESERVER, USERNAME, PASSWORD). The
    card recentres when it shows a notice, so find the caption rather than a
    fixed position."""
    def find():
        shot=Image.open(io.BytesIO(page.screenshot())).convert("RGB")
        return next(((x,y) for w,x,y in read(shot) if w.strip(".:")==caption),None)
    _,y=wait_for(f"the {caption} field",find,30)
    return 590,y+37


def login_client(page,web,fixture,*,navigate=True):
    """Sign in on the matrix-sdk engine's sign-in card and verify with the
    recovery key. With navigate=False the page already shows the preview."""
    if navigate:page.goto(web,wait_until="commit")
    page.wait_for_function("document.title.includes('Preview')",timeout=45000)
    page.mouse.click(1095,73)
    page.wait_for_function("document.title.includes('Account')")
    for caption,key in [("HOMESERVER","server"),("USERNAME","username"),("PASSWORD","password")]:
        page.mouse.click(*field(page,caption));page.keyboard.type(fixture[key],delay=6)
    page.keyboard.press("Enter")
    secret=page.get_by_label("Recovery key or passphrase")
    secret.fill(fixture["key"],timeout=90000);secret.press("Enter")
    page.wait_for_function("document.title.includes('media-proof')",timeout=90000)
