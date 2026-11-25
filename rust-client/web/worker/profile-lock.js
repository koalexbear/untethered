// The upstream SQLite OPFS pool is exclusive to one worker per origin.
// Hold this lock for the worker's lifetime, not just while opening the DB.
export async function withProfileLock(start) {
    // The pinned worker already exposes authenticated/decrypting media reads.
    // Return explicitly requested downloads to the active tab, without a
    // service worker caching plaintext attachments after the tab closes.
    self.addEventListener("message", event => {
        if (event.data?.command !== "untethered-download") return;
        event.stopImmediatePropagation();
        const port=event.ports[0];
        if (!port) return;
        const fail=()=>{port.postMessage(null);port.close();};
        try {
            const query = new URLSearchParams({encrypted: String(Boolean(event.data.encrypted))});
            if (event.data.limit) query.set("untethered_preview", "true");
            self.meowDownloadMedia(event.data.path,query.toString(),{
                resolve: ({buffer})=>{port.postMessage(buffer,[buffer.buffer]);port.close();},
                reject: fail,
            });
        } catch { fail(); }
    });
    const locks = navigator.locks;
    if (!locks) throw new Error("Web Locks API unavailable");
    const name = "untethered:gomuks-opfs-profile";
    const run = async () => {
        self.postMessage({command: "profile-opening"});
        await start();
    };
    const acquired = await locks.request(name, {ifAvailable: true}, async lock => {
        if (!lock) return false;
        await run();
        return true;
    });
    if (!acquired) {
        self.postMessage({command: "profile-waiting"});
        await locks.request(name, run);
    }
}
