"""Exercise the real worker and SQLite WASM in an isolated Chromium profile.

Serve the built app, then run: uv run --with playwright profile_handoff.py URL
Set CHROMIUM to use a system Chromium instead of Playwright's browser.
"""
import os
import sys

from playwright.sync_api import sync_playwright

with sync_playwright() as playwright:
    browser = playwright.chromium.launch(
        executable_path=os.environ.get("CHROMIUM"),
        args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
    )
    context = browser.new_context()
    context.add_init_script("""
        window.workerEvents = [];
        window.Worker = class extends Worker {
            constructor(...args) {
                super(...args);
                this.addEventListener('message', e => {
                    const {command, data} = e.data;
                    if (command === 'wasm-connection' && data.connected === false)
                        window.workerEvents.push('stopped');
                    else window.workerEvents.push(command);
                });
            }
        };
    """)

    def wait_for(page, event):
        page.wait_for_function(
            "e => window.workerEvents.includes(e) || window.workerEvents.includes('stopped')",
            arg=event, timeout=30000,
        )
        assert "stopped" not in page.evaluate("window.workerEvents")

    first = context.new_page()
    first.goto(sys.argv[1])
    wait_for(first, "client_state")
    second = context.new_page()
    second.goto(sys.argv[1])
    wait_for(second, "profile-waiting")
    assert "profile-opening" not in second.evaluate("window.workerEvents")
    first.close()
    wait_for(second, "client_state")
    for _ in range(2):
        second.reload()
        wait_for(second, "client_state")
    context.close()
    browser.close()
    print("PASS: second tab waited before opening SQLite, took over, and survived reloads.")
