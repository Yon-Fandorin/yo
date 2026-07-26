//! Domain-separated, length-delimited context identities.

use sha2::{Digest, Sha256};

pub(super) fn digest(bytes: &[u8]) -> String {
    tagged(Sha256::digest(bytes))
}

pub(super) struct StableHasher(Sha256);

impl StableHasher {
    pub(super) fn new(domain: &[u8]) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.part(b"domain", domain);
        hasher
    }

    pub(super) fn part(&mut self, label: &[u8], value: &[u8]) {
        self.0.update((label.len() as u64).to_be_bytes());
        self.0.update(label);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    pub(super) fn finish(self) -> String {
        tagged(self.0.finalize())
    }
}

pub(super) fn valid(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn tagged(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = digest.as_ref();
    let mut output = String::with_capacity(7 + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(*byte >> 4)]));
        output.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    output
}
