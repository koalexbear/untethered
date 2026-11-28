//! Profile fields only Untethered shows (ROADMAP M15) on the matrix-sdk
//! engine: a banner, a bio and an accent colour on the profile card.
//!
//! They are extended profile fields (spec v1.16, MSC4133): any key in the
//! global profile, beside `displayname` and `avatar_url`. The spec names
//! only one more, `m.tz`, so these use our own namespace. A homeserver says
//! which fields an account may set with the `m.profile_fields` capability;
//! reading the whole profile works everywhere, and a homeserver without
//! extended fields simply has none of these.
use super::{Failure, messaging};
use matrix_sdk::{
    Client,
    ruma::{
        UserId,
        profile::{ProfileFieldName, ProfileFieldValue},
    },
};
use serde_json::Value;

pub const BANNER: &str = "io.github.koalexbear.untethered.banner";
pub const BIO: &str = "io.github.koalexbear.untethered.bio";
pub const ACCENT: &str = "io.github.koalexbear.untethered.accent";
/// Discord's About Me limit.
pub const BIO_LENGTH: usize = 190;

/// Someone's Untethered-only profile fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fields {
    /// The banner picture (mxc URI).
    pub banner: Option<String>,
    pub bio: Option<String>,
    /// The accent colour, 0xRRGGBB; stored as `#rrggbb`.
    pub accent: Option<u32>,
}

/// The fields this homeserver lets the account set (`m.profile_fields`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Editable {
    pub banner: bool,
    pub bio: bool,
    pub accent: bool,
}

