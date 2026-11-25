// Page notifications live only as long as the open browser app. gomuks
// owns sync/decryption and evaluates the account's actual Matrix push rules.
let openRoom, deliveryFailed;
export function install_alerts(open, failed) { openRoom = open; deliveryFailed = failed; }
export function permission() {
    return window.isSecureContext && typeof window.Notification === "function"
        ? Notification.permission : "unavailable";
}
export function request() {
    return permission() === "default" ? Notification.requestPermission() : Promise.resolve(permission());
}
export function show(user, room, event, title, body, silent) {
    const n = new Notification(title, {body, silent, tag: `untethered:${user}:${event}`});
    n.onerror = () => deliveryFailed?.();
    n.onclick = () => {
        window.focus();
        openRoom?.(user, room);
        n.close();
    };
}
