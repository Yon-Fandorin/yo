//! Revision authoring orchestration across Source, Knowledge, Projection, and packet.
//!
//! Publication is sequential per-file compare-and-swap in a fixed order
//! (Source, Knowledge, Projection, packet). A mid-sequence failure reports
//! the paths already written, and re-running the same request converges:
//! completed writes compare equal and only the remainder is published.

use std::path::Path;

use super::{
    AUTHOR_REQUEST_SCHEMA, AUTHOR_SCHEMA, AuthorRevisionRequest, AuthorSuccess, PacketRef,
    records::{render_knowledge_file, render_source_record},
};
use crate::{
    check,
    model::{KnowledgeUnit, SOURCE_SCHEMA, SourcePayload, SourceRecord},
    publication,
    review::{
        COMPILER, MAX_RECORD_BYTES, OperationFailure, PROFILE, PROJECTION_SCHEMA,
        ProjectionMetadata, ProjectionRecord, failure_from_diagnostic, hash_bytes,
        operations::{
            load_operation_foundation, normalize_markdown, publish_review_packet, read_request,
            require_schema,
        },
        records::{
            parse_projection, projection_input_hash, render_projection, render_projection_body,
        },
        relative_path, semantic_hash,
        storage::{publication_failure, publish_tracked_record},
    },
    source::calculate_revision,
};

const OPERATION: &str = "author-revision";

