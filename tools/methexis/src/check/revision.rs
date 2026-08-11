use sha2::{Digest, Sha256};

use crate::model::{KnowledgeMetadata, KnowledgeUnit};

const REVISION_DOMAIN: &[u8] = b"methexis.knowledge-revision/v1alpha1";
const SNAPSHOT_DOMAIN: &[u8] = b"methexis.knowledge-snapshot/v1alpha1";

pub(crate) fn knowledge_revision(metadata: &KnowledgeMetadata, body: &str) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"domain", REVISION_DOMAIN);
    hash_part(&mut hasher, b"schema", metadata.schema.as_bytes());
    hash_part(&mut hasher, b"id", metadata.id.as_bytes());
    hash_part(&mut hasher, b"kind", metadata.kind.as_str().as_bytes());
    hash_part(&mut hasher, b"owner", metadata.owner.as_bytes());
    hash_part(&mut hasher, b"body", body.as_bytes());

    let mut sources = metadata.sources.clone();
    sources.sort();
    for source in sources {
        hash_part(&mut hasher, b"source_id", source.id.as_bytes());
        hash_part(&mut hasher, b"source_revision", source.revision.as_bytes());
    }
    for (relation, targets) in metadata.relations.typed() {
        let mut targets = targets.to_vec();
        targets.sort();
        hash_list(&mut hasher, relation.as_bytes(), &targets);
    }

    tagged_digest(hasher)
}

pub(super) fn snapshot_revision(units: &[KnowledgeUnit]) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"domain", SNAPSHOT_DOMAIN);
    for unit in units {
        hash_part(&mut hasher, b"id", unit.metadata.id.as_bytes());
        hash_part(&mut hasher, b"revision", unit.revision.as_bytes());
    }
    tagged_digest(hasher)
}

fn tagged_digest(hasher: Sha256) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = hasher.finalize();
    let mut output = String::with_capacity(7 + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn hash_list(hasher: &mut Sha256, label: &[u8], values: &[String]) {
    hash_part(hasher, b"list", label);
    hasher.update((values.len() as u64).to_be_bytes());
    for value in values {
        hash_part(hasher, b"item", value.as_bytes());
    }
}

fn hash_part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}
