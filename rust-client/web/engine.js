// Browser helpers for the matrix-sdk engine (src/engine/web.rs, session.rs).
// Nothing here touches the DOM, so the engine's worker can import it.

// gomuks's browser database is SQLite in the origin private file system,
// which the matrix-sdk engine doesn't use: any entry there is gomuks's.
export async function opfs_in_use() {
    try {
        const root = await globalThis.navigator?.storage?.getDirectory?.();
        if (!root) return false;
        for await (const _ of root.keys()) return true;
    } catch {}
    return false;
}

// Like gomuks's profile lock, one tab per origin owns the IndexedDB stores;
// the lock is held until the page closes.
export function engine_lock(name, waiting) {
    return new Promise((granted, refused) => {
        const locks = globalThis.navigator?.locks;
        if (!locks) {
            refused(new Error("Web Locks API unavailable"));
            return;
        }
        const hold = () => {
            granted();
            return new Promise(() => {});
        };
        locks.request(name, {ifAvailable: true}, lock => {
            if (lock) return hold();
            waiting();
            locks.request(name, hold).catch(refused);
        }).catch(refused);
    });
}
