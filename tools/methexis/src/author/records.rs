//! Deterministic Source record and Knowledge unit encoding.
//!
//! These are the first renderers that WRITE Source records and Knowledge
//! units, so every byte is derived from parsed metadata and must round-trip
//! through the existing loaders unchanged.

use crate::model::{KnowledgeMetadata, SourceRecord};

pub(crate) fn render_source_record(record: &SourceRecord) -> Vec<u8> {
    serde_norway::to_string(record)
        .expect("SourceRecord serializes")
        .into_bytes()
}

pub(crate) fn render_knowledge_file(metadata: &KnowledgeMetadata, body: &str) -> String {
    let mut output = String::from("---\n");
    output.push_str("schema: ");
    output.push_str(&yaml_scalar(&metadata.schema));
    output.push('\n');
    output.push_str("id: ");
    output.push_str(&yaml_scalar(&metadata.id));
    output.push('\n');
    output.push_str("kind: ");
    output.push_str(metadata.kind.as_str());
    output.push('\n');
    output.push_str("owner: ");
    output.push_str(&yaml_scalar(&metadata.owner));
    output.push('\n');
    output.push_str("sources:\n");
    for source in &metadata.sources {
        output.push_str("  - id: ");
        output.push_str(&yaml_scalar(&source.id));
        output.push('\n');
        output.push_str("    revision: ");
        output.push_str(&yaml_scalar(&source.revision));
        output.push('\n');
    }
    let relations = [
        ("depends_on", &metadata.relations.depends_on),
        ("constrained_by", &metadata.relations.constrained_by),
        ("validated_by", &metadata.relations.validated_by),
        ("applies_to", &metadata.relations.applies_to),
        ("supersedes", &metadata.relations.supersedes),
    ];
    if relations.iter().any(|(_, targets)| !targets.is_empty()) {
        output.push_str("relations:\n");
        for (name, targets) in relations {
            if targets.is_empty() {
                continue;
            }
            output.push_str("  ");
            output.push_str(name);
            output.push_str(":\n");
            for target in targets {
                output.push_str("    - ");
                output.push_str(&yaml_scalar(target));
                output.push('\n');
            }
        }
    }
    output.push_str("---\n");
    output.push_str(body);
    output
}

/// Renders a YAML scalar bare when it is a safe plain scalar and falls back
/// to a double-quoted scalar otherwise. Ids, revisions, and relation targets
/// in the corpus are plain; quoting keeps unusual preserved metadata valid.
fn yaml_scalar(value: &str) -> String {
    let bytes = value.as_bytes();
    let plain = matches!(bytes.first(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'))
        && bytes.iter().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'/' | b':'
            )
        });
    if plain {
        return value.to_owned();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}
