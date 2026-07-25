//! Closed Source schema and semantic-field validation.

use std::path::{Component, Path};

use crate::{
    check::{Diagnostic, DiagnosticPhase},
    model::{ConversationMaterial, ExternalFreshness, SOURCE_SCHEMA, SourcePayload, SourceRecord},
};

pub(super) fn validate(record: &SourceRecord, path: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if record.schema != SOURCE_SCHEMA {
        diagnostics.push(diagnostic(
            path,
            "unsupported_source_schema",
            format!("expected Source schema `{SOURCE_SCHEMA}`"),
            &record.id,
        ));
    }
    if !semantic_id(&record.id) {
        diagnostics.push(diagnostic(
            path,
            "invalid_source_id",
            "SourceId must use lowercase dot-separated semantic segments".to_owned(),
            &record.id,
        ));
    }
    let expected = super::revision::calculate(record);
    if record.revision != expected {
        diagnostics.push(diagnostic(
            path,
            "source_revision_mismatch",
            format!(
                "recorded SourceRevision `{}` does not match `{expected}`",
                record.revision
            ),
            &record.id,
        ));
    }
    match &record.payload {
        SourcePayload::Decision { content }
        | SourcePayload::Conversation {
            material: ConversationMaterial::Excerpt { content },
        } if content.trim().is_empty() => diagnostics.push(diagnostic(
            path,
            "empty_source_content",
            "Source content must not be empty".to_owned(),
            &record.id,
        )),
        SourcePayload::Code {
            path: code_path,
            symbol,
            content_hash,
            ..
        } => {
            if !safe_relative(code_path) || symbol.trim().is_empty() {
                diagnostics.push(diagnostic(
                    path,
                    "invalid_code_locator",
                    "code Source requires a safe repository-relative path and symbol".to_owned(),
                    &record.id,
                ));
            }
            if !valid_hash(content_hash) {
                diagnostics.push(diagnostic(
                    path,
                    "invalid_source_hash",
                    "code content_hash must be lowercase SHA-256".to_owned(),
                    &record.id,
                ));
            }
        },
        SourcePayload::Conversation {
            material:
                ConversationMaterial::Opaque {
                    reference,
                    content_hash,
                },
        } => validate_reference_hash(path, &record.id, reference, content_hash, &mut diagnostics),
        SourcePayload::External { freshness } => {
            validate_external(path, &record.id, freshness, &mut diagnostics);
        },
        _ => {},
    }
    diagnostics
}

fn validate_external(
    path: &str,
    id: &str,
    freshness: &ExternalFreshness,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match freshness {
        ExternalFreshness::Immutable {
            locator,
            version,
            content_hash,
        } => {
            if version.trim().is_empty() {
                diagnostics.push(diagnostic(
                    path,
                    "invalid_external_version",
                    "immutable external Source requires a version".to_owned(),
                    id,
                ));
            }
            validate_reference_hash(path, id, locator, content_hash, diagnostics);
        },
        ExternalFreshness::Mutable {
            locator,
            content_hash,
        } => validate_reference_hash(path, id, locator, content_hash, diagnostics),
        ExternalFreshness::Attested {
            reference,
            content_hash,
            expires_at,
        } => {
            validate_reference_hash(path, id, reference, content_hash, diagnostics);
            if expires_at.trim().is_empty() {
                diagnostics.push(diagnostic(
                    path,
                    "invalid_attestation_expiry",
                    "attested external Source requires expires_at".to_owned(),
                    id,
                ));
            }
        },
    }
}

fn validate_reference_hash(
    path: &str,
    id: &str,
    reference: &str,
    content_hash: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if reference.trim().is_empty() {
        diagnostics.push(diagnostic(
            path,
            "empty_source_reference",
            "Source reference must not be empty".to_owned(),
            id,
        ));
    }
    if !valid_hash(content_hash) {
        diagnostics.push(diagnostic(
            path,
            "invalid_source_hash",
            "content_hash must be lowercase SHA-256".to_owned(),
            id,
        ));
    }
}

fn safe_relative(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn semantic_id(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(segment)
}

fn segment(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z'))
        && matches!(bytes.last(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !value.contains("--")
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn diagnostic(path: &str, code: &str, message: String, id: &str) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path: path.to_owned(),
        code: code.to_owned(),
        message,
        line: None,
        column: None,
        affected_ids: vec![id.to_owned()],
    }
}
