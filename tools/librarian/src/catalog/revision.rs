//! Methexis semantic revision reproduced at the catalog boundary.

use super::records::{
    ConversationMaterial, ExternalFreshness, KnowledgeMetadata, SourcePayload, SourceRecord,
};
use crate::hash::StableHasher;

const REVISION_DOMAIN: &[u8] = b"methexis.knowledge-revision/v1alpha1";

pub(crate) fn calculate(metadata: &KnowledgeMetadata, body: &str) -> String {
    let mut hasher = StableHasher::new(REVISION_DOMAIN);
    hasher.part(b"schema", metadata.schema.as_bytes());
    hasher.part(b"id", metadata.id.as_bytes());
    hasher.part(b"kind", metadata.kind.as_str().as_bytes());
    hasher.part(b"owner", metadata.owner.as_bytes());
    hasher.part(b"body", body.as_bytes());
    let mut sources = metadata.sources.clone();
    sources.sort();
    for source in sources {
        hasher.part(b"source_id", source.id.as_bytes());
        hasher.part(b"source_revision", source.revision.as_bytes());
    }
    for (relation, targets) in metadata.relations.typed() {
        let mut targets = targets.to_vec();
        targets.sort();
        hasher.part(b"list", relation.as_bytes());
        hasher.count(targets.len());
        for target in targets {
            hasher.part(b"item", target.as_bytes());
        }
    }
    hasher.finish()
}

pub(crate) fn source(record: &SourceRecord) -> String {
    let mut hasher = StableHasher::new(b"methexis.source-revision/v1alpha1");
    hasher.part(b"schema", record.schema.as_bytes());
    hasher.part(b"id", record.id.as_bytes());
    hasher.part(b"kind", record.payload.kind().as_bytes());
    match &record.payload {
        SourcePayload::Decision { content } => hasher.part(b"content", content.as_bytes()),
        SourcePayload::Code {
            path,
            symbol,
            content_hash,
            line_hint: _,
        } => {
            hasher.part(b"path", path.as_bytes());
            hasher.part(b"symbol", symbol.as_bytes());
            hasher.part(b"content_hash", content_hash.as_bytes());
        },
        SourcePayload::Conversation { material } => match material {
            ConversationMaterial::Excerpt { content } => {
                hasher.part(b"mode", b"excerpt");
                hasher.part(b"content", content.as_bytes());
            },
            ConversationMaterial::Opaque {
                reference,
                content_hash,
            } => {
                hasher.part(b"mode", b"opaque");
                hasher.part(b"reference", reference.as_bytes());
                hasher.part(b"content_hash", content_hash.as_bytes());
            },
        },
        SourcePayload::External { freshness } => match freshness {
            ExternalFreshness::Immutable {
                locator,
                version,
                content_hash,
            } => {
                hasher.part(b"freshness", b"immutable");
                hasher.part(b"locator", locator.as_bytes());
                hasher.part(b"version", version.as_bytes());
                hasher.part(b"content_hash", content_hash.as_bytes());
            },
            ExternalFreshness::Mutable {
                locator,
                content_hash,
            } => {
                hasher.part(b"freshness", b"mutable");
                hasher.part(b"locator", locator.as_bytes());
                hasher.part(b"content_hash", content_hash.as_bytes());
            },
            ExternalFreshness::Attested {
                reference,
                content_hash,
                expires_at,
            } => {
                hasher.part(b"freshness", b"attested");
                hasher.part(b"reference", reference.as_bytes());
                hasher.part(b"content_hash", content_hash.as_bytes());
                hasher.part(b"expires_at", expires_at.as_bytes());
            },
        },
    }
    hasher.finish()
}
