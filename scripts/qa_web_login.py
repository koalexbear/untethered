"""Sign a disposable account into an actual 1180×760 Untethered canvas."""
def gomuks_page(web):
    """The web client on gomuks, which these tests seed (matrix-sdk is the default)."""
    return web.split("?")[0].rstrip("/")+"/?engine=gomuks"
def login_client(page,web,fixture):
    page.goto(gomuks_page(web),wait_until="commit")
    page.wait_for_function("document.title.includes('Preview')",timeout=45000)
    page.mouse.click(1095,73)
    page.wait_for_function("document.title.includes('Account')")
    # Homeserver, username and password fields of the sign-in card.
    for y,field in [(299,"server"),(381,"username"),(464,"password")]:
        page.mouse.click(510,y);page.keyboard.type(fixture[field],delay=6)
    page.keyboard.press("Enter")
    secret=page.get_by_label("Recovery key or passphrase")
    secret.fill(fixture["key"],timeout=45000);secret.press("Enter")
    page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
