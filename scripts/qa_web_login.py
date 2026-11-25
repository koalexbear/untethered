"""Sign a disposable account into an actual 1180×760 Untethered canvas."""
def login_client(page,web,fixture):
    page.goto(web,wait_until="commit")
    page.wait_for_function("document.title.includes('Preview')",timeout=45000)
    page.mouse.click(1095,73)
    page.wait_for_function("document.title.includes('Account')")
    for y,field in [(333,"server"),(391,"username"),(447,"password")]:
        page.mouse.click(510,y);page.keyboard.type(fixture[field],delay=6)
    page.keyboard.press("Enter")
    secret=page.get_by_label("Recovery key or passphrase")
    secret.fill(fixture["key"],timeout=45000);secret.press("Enter")
    page.wait_for_function("document.title.includes('media-proof')",timeout=45000)
