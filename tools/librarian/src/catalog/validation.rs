//! Closed Knowledge shape and global relation-graph validation.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use super::{
    markdown,
    records::{
        CatalogUnit, ConversationMaterial, ExternalFreshness, KNOWLEDGE_SCHEMA, KnowledgeKind,
        KnowledgeMetadata, SOURCE_SCHEMA, SourcePayload, SourceRecord,
    },
};

pub(crate) fn knowledge(metadata: &KnowledgeMetadata, body: &str) -> Result<(), String> {
    if metadata.schema != KNOWLEDGE_SCHEMA {
        return Err("unsupported Knowledge schema".to_owned());
    }
    if !is_semantic_id(&metadata.id) || !is_segment(&metadata.owner) {
        return Err("KnowledgeId or OwnerId is invalid".to_owned());
    }
    if metadata.sources.is_empty()
        || metadata
            .sources
            .iter()
            .any(|source| !is_semantic_id(&source.id) || !valid_hash(&source.revision))
        || metadata
            .sources
            .iter()
            .map(|source| &source.id)
            .collect::<BTreeSet<_>>()
            .len()
            != metadata.sources.len()
    {
        return Err("Knowledge Source references are invalid or duplicated".to_owned());
    }
    for (relation, targets) in metadata.relations.typed() {
        if targets.iter().collect::<BTreeSet<_>>().len() != targets.len()
            || targets.iter().any(|target| {
                target.is_empty()
                    || (matches!(relation, "depends_on" | "constrained_by" | "supersedes")
                        && !is_semantic_id(target))
            })
        {
            return Err(format!(
                "relation `{relation}` has invalid or duplicate targets"
            ));
        }
    }
    let sections: &[&str] = match metadata.kind {
        KnowledgeKind::Definition | KnowledgeKind::Rule => &["Statement"],
        KnowledgeKind::Decision => &["Statement", "Rationale"],
        KnowledgeKind::Procedure => &["Statement", "Steps", "Completion Criteria"],
    };
    markdown::validate_body(body, sections)
}

pub(crate) fn graphs(units: &BTreeMap<String, CatalogUnit>) -> Result<(), (String, Vec<String>)> {
    if units.is_empty() {
        return Err(("the Knowledge corpus is empty".to_owned(), Vec::new()));
    }
    for unit in units.values() {
        for target in unit.relations.knowledge_targets() {
            if !units.contains_key(target) {
                return Err((
                    format!("{} references missing KnowledgeId {target}", unit.id),
                    vec![unit.id.clone(), target.clone()],
                ));
            }
        }
    }
    let required = units
        .values()
        .map(|unit| {
            (
                unit.id.clone(),
                unit.relations
                    .depends_on
                    .iter()
                    .chain(&unit.relations.constrained_by)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    detect_cycle(&required, "required relation")?;
    let supersedes = units
        .values()
        .map(|unit| (unit.id.clone(), unit.relations.supersedes.clone()))
        .collect::<BTreeMap<_, _>>();
    detect_cycle(&supersedes, "supersedes")
}

fn detect_cycle(
    graph: &BTreeMap<String, Vec<String>>,
    name: &str,
) -> Result<(), (String, Vec<String>)> {
    let mut complete = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let mut stack = Vec::new();
    for id in graph.keys() {
        if let Some(cycle) = visit(id, graph, &mut visiting, &mut complete, &mut stack) {
            return Err((format!("{name} graph contains a cycle"), cycle));
        }
    }
    Ok(())
}

fn visit(
    id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    visiting: &mut BTreeSet<String>,
    complete: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if complete.contains(id) {
        return None;
    }
    if visiting.contains(id) {
        let start = stack.iter().position(|candidate| candidate == id)?;
        let mut cycle = stack[start..].to_vec();
        cycle.push(id.to_owned());
        return Some(cycle);
    }
    visiting.insert(id.to_owned());
    stack.push(id.to_owned());
    for target in graph.get(id).into_iter().flatten() {
        if let Some(cycle) = visit(target, graph, visiting, complete, stack) {
            return Some(cycle);
        }
    }
    stack.pop();
    visiting.remove(id);
    complete.insert(id.to_owned());
    None
}

pub(crate) fn is_semantic_id(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(is_segment)
}

fn is_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        && !segment.contains("--")
}

pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn owner_id(id: &str) -> bool {
    is_segment(id)
}

pub(crate) fn source(record: &SourceRecord) -> Result<(), String> {
    if record.schema != SOURCE_SCHEMA || !is_semantic_id(&record.id) {
        return Err("Source schema or SourceId is invalid".to_owned());
    }
    if record.revision != super::revision::source(record) {
        return Err("SourceRevision does not match the Source semantic fields".to_owned());
    }
    match &record.payload {
        SourcePayload::Decision { content }
        | SourcePayload::Conversation {
            material: ConversationMaterial::Excerpt { content },
        } if content.trim().is_empty() => Err("Source content must not be empty".to_owned()),
        SourcePayload::Code {
            path,
            symbol,
            content_hash,
            line_hint: _,
        } if !safe_relative(path) || symbol.trim().is_empty() || !valid_hash(content_hash) => {
            Err("code Source locator or content hash is invalid".to_owned())
        },
        SourcePayload::Conversation {
            material:
                ConversationMaterial::Opaque {
                    reference,
                    content_hash,
                },
        } => validate_reference_hash(reference, content_hash),
        SourcePayload::External { freshness } => match freshness {
            ExternalFreshness::Immutable {
                locator,
                version,
                content_hash,
            } if version.trim().is_empty() => {
                Err("immutable external Source requires a version".to_owned())
            },
            ExternalFreshness::Immutable {
                locator,
                version: _,
                content_hash,
            }
            | ExternalFreshness::Mutable {
                locator,
                content_hash,
            } => validate_reference_hash(locator, content_hash),
            ExternalFreshness::Attested {
                reference,
                content_hash,
                expires_at,
            } if expires_at.trim().is_empty() => {
                Err("attested external Source requires expires_at".to_owned())
            },
            ExternalFreshness::Attested {
                reference,
                content_hash,
                expires_at: _,
            } => validate_reference_hash(reference, content_hash),
        },
        _ => Ok(()),
    }
}

fn validate_reference_hash(reference: &str, content_hash: &str) -> Result<(), String> {
    if reference.trim().is_empty() || !valid_hash(content_hash) {
        Err("Source reference or content hash is invalid".to_owned())
    } else {
        Ok(())
    }
}

fn safe_relative(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}
