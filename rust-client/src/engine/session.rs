//! Profile, persisted session, password login and recovery.
//!
//! Desktop: inside `UNTETHERED_PROFILE` (gomuks keeps its own files beside
//! it), `matrix-sdk/session.json` holds the access token (mode 0600) and
//! `matrix-sdk/store/` holds the SDK's SQLite state, crypto, event cache and
//! media stores, encrypted with a random passphrase. The passphrase lives in
//! the OS keyring when one answers, otherwise in session.json.
//!
//! Browser: the page keeps the same session JSON (a worker has no
//! `localStorage`, so it arrives with the start request and every change goes
//! back through `persist`). The stores are IndexedDB databases encrypted with
//! the passphrase in that JSON; each sign-in gets its own database name, so
//! wiping an old store never waits on a connection that is still closing.
use super::{Failure, rt};
use matrix_sdk::{
    Client, ClientBuilder,
    authentication::{
        matrix::MatrixSession,
        oauth::{ClientId, OAuthSession, UserSession},
    },
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::api::error::ErrorKind,
};
use serde_json::{Value, json};
use std::time::Duration;

/// 1: step 1 (Matrix session, passphrase in the file). 2: adds OAuth
/// sessions and the keyring.
const FORMAT: u64 = 2;

const UNREADABLE: &str =
    "The saved matrix-sdk session couldn’t be opened. Sign in again to replace it.";

fn settings() -> EncryptionSettings {
    EncryptionSettings {
        // Like the gomuks engine, never create or replace cross-signing keys
        // or backups implicitly. Verification uses the existing recovery, and
        // recovery setup is an explicit command.
        auto_enable_cross_signing: false,
        auto_enable_backups: false,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
    }
}

