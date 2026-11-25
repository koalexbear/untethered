//! Per-sender MatrixRTC keys for Element Call's HKDF/GCM media path.
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use livekit::e2ee::key_provider::{KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions};
use serde_json::{Value, json};

pub const EVENT: &str = "io.element.call.encryption_keys";
pub fn provider() -> KeyProvider {
    KeyProvider::new(KeyProviderOptions {
        key_derivation_algorithm: KeyDerivationAlgorithm::HKDF,
        ratchet_window_size: 10,
        key_ring_size: 255,
        ..Default::default()
    })
}
pub struct SenderKey {
    pub index: u8,
    pub bytes: [u8; 32],
}
impl SenderKey {
    pub fn fresh(index: u8) -> Result<Self, &'static str> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| "OS randomness unavailable.")?;
        Ok(Self { index, bytes })
    }
    pub fn to_device(
        &self,
        room: &str,
        member: &str,
        device: &str,
        peer: &str,
        peer_device: &str,
    ) -> Value {
        self.to_devices(room, member, device, [(peer, peer_device)])
    }
    /// The key message for `member`'s `device`, as Element Call sends it.
    pub fn content(&self, room: &str, member: &str, device: &str) -> Value {
        let key = json!({"index":self.index,"key":STANDARD.encode(self.bytes)});
        json!({"room_id":room,"member_id":member,"member":{"id":member,"claimed_device_id":device},"media_key":key,"keys":key})
    }
    /// One gomuks `send_to_device` request, addressed to explicit user/device
    /// pairs, even when multiple devices belonging to the same room member are
    /// in voice.
    pub fn to_devices<'a>(
        &self,
        room: &str,
        member: &str,
        device: &str,
        peers: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Value {
        let content = self.content(room, member, device);
        let mut messages = json!({});
        for (user, target) in peers {
            if !target.is_empty() && target != "*" {
                messages[user][target] = content.clone();
            }
        }
        json!({"event_type":EVENT,"encrypted":true,"messages":messages})
    }
}
/// Only call on Olm-decrypted envelopes with matching sender and room.
pub fn receive(
    content: &Value,
    device: &str,
    member: &str,
) -> Result<Option<(i32, Vec<u8>)>, &'static str> {
    if content["member"]["claimed_device_id"] != device
        || content.get("member_id").unwrap_or(&content["member"]["id"]) != member
    {
        return Ok(None);
    }
    let k = content.get("media_key").unwrap_or(&content["keys"]);
    let idx = k["index"]
        .as_u64()
        .filter(|i| *i < 255)
        .ok_or("Unsupported sender key index.")?;
    let encoded = k["key"].as_str().ok_or("Missing sender key.")?;
    let bytes = STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .map_err(|_| "Invalid sender key.")?;
    if !matches!(bytes.len(), 16 | 32) {
        return Err("Invalid sender key length.");
    }
    Ok(Some((idx as i32, bytes)))
}
