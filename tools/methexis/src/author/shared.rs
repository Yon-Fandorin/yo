//! Derivation and publication shared by exact authoring contract versions.

use std::path::{Path, PathBuf};

use super::records::{render_knowledge_file, render_source_record};
use crate::{
    check,
    model::{KnowledgeUnit, SOURCE_SCHEMA, SourcePayload, SourceRecord},
    publication::{self, CapturedFile},
    review::{
        MAX_RECORD_BYTES, OperationFailure, failure_from_diagnostic, hash_bytes,
        operations::load_operation_foundation,
        relative_path,
        storage::{publication_failure, publish_tracked_record},
    },
    source::calculate_revision,
};

const OPERATION: &str = "author-revision";

pub(super) struct SemanticInput {
    pub(super) knowledge_id: String,
    pub(super) expected_revision: String,
    pub(super) source_content: Option<String>,
    pub(super) knowledge_body: Option<String>,
}

pub(super) struct NormalizedSemantic {
    id: String,
    expected_revision: String,
    unit: KnowledgeUnit,
    source_record: SourceRecord,
    source_path: PathBuf,
    source_content: Option<String>,
    knowledge_body: Option<String>,
}

pub(super) struct PreparedSemantic {
    pub(super) id: String,
    pub(super) revision: String,
    pub(super) authored_unit: KnowledgeUnit,
    pub(super) source_path: PathBuf,
    pub(super) knowledge_path: PathBuf,
    pub(super) new_source_record: Option<SourceRecord>,
    pub(super) rendered_knowledge: String,
    pub(super) source_capture: Option<CapturedFile>,
    pub(super) knowledge_capture: CapturedFile,
    pub(super) write_knowledge: bool,
}

pub(super) fn normalize(
    repository_root: &Path,
    input: SemanticInput,
) -> Result<NormalizedSemantic, OperationFailure> {
    let foundation = load_operation_foundation(repository_root, OPERATION, &input.knowledge_id)?;
    let Some(unit) = foundation
        .units
        .iter()
        .find(|unit| unit.metadata.id == input.knowledge_id)
    else {
        return Err(OperationFailure::new(
            OPERATION,
            "unknown_knowledge_id",
            format!("KnowledgeId `{}` does not exist", input.knowledge_id),
            vec![input.knowledge_id],
            "use an ID reported by `methexis check`",
        ));
    };
    let [source_ref] = unit.metadata.sources.as_slice() else {
        return Err(unsupported_unit_shape(&input.knowledge_id));
    };
    let source = foundation
        .sources
        .iter()
        .find(|source| source.record.id == source_ref.id)
        .expect("foundation validation requires the pinned Source record");
    let SourcePayload::Decision { content: _ } = &source.record.payload else {
        return Err(unsupported_unit_shape(&input.knowledge_id));
    };

    let source_content = input
        .source_content
        .as_deref()
        .map(|value| {
            normalize_content(
                value,
                "empty_source_content",
                "Source content",
                &input.knowledge_id,
            )
        })
        .transpose()?;
    let knowledge_body = input
        .knowledge_body
        .as_deref()
        .map(|value| {
            normalize_content(
                value,
                "empty_knowledge_body",
                "Knowledge body",
                &input.knowledge_id,
            )
        })
        .transpose()?
        .map(|body| format!("{body}\n"));

    Ok(NormalizedSemantic {
        id: input.knowledge_id,
        expected_revision: input.expected_revision,
        unit: unit.clone(),
        source_record: source.record.clone(),
        source_path: source.path.clone(),
        source_content,
        knowledge_body,
    })
}

