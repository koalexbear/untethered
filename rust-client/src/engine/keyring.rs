//! The SQLite store passphrase in the OS keyring: the Secret Service on
//! Linux (GNOME Keyring, KWallet, KeePassXC…), the Credential Manager on
//! Windows. Without a working keyring the passphrase stays in session.json
//! (mode 0600), as in step 1. `UNTETHERED_KEYRING=off` forces the file.
use std::sync::OnceLock;

const SERVICE: &str = "Untethered";

fn enabled() -> bool {
    !matches!(
        std::env::var("UNTETHERED_KEYRING").as_deref(),
        Ok("off" | "0" | "false")
    )
}

/// Install the platform store once. Calls block on D-Bus or Win32, so they
/// only ever run on the blocking pool.
fn ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new();
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(any(target_os = "linux", windows))]
        match store {
            Ok(store) => {
                keyring_core::set_default_store(store);
                true
            }
            Err(_) => false,
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        false
    })
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    if !enabled() {
        return None;
    }
    tokio::task::spawn_blocking(move || if ready() { job() } else { None })
        .await
        .ok()
        .flatten()
}

/// Store a secret; true only if it reads back unchanged.
pub async fn save(account: &str, secret: &str) -> bool {
    let (account, secret) = (account.to_owned(), secret.to_owned());
    blocking(move || {
        let entry = keyring_core::Entry::new(SERVICE, &account).ok()?;
        entry.set_password(&secret).ok()?;
        (entry.get_password().ok()? == secret).then_some(())
    })
    .await
    .is_some()
}

pub async fn load(account: &str) -> Option<String> {
    let account = account.to_owned();
    blocking(move || {
        keyring_core::Entry::new(SERVICE, &account)
            .ok()?
            .get_password()
            .ok()
    })
    .await
}

pub async fn delete(account: &str) {
    let account = account.to_owned();
    blocking(move || {
        keyring_core::Entry::new(SERVICE, &account)
            .ok()?
            .delete_credential()
            .ok()
    })
    .await;
}
