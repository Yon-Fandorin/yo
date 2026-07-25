//! Domain-separated, length-delimited identities.

use sha2::{Digest, Sha256};

pub(crate) fn digest(bytes: &[u8]) -> String {
    tagged(Sha256::digest(bytes))
}

pub(crate) struct StableHasher(Sha256);

impl StableHasher {
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.part(b"domain", domain);
        hasher
    }

    pub(crate) fn part(&mut self, label: &[u8], value: &[u8]) {
        self.0.update((label.len() as u64).to_be_bytes());
        self.0.update(label);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    pub(crate) fn count(&mut self, value: usize) {
        self.0.update((value as u64).to_be_bytes());
    }

    pub(crate) fn finish(self) -> String {
        tagged(self.0.finalize())
    }
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
