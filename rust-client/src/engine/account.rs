//! Recovery setup and reset, and the account's device list.
//!
//! Homeservers ask for the account password (user-interactive auth) before
//! replacing cross-signing keys or deleting a device. The first attempt goes
//! out without it, so servers that allow the first cross-signing upload
//! freely (MSC3967) never see the password.
use super::{DeviceInfo, Failure};
use matrix_sdk::{
    Client,
    encryption::{
        CrossSigningResetAuthType,
        recovery::{RecoveryError, RecoveryState},
    },
    ruma::{
        OwnedDeviceId,
        api::client::uiaa::{AuthData, MatrixUserIdentifier, Password, UiaaInfo, UserIdentifier},
    },
};

fn password(client: &Client, password: &str, info: &UiaaInfo) -> Result<AuthData, Failure> {
    let user = client.user_id().ok_or(Failure::Unavailable)?;
    let mut auth = Password::new(
        UserIdentifier::Matrix(MatrixUserIdentifier::new(user.to_string())),
        password.to_owned(),
    );
    auth.session = info.session.clone();
    Ok(AuthData::Password(auth))
}

fn sdk(e: &matrix_sdk::Error) -> Failure {
    if e.as_uiaa_response().is_some() {
        Failure::WrongPassword
    } else if e.client_api_error_kind().is_none() && matches!(e, matrix_sdk::Error::Http(_)) {
        Failure::Unreachable
    } else {
        Failure::Failed
    }
}

fn recovery(e: &RecoveryError) -> Failure {
    match e {
        RecoveryError::Sdk(e) => sdk(e),
        _ => Failure::Failed,
    }
}

/// Recovery for an account that has none: cross-signing keys if the account
/// has no identity yet, then a key backup and secret storage. This device
/// ends up cross-signed, because it creates (or already holds) the keys.
pub async fn set_up_recovery(client: &Client, secret: &str) -> Result<String, Failure> {
    let user = client.user_id().ok_or(Failure::Unavailable)?.to_owned();
    let encryption = client.encryption();
    let identity = encryption
        .request_user_identity(&user)
        .await
        .map_err(|e| sdk(&e))?;
    let has_keys = encryption
        .cross_signing_status()
        .await
        .is_some_and(|s| s.has_master && s.has_self_signing && s.has_user_signing);
    if identity.is_none() {
        if let Err(e) = encryption.bootstrap_cross_signing(None).await {
            let info = e.as_uiaa_response().ok_or_else(|| sdk(&e))?;
            let auth = password(client, secret, info)?;
            encryption
                .bootstrap_cross_signing(Some(auth))
                .await
                .map_err(|e| sdk(&e))?;
        }
    } else if !has_keys {
        // The account has keys this device can't reach: verify first.
        return Err(Failure::NoIdentity);
    }
    encryption
        .recovery()
        .enable()
        .wait_for_backups_to_upload()
        .await
        .map_err(|_| Failure::Failed)
}

/// A new recovery key from a verified device, or recovery for an account
/// whose verified devices never set it up.
pub async fn change_recovery_key(client: &Client) -> Result<String, Failure> {
    let recovery = client.encryption().recovery();
    match recovery.state() {
        RecoveryState::Disabled => recovery.enable().wait_for_backups_to_upload().await,
        _ => recovery.reset_key().await,
    }
    .map_err(|_| Failure::Failed)
}

/// Start over with a new identity: other people's devices will show that
/// this account's keys changed, and existing devices need verifying again.
pub async fn reset_encryption(client: &Client, secret: &str) -> Result<String, Failure> {
    let recovery = client.encryption().recovery();
    if let Some(handle) = recovery
        .reset_identity()
        .await
        .map_err(|e| self::recovery(&e))?
    {
        match handle.auth_type() {
            CrossSigningResetAuthType::Uiaa(info) => {
                let auth = password(client, secret, info.auth_info())?;
                handle.reset(auth).await.map_err(|e| self::recovery(&e))?;
            }
            // OAuth 2.0 accounts approve the reset in the browser. Not wired yet.
            CrossSigningResetAuthType::OAuth(_) => {
                handle.cancel().await;
                return Err(Failure::NeedsBrowser);
            }
        }
    }
    recovery
        .enable()
        .wait_for_backups_to_upload()
        .await
        .map_err(|_| Failure::Failed)
}

pub async fn devices(client: &Client) -> Result<Vec<DeviceInfo>, Failure> {
    let user = client.user_id().ok_or(Failure::Unavailable)?.to_owned();
    let own = client.device_id().map(ToOwned::to_owned);
    let listed = client
        .devices()
        .await
        .map_err(|e| {
            if e.client_api_error_kind().is_some() {
                Failure::Failed
            } else {
                Failure::Unreachable
            }
        })?
        .devices;
    let keys = client.encryption().get_user_devices(&user).await.ok();
    let mut devices: Vec<DeviceInfo> = listed
        .into_iter()
        .map(|d| DeviceInfo {
            verified: keys
                .as_ref()
                .and_then(|k| k.get(&d.device_id))
                .is_some_and(|k| k.is_cross_signed_by_owner()),
            current: own.as_ref() == Some(&d.device_id),
            id: d.device_id.to_string(),
            name: d.display_name,
            last_seen: d.last_seen_ts.map(|t| u64::from(t.0) as i64),
        })
        .collect();
    devices.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.id.cmp(&b.id))
    });
    Ok(devices)
}

pub async fn delete_device(client: &Client, device: &str, secret: &str) -> Result<(), Failure> {
    if client.device_id().is_some_and(|own| own.as_str() == device) {
        // Signing out this device is Logout, which also removes local data.
        return Err(Failure::Unavailable);
    }
    let ids: Vec<OwnedDeviceId> = vec![device.into()];
    let http = |e: &matrix_sdk::HttpError| {
        if e.as_uiaa_response().is_some() {
            Failure::WrongPassword
        } else if e.client_api_error_kind().is_some() {
            Failure::Failed
        } else {
            Failure::Unreachable
        }
    };
    match client.delete_devices(&ids, None).await {
        Ok(_) => Ok(()),
        Err(e) => {
            let info = e.as_uiaa_response().ok_or_else(|| http(&e))?;
            let auth = password(client, secret, info)?;
            client
                .delete_devices(&ids, Some(auth))
                .await
                .map(|_| ())
                .map_err(|e| http(&e))
        }
    }
}
