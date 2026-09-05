//! Working-tree catalog construction and structural validation.

mod markdown;
mod projection;
mod records;
mod revision;
mod snapshot;
#[cfg(test)]
mod tests;
mod validation;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(crate) use records::CatalogUnit as Unit;
use records::{
    CatalogUnit, KnowledgeMetadata, OWNER_SCHEMA, OwnerRecord, ProjectionMetadata, SourceRecord,
};

use crate::error::DiscoveryError;

pub(crate) struct Catalog {
    pub(crate) units: BTreeMap<String, CatalogUnit>,
    pub(crate) hash: String,
}

pub(crate) fn load(repository_root: &Path) -> Result<Catalog, DiscoveryError> {
    let captured = snapshot::capture(repository_root)?;
    let mut units = BTreeMap::new();
    let mut projections = Vec::new();
    let mut owners = BTreeMap::new();
    let mut sources = BTreeMap::new();

    for file in &captured.files {
        if file.path.starts_with("methexis/knowledge/") {
            let unit = parse_knowledge(&file.path, &file.bytes)?;
            if units.insert(unit.id.clone(), unit).is_some() {
                return Err(DiscoveryError::catalog(
                    "duplicate_knowledge_id",
                    "the catalog contains the same KnowledgeId more than once",
                    Vec::new(),
                    vec![file.path.clone()],
                ));
            }
        } else if file.path.starts_with("methexis/review-projections/") {
            projections.push(parse_projection(&file.path, &file.bytes)?);
        } else if file.path.starts_with("methexis/owners/") {
            let owner = parse_owner(&file.path, &file.bytes)?;
            if owners.insert(owner.id.clone(), file.path.clone()).is_some() {
                return Err(DiscoveryError::catalog(
                    "duplicate_owner_id",
                    "the catalog contains the same OwnerId more than once",
                    Vec::new(),
                    vec![file.path.clone()],
                ));
            }
        } else if file.path.starts_with("methexis/sources/") {
            let source = parse_source(&file.path, &file.bytes)?;
            if sources
                .insert(source.id.clone(), file.path.clone())
                .is_some()
            {
                return Err(DiscoveryError::catalog(
                    "duplicate_source_id",
                    "the catalog contains the same SourceId more than once",
                    Vec::new(),
                    vec![file.path.clone()],
                ));
            }
        }
    }

    validation::graphs(&units).map_err(|(message, affected_ids)| {
        DiscoveryError::catalog("invalid_relation_graph", message, affected_ids, Vec::new())
    })?;
    for unit in units.values() {
        if !owners.contains_key(&unit.owner) {
            return Err(DiscoveryError::catalog(
                "missing_owner",
                format!("{} references missing OwnerId {}", unit.id, unit.owner),
                vec![unit.id.clone()],
                vec![unit.path.clone()],
            ));
        }
        for source in &unit.sources {
            if !sources.contains_key(&source.id) {
                return Err(DiscoveryError::catalog(
                    "missing_source",
                    format!("{} references missing Source {}", unit.id, source.id),
                    vec![unit.id.clone(), source.id.clone()],
                    vec![unit.path.clone()],
                ));
            }
        }
    }

    let mut seen_projections = BTreeSet::new();
    for (metadata, body, path) in projections {
        if !seen_projections.insert(metadata.knowledge_id.clone()) {
            return Err(DiscoveryError::catalog(
                "duplicate_review_projection",
                "the catalog contains more than one review Projection for a KnowledgeId",
                vec![metadata.knowledge_id],
                vec![path],
            ));
        }
        let Some(unit) = units.get_mut(&metadata.knowledge_id) else {
            return Err(DiscoveryError::catalog(
                "orphan_review_projection",
                "a review Projection references a missing KnowledgeId",
                vec![metadata.knowledge_id],
                vec![path],
            ));
        };
        if metadata.revision == unit.revision {
            unit.projection = Some(body);
        }
    }

    Ok(Catalog {
        units,
        hash: captured.hash,
    })
}

fn parse_knowledge(path: &str, bytes: &[u8]) -> Result<CatalogUnit, DiscoveryError> {
    let content = normalized_utf8(path, bytes)?;
    let (frontmatter, body) = split_frontmatter(path, &content)?;
    let metadata: KnowledgeMetadata = serde_norway::from_str(frontmatter)
        .map_err(|error| invalid_record(path, format!("invalid Knowledge frontmatter: {error}")))?;
    validation::knowledge(&metadata, body).map_err(|message| invalid_record(path, message))?;
    let revision = revision::calculate(&metadata, body);
    let title = body
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or_default()
        .trim()
        .to_owned();

    Ok(CatalogUnit {
        id: metadata.id,
        revision,
        owner: metadata.owner,
        sources: metadata.sources,
        path: path.to_owned(),
        title,
        body: body.to_owned(),
        projection: None,
        relations: metadata.relations,
    })
}

fn parse_owner(path: &str, bytes: &[u8]) -> Result<OwnerRecord, DiscoveryError> {
    let content = normalized_utf8(path, bytes)?;
    let record: OwnerRecord = serde_norway::from_str(&content)
        .map_err(|error| invalid_record(path, format!("invalid Owner record: {error}")))?;
    if record.schema != OWNER_SCHEMA || !validation::owner_id(&record.id) {
        return Err(invalid_record(path, "Owner schema or OwnerId is invalid"));
    }
    Ok(record)
}

fn parse_source(path: &str, bytes: &[u8]) -> Result<SourceRecord, DiscoveryError> {
    let content = normalized_utf8(path, bytes)?;
    let record: SourceRecord = serde_norway::from_str(&content)
        .map_err(|error| invalid_record(path, format!("invalid Source record: {error}")))?;
    validation::source(&record).map_err(|message| invalid_record(path, message))?;
    Ok(record)
}

fn parse_projection(
    path: &str,
    bytes: &[u8],
) -> Result<(ProjectionMetadata, String, String), DiscoveryError> {
    let content = normalized_utf8(path, bytes)?;
    let (frontmatter, body) = split_frontmatter(path, &content)?;
    let metadata: ProjectionMetadata = serde_norway::from_str(frontmatter).map_err(|error| {
        invalid_record(path, format!("invalid Projection frontmatter: {error}"))
    })?;
    projection::validate(&metadata, body, bytes)
        .map_err(|message| invalid_record(path, message))?;
    Ok((metadata, body.to_owned(), path.to_owned()))
}

fn normalized_utf8(path: &str, bytes: &[u8]) -> Result<String, DiscoveryError> {
    let content = std::str::from_utf8(bytes)
        .map_err(|error| invalid_record(path, format!("record is not UTF-8: {error}")))?;
    Ok(content.replace("\r\n", "\n").replace('\r', "\n"))
}

fn split_frontmatter<'a>(
    path: &str,
    content: &'a str,
) -> Result<(&'a str, &'a str), DiscoveryError> {
    let remainder = content
        .strip_prefix("---\n")
        .ok_or_else(|| invalid_record(path, "record must start with YAML frontmatter"))?;
    let boundary = remainder
        .find("\n---\n")
        .ok_or_else(|| invalid_record(path, "record has no closing frontmatter delimiter"))?;
    Ok((&remainder[..boundary], &remainder[boundary + 5..]))
}

fn invalid_record(path: &str, message: impl Into<String>) -> DiscoveryError {
    DiscoveryError::catalog(
        "invalid_catalog_record",
        message,
        Vec::new(),
        vec![path.to_owned()],
    )
}
