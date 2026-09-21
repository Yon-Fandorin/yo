use chacha20::{
    XChaCha20,
    cipher::{KeyIvInit, StreamCipher},
};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::AeadInPlace};
use poly1305::{Poly1305, universal_hash::UniversalHash};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use zeroize::{Zeroize, Zeroizing};

use super::{Result, SecretStoreError, model::Generation};
use crate::SecretInput;

pub(super) type Key = Zeroizing<[u8; 32]>;
const MAGIC: &[u8; 8] = b"YOSEC001";
const PADDED: usize = 4 + SecretInput::MAX_BYTES;
pub(super) const ENTRY_BYTES: usize = 8 + 24 + PADDED + 16;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed<T> {
    envelope: T,
    authentication_nonce: [u8; 24],
    authentication_tag: [u8; 16],
}

fn nonce() -> Result<[u8; 24]> {
    let mut value = [0; 24];
    getrandom::fill(&mut value).map_err(|_| SecretStoreError)?;
    Ok(value)
}
fn cipher(key: &Key) -> Result<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| SecretStoreError)
}
pub(super) fn sign<T: Serialize>(key: &Key, envelope: T) -> Result<Vec<u8>> {
    let nonce = nonce()?;
    let aad = serde_json::to_vec(&envelope).map_err(|_| SecretStoreError)?;
    let tag = cipher(key)?
        .encrypt_in_place_detached(XNonce::from_slice(&nonce), &aad, &mut [])
        .map_err(|_| SecretStoreError)?;
    serde_json::to_vec(&Signed {
        envelope,
        authentication_nonce: nonce,
        authentication_tag: tag.into(),
    })
    .map_err(|_| SecretStoreError)
}
pub(super) fn verify<T: Serialize + DeserializeOwned>(key: &Key, bytes: &[u8]) -> Result<T> {
    let signed: Signed<T> = serde_json::from_slice(bytes).map_err(|_| SecretStoreError)?;
    let aad = serde_json::to_vec(&signed.envelope).map_err(|_| SecretStoreError)?;
    cipher(key)?
        .decrypt_in_place_detached(
            XNonce::from_slice(&signed.authentication_nonce),
            &aad,
            &mut [],
            chacha20poly1305::Tag::from_slice(&signed.authentication_tag),
        )
        .map_err(|_| SecretStoreError)?;
    Ok(signed.envelope)
}
fn aad(generation: &Generation) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Binding<'a> {
        format: &'static str,
        entry: &'a str,
        scope: &'a str,
        destination: &'a super::SecretDestination,
    }
    serde_json::to_vec(&Binding {
        format: "yo.secret-entry/v1",
        entry: generation.entry.as_deref().ok_or(SecretStoreError)?,
        scope: &generation.scope,
        destination: &generation.destination,
    })
    .map_err(|_| SecretStoreError)
}
pub(super) fn encrypt(key: &Key, generation: &Generation, input: &SecretInput) -> Result<Vec<u8>> {
    let nonce = nonce()?;
    let value = input.expose().as_bytes();
    let mut body = Zeroizing::new(vec![0; PADDED]);
    body[..4].copy_from_slice(&(value.len() as u32).to_be_bytes());
    body[4..4 + value.len()].copy_from_slice(value);
    let tag = cipher(key)?
        .encrypt_in_place_detached(XNonce::from_slice(&nonce), &aad(generation)?, body.as_mut())
        .map_err(|_| SecretStoreError)?;
    let mut output = Vec::with_capacity(ENTRY_BYTES);
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&body);
    output.extend_from_slice(&tag);
    Ok(output)
}
pub(super) fn authenticate(key: &Key, generation: &Generation, bytes: &[u8]) -> Result<()> {
    if bytes.len() != ENTRY_BYTES || &bytes[..8] != MAGIC {
        return Err(SecretStoreError);
    }
    let aad = aad(generation)?;
    let ciphertext = &bytes[32..32 + PADDED];
    let mut stream = XChaCha20::new(
        chacha20::Key::from_slice(key.as_ref()),
        chacha20::XNonce::from_slice(&bytes[8..32]),
    );
    let mut mac_key = poly1305::Key::default();
    stream.apply_keystream(mac_key.as_mut_slice());
    let mut mac = Poly1305::new(&mac_key);
    mac_key.as_mut_slice().zeroize();
    mac.update_padded(&aad);
    mac.update_padded(ciphertext);
    let mut lengths = poly1305::Block::default();
    lengths[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lengths[8..].copy_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    mac.update(&[lengths]);
    mac.verify(chacha20poly1305::Tag::from_slice(&bytes[32 + PADDED..]))
        .map_err(|_| SecretStoreError)
}
pub(super) fn decrypt(key: &Key, generation: &Generation, bytes: &[u8]) -> Result<SecretInput> {
    authenticate(key, generation, bytes)?;
    let mut body = Zeroizing::new(bytes[32..32 + PADDED].to_vec());
    cipher(key)?
        .decrypt_in_place_detached(
            XNonce::from_slice(&bytes[8..32]),
            &aad(generation)?,
            body.as_mut(),
            chacha20poly1305::Tag::from_slice(&bytes[32 + PADDED..]),
        )
        .map_err(|_| SecretStoreError)?;
    let length = u32::from_be_bytes(body[..4].try_into().map_err(|_| SecretStoreError)?) as usize;
    if length > SecretInput::MAX_BYTES || body[4 + length..].iter().any(|b| *b != 0) {
        return Err(SecretStoreError);
    }
    let text = String::from_utf8(body[4..4 + length].to_vec()).map_err(|_| SecretStoreError)?;
    SecretInput::new(text).map_err(|_| SecretStoreError)
}
