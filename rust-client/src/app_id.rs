//! The Linux client's application ID: the desktop file's name, the window's
//! Wayland app_id and X11 WM_CLASS, and the ID XDG desktop portals know us by.
//!
//! A host (unsandboxed) app has no app ID a portal can read, so
//! xdg-desktop-portal guesses one from the launcher's systemd scope: dialogs
//! then name the terminal or launcher, and stored grants and GlobalShortcuts
//! bindings follow it. Registering our ID fixes both, but the portal accepts it
//! only while `<APP_ID>.desktop` is installed with an `Exec` found on PATH (a
//! packaged build), and only as the D-Bus connection's first portal call.
//! ashpd, and rfd through it, shares one connection, so every portal user
//! awaits [`portal_ready`] before its first call.

pub const APP_ID: &str = "io.github.koalexbear.Untethered";

/// Registers [`APP_ID`] with the portal once per process. Unregistered (a run
/// from the checkout, or a portal older than 1.19), portals keep guessing.
pub async fn portal_ready() {
    static REGISTERED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    REGISTERED
        .get_or_init(|| async {
            let id = ashpd::AppID::try_from(APP_ID).expect("APP_ID is a valid app ID");
            if let Err(e) = ashpd::register_host_app(id).await {
                eprintln!("Portals will not know this app as {APP_ID}: {e}");
            }
        })
        .await;
}