fn configured(builder: ClientBuilder) -> ClientBuilder {
    builder
        .with_encryption_settings(settings())
        // OAuth 2.0 (MAS) sessions expire quickly; the SDK refreshes them and
        // `save_tokens` persists the new pair.
        .handle_refresh_tokens()
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    //! Session file, OS keyring and SQLite stores in the profile directory.
    use super::super::{Failure, keyring};
    use matrix_sdk::{Client, ClientBuilder};
    use serde_json::Value;
    use std::{
        env, fs,
        io::Write,
        path::{Path, PathBuf},
    };

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

    pub fn profile_root() -> Result<PathBuf, String> {
        env::var_os("UNTETHERED_PROFILE")
            .map(PathBuf::from)
            .or_else(default_profile)
            .ok_or_else(|| {
                "Set UNTETHERED_PROFILE to a separate Rust client profile directory.".into()
            })
    }

    fn private_dir(path: &Path) -> std::io::Result<()> {
        let mut dir = fs::DirBuilder::new();
        dir.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
        dir.create(path)
    }

    impl Profile {
        pub fn open(root: PathBuf) -> Result<Self, String> {
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
        pub fn has_session(&self) -> bool {
            self.session().exists()
        }
        pub fn read(&self) -> Option<Value> {
            serde_json::from_slice(&fs::read(self.session()).ok()?).ok()
        }
        pub fn write(&self, value: &Value) -> Result<(), Failure> {
            // NamedTempFile is created with mode 0600; rename it into place.
            let mut file =
                tempfile::NamedTempFile::new_in(&self.dir).map_err(|_| Failure::Failed)?;
            file.write_all(value.to_string().as_bytes())
                .and_then(|_| file.as_file().sync_all())
                .map_err(|_| Failure::Failed)?;
            file.persist(self.session()).map_err(|_| Failure::Failed)?;
            Ok(())
        }
        /// The session, its keyring entry and every SDK store.
        pub async fn forget(&self) {
            if let Some(name) = self
                .read()
                .and_then(|v| v["keyring"].as_str().map(str::to_owned))
            {
                keyring::delete(&name).await;
            }
            let _ = fs::remove_file(self.session());
            let _ = fs::remove_dir_all(self.store());
        }
        /// A store without a session belongs to an unfinished login.
        pub async fn forget_stores(&self) {
            let _ = fs::remove_dir_all(self.store());
        }
        pub fn new_store(&self) -> Result<(), Failure> {
            private_dir(&self.store()).map_err(|_| Failure::Failed)
        }
        pub fn builder(&self, passphrase: &str) -> ClientBuilder {
            Client::builder().sqlite_store(self.store(), Some(passphrase))
        }
        /// Where the store passphrase goes: the keyring if one accepts it.
        pub async fn keep_secret(&self, value: &mut Value, client: &Client, secret: &str) {
            let name = format!(
                "matrix-sdk store {} {}",
                client.user_id().map(|u| u.as_str()).unwrap_or_default(),
                client.device_id().map(|d| d.as_str()).unwrap_or_default()
            );
            if keyring::save(&name, secret).await {
                value["keyring"] = name.into();
            } else {
                value["passphrase"] = secret.into();
            }
        }
        pub async fn load_secret(&self, saved: &Value) -> Result<String, &'static str> {
            const NO_KEYRING: &str = "The system keyring didn’t return this profile’s store key. Unlock it and restart, or sign in again.";
            match (saved["passphrase"].as_str(), saved["keyring"].as_str()) {
                (Some(secret), _) => Ok(secret.to_owned()),
                (None, Some(name)) => keyring::load(name).await.ok_or(NO_KEYRING),
                (None, None) => Err(super::UNREADABLE),
            }
        }
        pub fn passphrase() -> String {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    //! The page's copy of the session and IndexedDB stores.
    use super::super::Failure;
    use matrix_sdk::{Client, ClientBuilder};
    use serde_json::Value;
    use std::sync::Mutex;
    use wasm_bindgen::{JsCast, JsValue};

    /// Every IndexedDB database this client creates starts with this.
    const PREFIX: &str = "untethered-sdk-";

    pub struct Profile {
        saved: Mutex<Option<String>>,
        /// The current store's database name prefix.
        store: Mutex<String>,
        persist: fn(Option<&str>),
    }

    impl Profile {
        /// `saved` is the session JSON the page kept; `persist` hands every
        /// change back to it (`None` removes it).
        pub fn web(saved: Option<String>, persist: fn(Option<&str>)) -> Self {
            let store = saved
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .and_then(|v| v["store"].as_str().map(str::to_owned))
                .filter(|s| s.starts_with(PREFIX))
                .unwrap_or_default();
            Self {
                saved: Mutex::new(saved),
                store: Mutex::new(store),
                persist,
            }
        }
        pub fn has_session(&self) -> bool {
            self.saved.lock().is_ok_and(|s| s.is_some())
        }
        pub fn read(&self) -> Option<Value> {
            serde_json::from_str(self.saved.lock().ok()?.as_deref()?).ok()
        }
        pub fn write(&self, value: &Value) -> Result<(), Failure> {
            let mut value = value.clone();
            value["store"] = self
                .store
                .lock()
                .map_err(|_| Failure::Failed)?
                .clone()
                .into();
            let text = value.to_string();
            (self.persist)(Some(&text));
            *self.saved.lock().map_err(|_| Failure::Failed)? = Some(text);
            Ok(())
        }
        pub async fn forget(&self) {
            if let Ok(mut saved) = self.saved.lock() {
                *saved = None;
            }
            (self.persist)(None);
            self.forget_stores().await;
        }
        /// Delete every store this client made, including unfinished logins'.
        /// A database that is still open is deleted once it closes.
        pub async fn forget_stores(&self) {
            let _ = delete_databases().await;
        }
        pub fn new_store(&self) -> Result<(), Failure> {
            let name = format!(
                "{PREFIX}{}",
                Self::passphrase().get(..16).unwrap_or_default()
            );
            *self.store.lock().map_err(|_| Failure::Failed)? = name;
            Ok(())
        }
        pub fn builder(&self, passphrase: &str) -> ClientBuilder {
            let name = self.store.lock().map(|s| s.clone()).unwrap_or_default();
            Client::builder().indexeddb_store(&name, Some(passphrase))
        }
        pub async fn keep_secret(&self, value: &mut Value, _client: &Client, secret: &str) {
            value["passphrase"] = secret.into();
        }
        pub async fn load_secret(&self, saved: &Value) -> Result<String, &'static str> {
            if self.store.lock().map_or(true, |s| s.is_empty()) {
                return Err(super::UNREADABLE);
            }
            saved["passphrase"]
                .as_str()
                .map(str::to_owned)
                .ok_or(super::UNREADABLE)
        }
        pub fn passphrase() -> String {
            let mut bytes = [0u8; 32];
            if getrandom::fill(&mut bytes).is_err() {
                // No randomness, no store: opening it fails visibly instead.
                return String::new();
            }
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
    }

    async fn delete_databases() -> Result<(), JsValue> {
        use js_sys::{Array, Function, Promise, Reflect};
        let factory = Reflect::get(&js_sys::global(), &"indexedDB".into())?;
        let list: Function = Reflect::get(&factory, &"databases".into())?.dyn_into()?;
        let delete: Function = Reflect::get(&factory, &"deleteDatabase".into())?.dyn_into()?;
        let all: Promise = list.call0(&factory)?.dyn_into()?;
        let all: Array = wasm_bindgen_futures::JsFuture::from(all)
            .await?
            .dyn_into()?;
        for info in all.iter() {
            if let Some(name) = Reflect::get(&info, &"name".into())?.as_string()
                && name.starts_with(PREFIX)
            {
                delete.call1(&factory, &name.into())?;
            }
        }
        Ok(())
    }
}

pub use store::Profile;
#[cfg(not(target_arch = "wasm32"))]
pub use store::profile_root;

fn passphrase() -> String {
    Profile::passphrase()
}

/// Remove the saved session, its secret and every SDK store. The device
/// itself stays registered on the homeserver.
pub async fn forget(profile: &Profile) {
    profile.forget().await;
}

/// A fresh, empty store for a new device.
pub async fn fresh_store(profile: &Profile) -> Result<(ClientBuilder, String), Failure> {
    forget(profile).await;
    profile.new_store()?;
    let secret = passphrase();
    if secret.is_empty() {
        return Err(Failure::Failed);
    }
    Ok((configured(profile.builder(&secret)), secret))
}

fn session_value(client: &Client) -> Result<Value, Failure> {
    if let Some(session) = client.oauth().full_session() {
        return Ok(json!({
            "auth": "oauth",
            "client_id": session.client_id.as_str(),
            "session": serde_json::to_value(session.user).map_err(|_| Failure::Failed)?,
        }));
    }
    let session = client.matrix_auth().session().ok_or(Failure::Failed)?;
    Ok(json!({
        "auth": "matrix",
        "session": serde_json::to_value(session).map_err(|_| Failure::Failed)?,
    }))
}

/// Save the session after a sign-in, with the store passphrase in the
/// keyring if one accepts it.
pub async fn save(profile: &Profile, client: &Client, secret: &str) -> Result<(), Failure> {
    let mut value = session_value(client)?;
    value["format"] = FORMAT.into();
    value["homeserver"] = client.homeserver().to_string().into();
    profile.keep_secret(&mut value, client, secret).await;
    profile.write(&value)
}

/// Persist refreshed tokens, keeping where the passphrase lives.
pub fn save_tokens(profile: &Profile, client: &Client) {
    let (Some(mut saved), Ok(fresh)) = (profile.read(), session_value(client)) else {
        return;
    };
    saved["session"] = fresh["session"].clone();
    let _ = profile.write(&saved);
}

pub async fn restore(profile: &Profile) -> Result<Option<Client>, &'static str> {
    if !profile.has_session() {
        // A store without a session belongs to an unfinished login.
        profile.forget_stores().await;
        return Ok(None);
    }
    let saved = profile.read().ok_or(UNREADABLE)?;
    let (Some(1 | 2), Some(homeserver)) = (saved["format"].as_u64(), saved["homeserver"].as_str())
    else {
        return Err(UNREADABLE);
    };
    let secret = profile.load_secret(&saved).await?;
    // No discovery: the saved URL lets the client start offline.
    let client = configured(profile.builder(&secret))
        .homeserver_url(homeserver)
        .build()
        .await
        .map_err(|_| UNREADABLE)?;
    match saved["auth"].as_str() {
        Some("oauth") => {
            let user: UserSession =
                serde_json::from_value(saved["session"].clone()).map_err(|_| UNREADABLE)?;
            let client_id = ClientId::new(saved["client_id"].as_str().ok_or(UNREADABLE)?.into());
            client
                .restore_session(OAuthSession { client_id, user })
                .await
                .map_err(|_| UNREADABLE)?;
        }
        // Format 1 sessions have no "auth" field.
        _ => {
            let session: MatrixSession =
                serde_json::from_value(saved["session"].clone()).map_err(|_| UNREADABLE)?;
            client
                .restore_session(session)
                .await
                .map_err(|_| UNREADABLE)?;
        }
    }
    Ok(Some(client))
}

pub async fn login(
    profile: &Profile,
    homeserver: &str,
    username: &str,
    password: &str,
) -> Result<Client, Failure> {
    let (builder, secret) = fresh_store(profile).await?;
    let result = async {
        let client = builder
            .server_name_or_homeserver_url(homeserver)
            .build()
            .await
            .map_err(|_| Failure::Unreachable)?;
        // SyncService would otherwise retry forever after signing in.
        if client.available_sliding_sync_versions().await.is_empty() {
            return Err(Failure::NoSlidingSync);
        }
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
        save(profile, &client, &secret).await?;
        Ok(client)
    }
    .await;
    if result.is_err() {
        forget(profile).await;
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
    let deadline = rt::Instant::now() + Duration::from_secs(15);
    while rt::Instant::now() < deadline {
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
        rt::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}
