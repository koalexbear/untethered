// The matrix-sdk engine's browser storage lock. Like gomuks's profile lock,
// one tab (or worker) per origin owns the IndexedDB stores; the lock is held
// until it closes. Nothing here touches the DOM, so workers can import it.
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
