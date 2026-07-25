//! Deterministic SourceRevision identity.

use sha2::{Digest, Sha256};

use crate::model::{ConversationMaterial, ExternalFreshness, SourcePayload, SourceRecord};

const REVISION_DOMAIN: &[u8] = b"methexis.source-revision/v1alpha1";

pub(crate) fn calculate(record: &SourceRecord) -> String {
    let mut hasher = Sha256::new();
    part(&mut hasher, b"domain", REVISION_DOMAIN);
    part(&mut hasher, b"schema", record.schema.as_bytes());
    part(&mut hasher, b"id", record.id.as_bytes());
    part(&mut hasher, b"kind", record.payload.kind().as_bytes());
    match &record.payload {
        SourcePayload::Decision { content } => part(&mut hasher, b"content", content.as_bytes()),
        SourcePayload::Code {
            path,
            symbol,
            content_hash,
            line_hint: _,
        } => {
            part(&mut hasher, b"path", path.as_bytes());
            part(&mut hasher, b"symbol", symbol.as_bytes());
            part(&mut hasher, b"content_hash", content_hash.as_bytes());
        },
        SourcePayload::Conversation { material } => match material {
            ConversationMaterial::Excerpt { content } => {
                part(&mut hasher, b"mode", b"excerpt");
                part(&mut hasher, b"content", content.as_bytes());
            },
            ConversationMaterial::Opaque {
                reference,
                content_hash,
            } => {
                part(&mut hasher, b"mode", b"opaque");
                part(&mut hasher, b"reference", reference.as_bytes());
                part(&mut hasher, b"content_hash", content_hash.as_bytes());
            },
        },
        SourcePayload::External { freshness } => match freshness {
            ExternalFreshness::Immutable {
                locator,
                version,
                content_hash,
            } => {
                part(&mut hasher, b"freshness", b"immutable");
                part(&mut hasher, b"locator", locator.as_bytes());
                part(&mut hasher, b"version", version.as_bytes());
                part(&mut hasher, b"content_hash", content_hash.as_bytes());
            },
            ExternalFreshness::Mutable {
                locator,
                content_hash,
            } => {
                part(&mut hasher, b"freshness", b"mutable");
                part(&mut hasher, b"locator", locator.as_bytes());
                part(&mut hasher, b"content_hash", content_hash.as_bytes());
            },
            ExternalFreshness::Attested {
                reference,
                content_hash,
                expires_at,
            } => {
                part(&mut hasher, b"freshness", b"attested");
                part(&mut hasher, b"reference", reference.as_bytes());
                part(&mut hasher, b"content_hash", content_hash.as_bytes());
                part(&mut hasher, b"expires_at", expires_at.as_bytes());
            },
        },
    }
    tagged_digest(hasher)
}

fn part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn tagged_digest(hasher: Sha256) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