pub(super) fn derive_and_capture(
    repository_root: &Path,
    normalized: NormalizedSemantic,
) -> Result<PreparedSemantic, OperationFailure> {
    let NormalizedSemantic {
        id,
        expected_revision,
        unit,
        source_record,
        source_path,
        source_content,
        knowledge_body,
    } = normalized;

    let new_source_record = source_content.as_ref().map(|content| {
        let mut record = SourceRecord {
            schema: SOURCE_SCHEMA.to_owned(),
            id: source_record.id.clone(),
            revision: String::new(),
            payload: SourcePayload::Decision {
                content: content.clone(),
            },
        };
        record.revision = calculate_revision(&record);
        record
    });
    let source_revision = new_source_record.as_ref().map_or_else(
        || source_record.revision.clone(),
        |record| record.revision.clone(),
    );
    let mut metadata = unit.metadata.clone();
    metadata.sources[0].revision = source_revision;
    let write_knowledge = new_source_record.is_some() || knowledge_body.is_some();
    let body = knowledge_body.unwrap_or_else(|| unit.body.clone());
    let rendered_knowledge = render_knowledge_file(&metadata, &body);
    let display = relative_path(repository_root, &unit.path);
    let mut diagnostics = check::validate_metadata(
        &metadata,
        &body,
        check::body_start_line(&rendered_knowledge, &body),
        &display,
    );
    if let Some(diagnostic) = diagnostics.drain(..).next() {
        return Err(failure_from_diagnostic(
            OPERATION,
            diagnostic,
            "repair the request content and retry",
        ));
    }
    let revision = check::knowledge_revision(&metadata, &body);
    if unit.revision != expected_revision && !(write_knowledge && revision == unit.revision) {
        return Err(OperationFailure::new(
            OPERATION,
            "revision_mismatch",
            format!(
                "expected revision `{}` but current revision is `{}`",
                expected_revision, unit.revision
            ),
            vec![id],
            "rebuild the request from the current revision",
        ));
    }

    let knowledge_path = unit.path.clone();
    let source_capture = new_source_record
        .as_ref()
        .map(|_| capture(repository_root, &source_path, &id))
        .transpose()?;
    let knowledge_capture = capture(repository_root, &knowledge_path, &id)?;
    let authored_unit = KnowledgeUnit {
        metadata,
        body,
        path: knowledge_path.clone(),
        revision: revision.clone(),
    };

    Ok(PreparedSemantic {
        id,
        revision,
        authored_unit,
        source_path,
        knowledge_path,
        new_source_record,
        rendered_knowledge,
        source_capture,
        knowledge_capture,
        write_knowledge,
    })
}

pub(super) fn publish_semantic(
    repository_root: &Path,
    prepared: &PreparedSemantic,
    changed_paths: &mut Vec<String>,
) -> Result<bool, OperationFailure> {
    let mut written = false;
    if let Some(record) = &prepared.new_source_record {
        let expected = hash_bytes(prepared.source_capture.as_ref().expect("captured").bytes());
        written |= publish_result(
            repository_root,
            publish_tracked_record(
                repository_root,
                &prepared.source_path,
                &render_source_record(record),
                Some(&expected),
                OPERATION,
                &prepared.id,
                "Source record",
                "re-run the same request to rebuild from current state",
                "re-run the same request to rebuild from current state",
            ),
            &prepared.source_path,
            changed_paths,
        )?;
    }
    if prepared.write_knowledge {
        let expected = hash_bytes(prepared.knowledge_capture.bytes());
        written |= publish_result(
            repository_root,
            publish_tracked_record(
                repository_root,
                &prepared.knowledge_path,
                prepared.rendered_knowledge.as_bytes(),
                Some(&expected),
                OPERATION,
                &prepared.id,
                "Knowledge unit",
                "re-run the same request to rebuild from current state",
                "re-run the same request to rebuild from current state",
            ),
            &prepared.knowledge_path,
            changed_paths,
        )?;
    }
    Ok(written)
}

pub(super) fn publish_result(
    repository_root: &Path,
    result: Result<&'static str, OperationFailure>,
    path: &Path,
    changed_paths: &mut Vec<String>,
) -> Result<bool, OperationFailure> {
    match result {
        Ok("written") => {
            changed_paths.push(relative_path(repository_root, path));
            Ok(true)
        },
        Ok(_) => Ok(false),
        Err(failure) => Err(failure.into_partial(changed_paths)),
    }
}

pub(super) fn capture(
    repository_root: &Path,
    path: &Path,
    id: &str,
) -> Result<CapturedFile, OperationFailure> {
    publication::capture_file(repository_root, path, MAX_RECORD_BYTES)
        .map_err(|error| publication_failure(OPERATION, id, error))
}

fn unsupported_unit_shape(id: &str) -> OperationFailure {
    OperationFailure::new(
        OPERATION,
        "unsupported_unit_shape",
        format!(
            "KnowledgeId `{id}` must pin exactly one Source of kind `decision`; this first version only supports a single decision Source"
        ),
        vec![id.to_owned()],
        "author multi-Source or non-decision units through the manual review flow",
    )
}

fn normalize_content(
    value: &str,
    code: &'static str,
    label: &str,
    id: &str,
) -> Result<String, OperationFailure> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim().to_owned();
    if normalized.is_empty() {
        return Err(OperationFailure::new(
            OPERATION,
            code,
            format!("{label} must not be empty"),
            vec![id.to_owned()],
            "provide the new content or omit the field",
        ));
    }
    Ok(normalized)
}
