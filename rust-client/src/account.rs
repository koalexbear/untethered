use serde_json::Value;
use url::{Host, Url};

pub fn homeserver(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty()
        || input.starts_with(['/', ':'])
        || input.contains(['\\', '@', '?', '#'])
        || input.chars().any(char::is_whitespace)
    {
        return None;
    }
    let raw = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let (_, after) = raw.split_once("://")?;
    if after.is_empty() || after.starts_with('/') {
        return None;
    }
    let url = Url::parse(&raw).ok()?;
    let loopback = match url.host()? {
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
    };
    (url.scheme() == "https" || (url.scheme() == "http" && loopback)).then(|| url.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryFormat {
    Key,
    FreeText,
}

/// A recovery *passphrase* can contain meaningful spaces. Strip copy/paste
/// formatting only when the result is an actual Matrix-format recovery key.
pub fn recovery_input(input: &str) -> (RecoveryFormat, String) {
    let compact: String = input
        .chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{2060}' | '\u{feff}'
                )
        })
        .collect();
    if compact.len() == 48
        && bs58::decode(&compact).into_vec().is_ok_and(|v| {
            v.len() == 35 && v.starts_with(&[0x8b, 0x01]) && v.iter().fold(0u8, |a, b| a ^ b) == 0
        })
    {
        (RecoveryFormat::Key, compact)
    } else {
        (RecoveryFormat::FreeText, input.to_owned())
    }
}

// Give useful diagnostics without ever echoing an arbitrary RPC error, which
// can include identifiers, request data, or server-controlled text.
pub fn verify_error(data: &Value, format: RecoveryFormat) -> &'static str {
    let error = data
        .as_str()
        .or_else(|| data["error"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if error.contains("incorrect ssss key") {
        match format {
            RecoveryFormat::Key => {
                "The recovery key’s format and checksum are valid, but gomuks couldn’t unlock this account’s secret storage with it. Check the Matrix ID above and test the same key in another client."
            }
            RecoveryFormat::FreeText => {
                "This entry isn’t a checksum-valid Matrix recovery key, and it didn’t unlock recovery as a passphrase. Use the full recovery key or the passphrase chosen when recovery was set up, not your sign-in password."
            }
        }
    } else if error.contains("invalid recovery key") {
        "The recovery key format or checksum is invalid. Copy the full key from your trusted client; grouping spaces are fine."
    } else if error.contains("key backup") || error.contains("backup version") {
        "gomuks couldn’t access the account’s key backup. Check that key backup is available in your trusted Matrix client."
    } else if error.contains("no key ids")
        || error.contains("ssss key id")
        || error.contains("missing")
        || error.contains("not found")
    {
        "gomuks couldn’t find all the secrets needed for verification on this homeserver. Check recovery in your trusted Matrix client."
    } else if error.contains("sign own") || error.contains("cross-signing") {
        "gomuks couldn’t complete device cross-signing. Check the device and recovery settings in your trusted Matrix client."
    } else if error.contains("timeout")
        || error.contains("deadline")
        || error.contains("connection")
    {
        "gomuks couldn’t reach the homeserver to finish verification. Try again when it’s reachable."
    } else {
        "gomuks could not verify this device. Check that this is the current key or recovery passphrase for this account."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Msg};
    use serde_json::json;
    #[test]
    fn repeated_unverified_state_cannot_erase_error_or_queue_duplicate_verify() {
        let (tx, rx) = async_channel::bounded(4);
        let mut app = App::new(tx);
        let state = json!({"command":"client_state","data":{"is_logged_in":true,"is_verified":false,
            "verification_state":{"state_checked":true,"has_ssss":true}}});
        app.frame(state.clone());
        app.recovery = "example".into();
        app.submit_verification();
        app.submit_verification();
        let sent = rx.try_recv().unwrap().into_rpc();
        assert!(rx.try_recv().is_err());
        app.frame(state.clone());
        assert!(app.verifying());
        app.frame(
            json!({"command":"error","request_id":sent["request_id"],"data":"incorrect SSSS key"}),
        );
        app.frame(state);
        assert!(!app.verifying());
        assert!(app.verification_feedback().contains("as a passphrase"));
        app.submit_verification();
        assert!(rx.try_recv().is_ok());
    }
    #[test]
    fn login_sends_the_normalized_url() {
        let (tx, rx) = async_channel::bounded(4);
        let mut app = App::new(tx);
        app.connected = true;
        app.server = "example.org:8448".into();
        app.username = "user".into();
        app.password = "test".into();
        let _ = app.update(Msg::Login);
        assert_eq!(
            rx.try_recv().unwrap().into_rpc()["data"]["homeserver_url"],
            "https://example.org:8448/"
        );
    }
    #[test]
    fn normalize_homeservers_before_login() {
        for (input, want) in [
            (" matrix.example.org ", "https://matrix.example.org/"),
            (
                "matrix.example.org:8448/hs",
                "https://matrix.example.org:8448/hs",
            ),
            (
                "https://matrix.example.org/hs",
                "https://matrix.example.org/hs",
            ),
            ("[::1]:8448", "https://[::1]:8448/"),
            ("http://127.0.0.1:8008", "http://127.0.0.1:8008/"),
        ] {
            assert_eq!(homeserver(input).as_deref(), Some(want));
        }
        for input in [
            "",
            "https://",
            "https:///example.org",
            "file://example.org",
            "http://matrix.org",
            "bob:pass@example.org",
            "//example.org",
            "https://a.org?query",
        ] {
            assert!(homeserver(input).is_none());
        }
    }
    #[test]
    fn only_remove_separators_from_real_keys() {
        // Public test vector from mautrix-go's crypto/utils tests.
        let key = "EsTL 2cTx 9Qy1 8TVd qGsn GDrD i5dT EEuX Qz8U P7hi Z7uu U8wZ";
        for sep in [" ", "\t", "\r\n", "\u{a0}", "\u{202f}", "\u{200b}"] {
            assert_eq!(
                recovery_input(&key.replace(' ', sep)),
                (RecoveryFormat::Key, key.replace(' ', ""))
            );
        }
        assert_eq!(
            recovery_input(" a  pass phrase "),
            (RecoveryFormat::FreeText, " a  pass phrase ".into())
        );
    }

    #[test]
    fn well_formed_but_unmatched_key_is_not_reported_as_a_passphrase() {
        let (tx, rx) = async_channel::bounded(1);
        let mut app = App::new(tx);
        app.frame(json!({"command":"client_state","data":{"user_id":"@alice:example.org","displayname":"Alice","is_logged_in":true}}));
        assert_eq!(app.matrix_id, "@alice:example.org");
        app.recovery = "EsTL 2cTx 9Qy1 8TVd qGsn GDrD i5dT EEuX Qz8U P7hi Z7uu U8wZ".into();
        app.submit_verification();
        let id = rx.try_recv().unwrap().into_rpc()["request_id"].clone();
        app.frame(json!({"command":"error","request_id":id,"data":"incorrect SSSS key"}));
        assert!(app.verification_feedback().contains("checksum are valid"));
    }
}
