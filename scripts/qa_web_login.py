"""Sign a disposable account into an actual 1180×760 Untethered canvas."""
import io

from PIL import Image

from qa_native_paste import wait_for
from qa_ui import read


def caption_field(im,caption):
    """The input under a sign-in caption (HOMESERVER, USERNAME, PASSWORD, or
    RECOVERY on the verification card) in a
    1180×760 screenshot of the sign-in card, or None. The card recentres when
    it shows a notice, so this finds the caption rather than a fixed position;
    it matches case, because the subtitle and placeholders repeat the words."""
    at=next(((x,y) for w,x,y in read(im) if w.strip(".:")==caption),None)
    return at and (590,at[1]+37)


def field(page,caption):
    return wait_for(f"the {caption} field",lambda:caption_field(
        Image.open(io.BytesIO(page.screenshot())).convert("RGB"),caption),30)


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