impl Editable {
    pub fn any(self) -> bool {
        self.banner || self.bio || self.accent
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Request {
    /// Someone's fields, for their card; for this account, also which ones
    /// it may set.
    Load(String),
    Save(Edit),
}

/// One field of this account's profile; None removes it.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Edit {
    /// A picked picture, uploaded before it's set.
    Banner(Option<Vec<u8>>),
    Bio(Option<String>),
    Accent(Option<u32>),
}

impl std::fmt::Debug for Edit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Banner(bytes) => write!(f, "Banner({:?} bytes)", bytes.as_ref().map(Vec::len)),
            Self::Bio(bio) => write!(f, "Bio({bio:?})"),
            Self::Accent(accent) => write!(f, "Accent({accent:?})"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    Loaded {
        user: String,
        fields: Fields,
        /// For this account only.
        editable: Option<Editable>,
    },
    /// The field as saved: the banner as its mxc URI.
    Saved(Saved),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Saved {
    Banner(Option<String>),
    Bio(Option<String>),
    Accent(Option<u32>),
}

impl Fields {
    /// The fields from a profile's JSON, keeping only well-formed values.
    pub fn from_profile(get: impl Fn(&str) -> Option<Value>) -> Self {
        let text = |key| {
            get(key)
                .and_then(|v| v.as_str().map(str::trim).map(str::to_owned))
                .filter(|v| !v.is_empty())
        };
        Self {
            banner: text(BANNER).filter(|uri| uri.starts_with("mxc://")),
            bio: text(BIO).map(|bio| bio.chars().take(BIO_LENGTH * 2).collect()),
            accent: text(ACCENT).and_then(|c| parse_accent(&c)),
        }
    }

    pub fn apply(&mut self, saved: Saved) {
        match saved {
            Saved::Banner(banner) => self.banner = banner,
            Saved::Bio(bio) => self.bio = bio,
            Saved::Accent(accent) => self.accent = accent,
        }
    }
}

/// `#rrggbb` as 0xRRGGBB.
pub fn parse_accent(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

pub fn format_accent(accent: u32) -> String {
    format!("#{:06x}", accent & 0xff_ffff)
}

pub(super) async fn handle(client: &Client, request: Request) -> Result<Reply, Failure> {
    match request {
        Request::Load(user) => load(client, &user).await,
        Request::Save(edit) => save(client, edit).await.map(Reply::Saved),
    }
}

async fn load(client: &Client, user: &str) -> Result<Reply, Failure> {
    let user = UserId::parse(user).map_err(|_| Failure::Failed)?;
    let profile = client
        .account()
        .fetch_user_profile_of(&user)
        .await
        .map_err(|e| messaging::http(&e))?;
    let fields = Fields::from_profile(|key| profile.get(key).cloned());
    let editable = if client.user_id() == Some(user.as_ref()) {
        Some(editable(client).await)
    } else {
        None
    };
    Ok(Reply::Loaded {
        user: user.to_string(),
        fields,
        editable,
    })
}

/// What `m.profile_fields` allows; nothing when the homeserver doesn't say.
async fn editable(client: &Client) -> Editable {
    let Ok(capability) = client
        .homeserver_capabilities()
        .extended_profile_fields()
        .await
    else {
        return Editable::default();
    };
    let may = |key: &str| capability.can_set_field(&ProfileFieldName::from(key));
    Editable {
        banner: may(BANNER),
        bio: may(BIO),
        accent: may(ACCENT),
    }
}

async fn save(client: &Client, edit: Edit) -> Result<Saved, Failure> {
    let (key, value, saved) = match edit {
        Edit::Banner(Some(bytes)) => {
            let uri = super::profile::upload(client, bytes).await?;
            (
                BANNER,
                Some(Value::from(uri.clone())),
                Saved::Banner(Some(uri)),
            )
        }
        Edit::Banner(None) => (BANNER, None, Saved::Banner(None)),
        Edit::Bio(bio) => {
            let bio = bio
                .map(|b| b.trim().chars().take(BIO_LENGTH).collect::<String>())
                .filter(|b| !b.is_empty());
            (BIO, bio.clone().map(Value::from), Saved::Bio(bio))
        }
        Edit::Accent(accent) => (
            ACCENT,
            accent.map(|c| Value::from(format_accent(c))),
            Saved::Accent(accent.map(|c| c & 0xff_ffff)),
        ),
    };
    let account = client.account();
    let result = match value {
        Some(value) => {
            let value = ProfileFieldValue::new(key, value).map_err(|_| Failure::Failed)?;
            account.set_profile_field(value).await
        }
        None => {
            account
                .delete_profile_field(ProfileFieldName::from(key))
                .await
        }
    };
    result.map_err(|e| messaging::http(&e))?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_come_from_our_keys_and_keep_only_good_values() {
        let profile = json!({
            "displayname": "Mira",
            BANNER: "mxc://h/banner",
            BIO: "  Plays the cello.  ",
            ACCENT: "#3CCFAE",
        });
        let fields = Fields::from_profile(|key| profile.get(key).cloned());
        assert_eq!(
            fields,
            Fields {
                banner: Some("mxc://h/banner".into()),
                bio: Some("Plays the cello.".into()),
                accent: Some(0x3ccfae),
            }
        );
        let odd = json!({BANNER: "https://example.org/a.png", BIO: "", ACCENT: "teal"});
        assert_eq!(
            Fields::from_profile(|key| odd.get(key).cloned()),
            Fields::default()
        );
        assert_eq!(
            Fields::from_profile(|key| json!({ACCENT: 7}).get(key).cloned()).accent,
            None
        );
    }

    #[test]
    fn accents_are_written_as_hex() {
        assert_eq!(format_accent(0x3ccfae), "#3ccfae");
        assert_eq!(format_accent(0x0000ff), "#0000ff");
        assert_eq!(parse_accent("#0000ff"), Some(0xff));
        assert_eq!(parse_accent("0000ff"), None);
        assert_eq!(parse_accent("#00ff"), None);
        assert_eq!(parse_accent("#gg0000"), None);
    }

    #[test]
    fn a_saved_field_replaces_the_known_one() {
        let mut fields = Fields {
            bio: Some("Old".into()),
            ..Fields::default()
        };
        fields.apply(Saved::Bio(None));
        fields.apply(Saved::Banner(Some("mxc://h/b".into())));
        assert_eq!(fields.bio, None);
        assert_eq!(fields.banner.as_deref(), Some("mxc://h/b"));
    }
}
