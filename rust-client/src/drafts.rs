//! Account-scoped composer text, including captions in rooms with staged files.
//! Keep the attachment payload and upload state out of the draft document.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::collections::BTreeMap;

type Drafts = BTreeMap<String, String>;

/// Return true when the stored text changed. Empty composers have no entry.
pub(crate) fn update(drafts: &mut Drafts, room: &str, text: String) -> bool {
    if room.is_empty() {
        return false;
    }
    if text.is_empty() {
        return drafts.remove(room).is_some();
    }
    if drafts.get(room) == Some(&text) {
        return false;
    }
    drafts.insert(room.to_owned(), text);
    true
}

fn account_key(account: &str) -> String {
    URL_SAFE_NO_PAD.encode(account)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod disk {
    use super::{Drafts, account_key};
    use std::{fs, io, path::Path};

    pub(crate) fn load(root: &Path, account: &str) -> io::Result<Drafts> {
        match fs::read(root.join("drafts").join(account_key(account))) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Drafts::new()),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn save(root: &Path, account: &str, drafts: &Drafts) -> io::Result<()> {
        let dir = root.join("drafts");
        let path = dir.join(account_key(account));
        if drafts.is_empty() {
            return match fs::remove_file(path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
        fs::create_dir_all(&dir)?;
        let mut pending = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer(&mut pending, drafts)?;
        pending.as_file().sync_all()?;
        pending.persist(path)?;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod browser {
    use super::{Drafts, account_key};
    use wasm_bindgen::JsValue;

    fn storage() -> Result<web_sys::Storage, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("No browser window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("Local storage unavailable"))
    }

    fn key(account: &str) -> String {
        format!("untethered:drafts:v1:{}", account_key(account))
    }

    pub(crate) fn load(account: &str) -> Result<Drafts, JsValue> {
        storage()?
            .get_item(&key(account))?
            .map(|value| {
                serde_json::from_str(&value).map_err(|e| JsValue::from_str(&e.to_string()))
            })
            .unwrap_or_else(|| Ok(Drafts::new()))
    }

    pub(crate) fn save(account: &str, drafts: &Drafts) -> Result<(), JsValue> {
        let storage = storage()?;
        let key = key(account);
        if drafts.is_empty() {
            storage.remove_item(&key)
        } else {
            let value =
                serde_json::to_string(drafts).map_err(|e| JsValue::from_str(&e.to_string()))?;
            storage.set_item(&key, &value)
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn drafts_round_trip_independently_by_account_and_room() {
        let root = tempfile::tempdir().unwrap();
        let mut drafts = Drafts::new();
        assert!(update(&mut drafts, "!chat:example", "unfinished".into()));
        assert!(update(&mut drafts, "!files:example", "caption".into()));
        assert!(!update(&mut drafts, "!chat:example", "unfinished".into()));
        disk::save(root.path(), "@alice:example", &drafts).unwrap();

        assert_eq!(disk::load(root.path(), "@alice:example").unwrap(), drafts);
        assert!(disk::load(root.path(), "@bob:example").unwrap().is_empty());

        assert!(update(&mut drafts, "!chat:example", String::new()));
        assert!(update(&mut drafts, "!files:example", String::new()));
        disk::save(root.path(), "@alice:example", &drafts).unwrap();
        assert!(
            disk::load(root.path(), "@alice:example")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("drafts"))
                .unwrap()
                .count(),
            0
        );
    }
}

impl crate::App {
    pub(crate) fn record_current_draft(&mut self) {
        if !self.selected.is_empty() && update(&mut self.drafts, &self.selected, self.editor.text())
        {
            self.drafts_changed();
        }
    }

    pub(crate) fn drafts_changed(&mut self) {
        if self.logged_in && !self.matrix_id.is_empty() {
            self.drafts_dirty = true;
            #[cfg(target_arch = "wasm32")]
            self.persist_drafts();
        }
    }

    pub(crate) fn load_drafts(&mut self) {
        self.drafts_dirty = false;
        self.drafts.clear();
        if !self.logged_in || self.matrix_id.is_empty() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let loaded = crate::engine::profile_root()
            .ok()
            .and_then(|root| disk::load(&root, &self.matrix_id).ok());
        #[cfg(target_arch = "wasm32")]
        let loaded = browser::load(&self.matrix_id).ok();
        if let Some(drafts) = loaded {
            self.drafts = drafts;
        }
    }

    pub(crate) fn persist_drafts(&mut self) {
        if !std::mem::take(&mut self.drafts_dirty) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let saved = crate::engine::profile_root()
            .ok()
            .is_some_and(|root| disk::save(&root, &self.matrix_id, &self.drafts).is_ok());
        #[cfg(target_arch = "wasm32")]
        let saved = browser::save(&self.matrix_id, &self.drafts).is_ok();
        if !saved {
            self.status = "Could not save message drafts on this device.".into();
        }
    }
}
