use crate::{App, Request, Section, media, rooms::Rooms};
use iced::widget::text_editor;
use serde_json::json;
use url::{Host, Url};

/// Sign-in and device state as reported by either engine: gomuks's
/// `client_state` frame or the matrix-sdk engine's `Event::Account`.
pub struct State {
    pub logged_in: bool,
    pub verified: bool,
    pub recovery_unavailable: bool,
    pub profile: String,
    pub user_id: String,
}

impl App {
    pub fn apply_account(&mut self, state: State) {
        let was = self.logged_in;
        let was_verified = self.verified;
        self.connected = true;
        self.logged_in = state.logged_in;
        self.verified = state.verified;
        #[cfg(target_os = "linux")]
        if !self.verified {
            self.clear_ring();
        }
        self.recovery_unavailable = state.recovery_unavailable;
        if self.verified || !self.logged_in {
            self.recovery.clear();
            self.verification_note.clear();
        }
        self.profile = state.profile;
        let user = state.user_id;
        if self.matrix_id != user || !self.verified || !self.logged_in {
            self.setup = None;
            self.pending.retain(|_, p| !matches!(p, Request::Setup(_)));
            self.close_mentions();
            self.pending
                .retain(|_, p| !matches!(p, Request::ShareReads(..) | Request::ReadState(..)));
        }
        // Account data can arrive before the first account state of a
        // restored session (the engine reads it from its store right away), so
        // forget it only when the account goes away, not when it shows up.
        if !self.logged_in || (!self.matrix_id.is_empty() && self.matrix_id != user) {
            self.read_sharing = Default::default();
            #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
            self.alerts.reset();
        }
        self.matrix_id = user;
        if was != self.logged_in {
            #[cfg(target_os = "linux")]
            {
                self.drop_epoch += 1;
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.paste_seq += 1;
                self.pasting = false;
            }
            self.pending.retain(|_, p| matches!(p, Request::Login));
            self.drafts.clear();
            self.clear_search();
            self.reply_drafts.clear();
            self.reset_directory();
            self.leave_permission_panel();
            self.editing = None;
            self.close_original();
            self.editor = text_editor::Content::new();
            self.loaded.clear();
            self.timeline_end = None;
            self.selected.clear();
            self.section = Section::All;
            self.account = !self.logged_in;
            self.outbox.rooms.clear();
            self.outbox.skipped.clear();
            self.inline.reset();
            self.batch = None;
            self.rtc_status.clear();
            self.media_limit = media::MAX_BYTES;
            if !self.logged_in {
                self.live = Rooms::default();
            }
        }
        if self.logged_in && self.verified && !was_verified {
            self.rpc("get_media_config", json!({}), Request::MediaConfig);
        }
        self.status.clear();
        self.reconcile();
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
