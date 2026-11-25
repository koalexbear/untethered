//! Profile directory, persisted session, password login and recovery.
//!
//! Layout inside `UNTETHERED_PROFILE` (gomuks keeps its own files beside it):
//! `matrix-sdk/session.json` holds the access token and the store passphrase
//! (mode 0600); `matrix-sdk/store/` holds the SDK's SQLite state, crypto,
//! event cache and media stores, encrypted with that passphrase.
use super::Failure;
use matrix_sdk::{
    Client,
    authentication::matrix::MatrixSession,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::api::error::ErrorKind,
};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

const FORMAT: u64 = 1;

pub struct Profile {
    dir: PathBuf,
    _lock: fs::File,
}

// Same location policy as the gomuks launcher in backend/native.rs.
#[cfg(target_os = "linux")]
fn default_profile() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .map(|p| p.join("untethered-iced"))
}
#[cfg(not(target_os = "linux"))]
fn default_profile() -> Option<PathBuf> {
    None
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    let mut dir = fs::DirBuilder::new();
    dir.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(path)
}

impl Profile {
    pub fn open() -> Result<Self, String> {
        let root = env::var_os("UNTETHERED_PROFILE")
            .map(PathBuf::from)
            .or_else(default_profile)
            .ok_or("Set UNTETHERED_PROFILE to a separate Rust client profile directory.")?;
        private_dir(&root).map_err(|_| "Cannot create the profile directory.")?;
        // Shared with the gomuks launcher: one engine per profile at a time.
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("rust-client.lock"))
            .map_err(|_| "Cannot open the profile lock.")?;
        lock.try_lock()
            .map_err(|_| "This profile is already open.")?;
        let dir = root.join("matrix-sdk");
        private_dir(&dir).map_err(|_| "Cannot create the matrix-sdk profile directory.")?;
        Ok(Self { dir, _lock: lock })
    }
    fn session(&self) -> PathBuf {
        self.dir.join("session.json")
    }
    fn store(&self) -> PathBuf {
        self.dir.join("store")
    }
}

fn settings() -> EncryptionSettings {
    EncryptionSettings {
        // Like the gomuks engine, never create or replace cross-signing keys
        // or backups implicitly. Verification uses the existing recovery.
        auto_enable_cross_signing: false,
        auto_enable_backups: false,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
    }
}

fn passphrase() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Remove the saved session and every SDK store. The device itself stays
/// registered on the homeserver.
pub fn forget(profile: &Profile) {
    let _ = fs::remove_file(profile.session());
    let _ = fs::remove_dir_all(profile.store());
}

fn save(profile: &Profile, client: &Client, passphrase: &str) -> Result<(), Failure> {
    let session = client.matrix_auth().session().ok_or(Failure::Failed)?;
    let value = json!({
        "format": FORMAT,
        "homeserver": client.homeserver().to_string(),
        "passphrase": passphrase,
        "session": serde_json::to_value(session).map_err(|_| Failure::Failed)?,
    });
    // NamedTempFile is created with mode 0600; rename it into place.
    let mut file = tempfile::NamedTempFile::new_in(&profile.dir).map_err(|_| Failure::Failed)?;
    file.write_all(value.to_string().as_bytes())
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| Failure::Failed)?;
    file.persist(profile.session())
        .map_err(|_| Failure::Failed)?;
    Ok(())
}

pub async fn restore(profile: &Profile) -> Result<Option<Client>, &'static str> {
    let Ok(bytes) = fs::read(profile.session()) else {
        // A store without a session belongs to an unfinished login.
        let _ = fs::remove_dir_all(profile.store());
        return Ok(None);
    };
    const UNREADABLE: &str =
        "The saved matrix-sdk session couldn’t be opened. Sign in again to replace it.";
    let saved: Value = serde_json::from_slice(&bytes).map_err(|_| UNREADABLE)?;
    let (Some(FORMAT), Some(homeserver), Some(passphrase)) = (
        saved["format"].as_u64(),
        saved["homeserver"].as_str(),
        saved["passphrase"].as_str(),
    ) else {
        return Err(UNREADABLE);
    };
    let session: MatrixSession =
        serde_json::from_value(saved["session"].clone()).map_err(|_| UNREADABLE)?;
    // No discovery: the saved URL lets the client start offline.
    let client = Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store(profile.store(), Some(passphrase))
        .with_encryption_settings(settings())
        .build()
        .await
        .map_err(|_| UNREADABLE)?;
    client
        .restore_session(session)
        .await
        .map_err(|_| UNREADABLE)?;
    Ok(Some(client))
}

pub async fn login(
    profile: &Profile,
    homeserver: &str,
    username: &str,
    password: &str,
) -> Result<Client, Failure> {
    // A new device needs a new crypto store.
    forget(profile);
    private_dir(&profile.store()).map_err(|_| Failure::Failed)?;
    let passphrase = passphrase();
    let result = async {
        let client = Client::builder()
            .server_name_or_homeserver_url(homeserver)
            .sqlite_store(profile.store(), Some(&passphrase))
            .with_encryption_settings(settings())
            .build()
            .await
            .map_err(|_| Failure::Unreachable)?;
        client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("Untethered")
            .await
            .map_err(|e| match e.client_api_error_kind() {
                Some(
                    ErrorKind::Forbidden | ErrorKind::UserDeactivated | ErrorKind::InvalidUsername,
                ) => Failure::Forbidden,
                Some(_) => Failure::Failed,
                None => Failure::Unreachable,
            })?;
        save(profile, &client, &passphrase)?;
        Ok(client)
    }
    .await;
    if result.is_err() {
        forget(profile);
    }
    result
}

pub async fn recover(client: &Client, secret: &str) -> Result<(), Failure> {
    use matrix_sdk::encryption::{recovery::RecoveryError, secret_storage::SecretStorageError};
    client
        .encryption()
        .recovery()
        .recover(secret)
        .await
        .map_err(|e| match e {
            RecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(_)) => {
                Failure::WrongSecret
            }
            RecoveryError::SecretStorage(SecretStorageError::MissingKeyInfo { .. }) => {
                Failure::NoRecovery
            }
            RecoveryError::SecretStorage(SecretStorageError::Sdk(e)) | RecoveryError::Sdk(e)
                if e.client_api_error_kind().is_none()
                    && matches!(e, matrix_sdk::Error::Http(_)) =>
            {
                Failure::Unreachable
            }
            _ => Failure::Failed,
        })?;
    // Importing the self-signing key signs this device; wait until the
    // signature is in the store so the UI sees a verified device.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        if client
            .encryption()
            .get_own_device()
            .await
            .ok()
            .flatten()
            .is_some_and(|d| d.is_cross_signed_by_owner())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}