pub(crate) fn author_revision(
    repository_root: &Path,
    request_path: &Path,
) -> Result<AuthorSuccess, OperationFailure> {
    let request: AuthorRevisionRequest = read_request(request_path, OPERATION)?;
    require_schema(
        OPERATION,
        &request.schema,
        AUTHOR_REQUEST_SCHEMA,
        &request.knowledge_id,
    )?;
    if request.source_content.is_none()
        && request.knowledge_body.is_none()
        && request.korean_markdown.is_none()
    {
        return Err(OperationFailure::new(
            OPERATION,
            "empty_author_request",
            "at least one of source_content, knowledge_body, or korean_markdown is required",
            vec![request.knowledge_id.clone()],
            "provide the new Source content, Knowledge body, or Korean Markdown",
        ));
    }
    let foundation = load_operation_foundation(repository_root, OPERATION, &request.knowledge_id)?;
    let Some(unit) = foundation
        .units
        .iter()
        .find(|unit| unit.metadata.id == request.knowledge_id)
    else {
        return Err(OperationFailure::new(
            OPERATION,
            "unknown_knowledge_id",
            format!("KnowledgeId `{}` does not exist", request.knowledge_id),
            vec![request.knowledge_id.clone()],
            "use an ID reported by `methexis check`",
        ));
    };
    let [source_ref] = unit.metadata.sources.as_slice() else {
        return Err(unsupported_unit_shape(&request.knowledge_id));
    };
    let source = foundation
        .sources
        .iter()
        .find(|source| source.record.id == source_ref.id)
        .expect("foundation validation requires the pinned Source record");
    let SourcePayload::Decision { content: _ } = &source.record.payload else {
        return Err(unsupported_unit_shape(&request.knowledge_id));
    };

    let source_content = request
        .source_content
        .as_deref()
        .map(|value| {
            normalize_content(
                value,
                "empty_source_content",
                "Source content",
                &request.knowledge_id,
            )
        })
        .transpose()?;
    let knowledge_body = request
        .knowledge_body
        .as_deref()
        .map(|value| {
            normalize_content(
                value,
                "empty_knowledge_body",
                "Knowledge body",
                &request.knowledge_id,
            )
        })
        .transpose()?
        .map(|body| format!("{body}\n"));
    let korean = request
        .korean_markdown
        .as_deref()
        .map(|value| normalize_markdown(value, OPERATION, &request.knowledge_id))
        .transpose()?;

    // Derive every byte and hash from the captured current state before any
    // publication. The Source revision feeds the Knowledge source pin, which
    // feeds the derived Knowledge revision, which feeds the Projection.
    let new_source_record = source_content.as_ref().map(|content| {
        let mut record = SourceRecord {
            schema: SOURCE_SCHEMA.to_owned(),
            id: source.record.id.clone(),
            revision: String::new(),
            payload: SourcePayload::Decision {
                content: content.clone(),
            },
        };
        record.revision = calculate_revision(&record);
        record
    });
    let source_revision = new_source_record.as_ref().map_or_else(
        || source.record.revision.clone(),
        |record| record.revision.clone(),
    );
    let mut metadata = unit.metadata.clone();
    metadata.sources[0].revision = source_revision;
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
    let new_revision = check::knowledge_revision(&metadata, &body);

    if unit.revision != request.expected_revision {
        // A re-run after a partial publication already applied the request:
        // when the request itself moves the revision and the derived target
        // equals the current revision, every content write will compare equal
        // and only the remaining Projection or packet writes converge.
        let moves_revision = request.source_content.is_some() || request.knowledge_body.is_some();
        if !(moves_revision && new_revision == unit.revision) {
            return Err(OperationFailure::new(
                OPERATION,
                "revision_mismatch",
                format!(
                    "expected revision `{}` but current revision is `{}`",
                    request.expected_revision, unit.revision
                ),
                vec![request.knowledge_id.clone()],
                "rebuild the request from the current revision",
            ));
        }
    }

    let projection_path = repository_root
        .join("methexis/review-projections")
        .join(format!("{}.md", request.knowledge_id));
    let korean = match korean {
        Some(korean) => korean,
        None => parse_projection(&projection_path, repository_root)
            .map_err(|diagnostic| {
                failure_from_diagnostic(
                    OPERATION,
                    diagnostic,
                    "provide korean_markdown to author the replacement Projection",
                )
            })?
            .translation()
            .to_owned(),
    };
    let authored_unit = KnowledgeUnit {
        metadata,
        body,
        path: unit.path.clone(),
        revision: new_revision.clone(),
    };
    let projection_request_hash =
        projection_input_hash(&request.knowledge_id, &new_revision, &korean);
    let projection_bytes = render_projection(&authored_unit, &projection_request_hash, &korean);
    let projection_hash = hash_bytes(&projection_bytes);
    let projection = ProjectionRecord {
        metadata: ProjectionMetadata {
            schema: PROJECTION_SCHEMA.to_owned(),
            knowledge_id: request.knowledge_id.clone(),
            revision: new_revision.clone(),
            profile: PROFILE.to_owned(),
            compiler: COMPILER.to_owned(),
            request_hash: projection_request_hash,
        },
        path: projection_path.clone(),
        hash: projection_hash.clone(),
        body: render_projection_body(&korean),
    };
    let request_hash = semantic_hash(&request);

    // Capture the exact current bytes that each compare-and-swap precondition
    // pins; a concurrent mutation then fails instead of being overwritten.
    let source_capture = new_source_record
        .as_ref()
        .map(|_| capture(repository_root, &source.path, &request.knowledge_id))
        .transpose()?;
    let knowledge_capture = capture(repository_root, &unit.path, &request.knowledge_id)?;
    let projection_capture = match std::fs::symlink_metadata(&projection_path) {
        Ok(_) => Some(capture(
            repository_root,
            &projection_path,
            &request.knowledge_id,
        )?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(OperationFailure::new(
                OPERATION,
                "publication_failed",
                format!("cannot inspect existing Projection: {error}"),
                vec![request.knowledge_id.clone()],
                "repair the destination and retry",
            ));
        },
    };

    let mut changed_paths: Vec<String> = Vec::new();
    let publish = |result: Result<&'static str, OperationFailure>,
                   path: &Path,
                   changed_paths: &mut Vec<String>|
     -> Result<bool, OperationFailure> {
        match result {
            Ok("written") => {
                changed_paths.push(relative_path(repository_root, path));
                Ok(true)
            },
            Ok(_) => Ok(false),
            Err(failure) => Err(failure.into_partial(changed_paths)),
        }
    };

    let mut written = false;
    if let Some(record) = &new_source_record {
        let expected = hash_bytes(source_capture.as_ref().expect("captured").bytes());
        written |= publish(
            publish_tracked_record(
                repository_root,
                &source.path,
                &render_source_record(record),
                Some(&expected),
                OPERATION,
                &request.knowledge_id,
                "Source record",
                "re-run the same request to rebuild from current state",
                "re-run the same request to rebuild from current state",
            ),
            &source.path,
            &mut changed_paths,
        )?;
    }
    if new_source_record.is_some() || request.knowledge_body.is_some() {
        let expected = hash_bytes(knowledge_capture.bytes());
        written |= publish(
            publish_tracked_record(
                repository_root,
                &unit.path,
                rendered_knowledge.as_bytes(),
                Some(&expected),
                OPERATION,
                &request.knowledge_id,
                "Knowledge unit",
                "re-run the same request to rebuild from current state",
                "re-run the same request to rebuild from current state",
            ),
            &unit.path,
            &mut changed_paths,
        )?;
    }
    written |= publish(
        publish_tracked_record(
            repository_root,
            &projection_path,
            &projection_bytes,
            projection_capture
                .as_ref()
                .map(|capture| hash_bytes(capture.bytes()))
                .as_deref(),
            OPERATION,
            &request.knowledge_id,
            "Projection",
            "re-run the same request to rebuild from current state",
            "re-run the same request to rebuild from current state",
        ),
        &projection_path,
        &mut changed_paths,
    )?;
    let published = publish_review_packet(repository_root, OPERATION, &authored_unit, &projection)
        .map_err(|failure| failure.into_partial(&changed_paths))?;
    written |= published.status == "written";

    Ok(AuthorSuccess {
        schema: AUTHOR_SCHEMA,
        ok: true,
        operation: OPERATION,
        status: if written { "written" } else { "unchanged" },
        authority: "draft_proposal",
        affected_ids: vec![request.knowledge_id.clone()],
        revision: new_revision,
        projection_hash,
        packet: PacketRef {
            path: relative_path(repository_root, &published.manifest_path),
            hash: published.packet_hash,
        },
        changed_paths,
        request_hash,
        next_actions: vec![
            "review the tracked diff and packet, then record approval with methexis approve"
                .to_owned(),
        ],
    })
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

fn capture(
    repository_root: &Path,
    path: &Path,
    id: &str,
) -> Result<publication::CapturedFile, OperationFailure> {
    publication::capture_file(repository_root, path, MAX_RECORD_BYTES)
        .map_err(|error| publication_failure(OPERATION, id, error))
}
