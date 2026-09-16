use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use super::model::{
    REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2, REQUEST_SCHEMA_V1_ALPHA3, REQUEST_SCHEMA_V1_ALPHA4,
    REQUEST_SCHEMA_V1_ALPHA5, REQUEST_SCHEMA_V1_ALPHA6, REQUEST_SCHEMA_V1_ALPHA7, Request, Target,
};
use crate::review_result;

const MAX_TOKEN_BUDGET: usize = 1_000_000;
const MAX_VALUE_BYTES: usize = 4096;

pub(super) fn validate_and_normalize(request: &mut Request) -> Result<(), String> {
    if !matches!(
        request.schema.as_str(),
        REQUEST_SCHEMA
            | REQUEST_SCHEMA_V1_ALPHA2
            | REQUEST_SCHEMA_V1_ALPHA3
            | REQUEST_SCHEMA_V1_ALPHA4
            | REQUEST_SCHEMA_V1_ALPHA5
            | REQUEST_SCHEMA_V1_ALPHA6
            | REQUEST_SCHEMA_V1_ALPHA7
    ) {
        return Err(format!(
            "unsupported Slice review preparation schema `{}`; expected `{REQUEST_SCHEMA}`, `{REQUEST_SCHEMA_V1_ALPHA2}`, `{REQUEST_SCHEMA_V1_ALPHA3}`, `{REQUEST_SCHEMA_V1_ALPHA4}`, `{REQUEST_SCHEMA_V1_ALPHA5}`, `{REQUEST_SCHEMA_V1_ALPHA6}`, or `{REQUEST_SCHEMA_V1_ALPHA7}`",
            request.schema
        ));
    }
    compact_token(&request.slice, "slice")?;
    require_path_component(&request.slice, "slice")?;
    normalize_strings(&mut request.knowledge_ids, "knowledge_ids")?;
    if matches!(
        request.schema.as_str(),
        REQUEST_SCHEMA_V1_ALPHA4
            | REQUEST_SCHEMA_V1_ALPHA5
            | REQUEST_SCHEMA_V1_ALPHA6
            | REQUEST_SCHEMA_V1_ALPHA7
    ) {
        if !request.repository_authority_paths.is_empty() {
            return Err(
                "derived-authority preparation requires the caller list to be empty".to_owned(),
            );
        }
        let expected_policy = if matches!(
            request.schema.as_str(),
            REQUEST_SCHEMA_V1_ALPHA5 | REQUEST_SCHEMA_V1_ALPHA6 | REQUEST_SCHEMA_V1_ALPHA7
        ) {
            "changed-workflow-authority/v1alpha2"
        } else {
            "changed-workflow-authority/v1alpha1"
        };
        if request.repository_authority_policy.as_deref() != Some(expected_policy) {
            return Err(format!(
                "{} requires repository_authority_policy `{expected_policy}`",
                request.schema
            ));
        }
    } else {
        if request.repository_authority_policy.is_some() {
            return Err(
                "repository_authority_policy is supported only by derived-authority review preparation"
                    .to_owned(),
            );
        }
        normalize_strings(
            &mut request.repository_authority_paths,
            "repository_authority_paths",
        )?;
    }
    normalize_strings(&mut request.review_lenses, "review_lenses")?;
    require_non_empty_strings(&request.review_questions, "review_questions")?;
    if request.validation_evidence.is_empty() {
        return Err("validation_evidence must not be empty".to_owned());
    }
    request
        .validation_evidence
        .sort_by(|left, right| left.name.cmp(&right.name));
    let mut names = BTreeSet::new();
    for evidence in &request.validation_evidence {
        compact_token(&evidence.name, "validation evidence name")?;
        compact_value(&evidence.path, "validation evidence path")?;
        if !names.insert(evidence.name.as_str()) {
            return Err(format!(
                "validation_evidence contains duplicate name `{}`",
                evidence.name
            ));
        }
    }
    for path in &request.repository_authority_paths {
        require_repository_relative(path, "repository authority path")?;
    }
    require_budget(request.context_max_tokens, "context_max_tokens")?;
    require_budget(
        request.max_managed_payload_tokens,
        "max_managed_payload_tokens",
    )?;
    match &request.target {
        Target::ManagedModel {
            provider,
            account,
            model,
            connection_repository_path,
            session_repository_path,
        } => {
            compact_token(provider, "target provider")?;
            compact_token(account, "target account")?;
            compact_token(model, "target model")?;
            require_absolute(connection_repository_path, "connection_repository_path")?;
            if let Some(path) = session_repository_path {
                require_absolute(path, "session_repository_path")?;
            }
        },
        Target::DelegatedHost {
            host,
            session_repository_path,
        } => {
            if !matches!(host.as_str(), "codex" | "grok") {
                return Err("delegated review target must be `codex` or `grok`".to_owned());
            }
            if let Some(path) = session_repository_path {
                require_absolute(path, "session_repository_path")?;
            }
        },
    }
    Ok(())
}

pub(super) fn prepared_review_questions(request: &Request) -> Vec<String> {
    let mut questions = request.review_questions.clone();
    if matches!(
        request.schema.as_str(),
        REQUEST_SCHEMA_V1_ALPHA2
            | REQUEST_SCHEMA_V1_ALPHA3
            | REQUEST_SCHEMA_V1_ALPHA4
            | REQUEST_SCHEMA_V1_ALPHA5
            | REQUEST_SCHEMA_V1_ALPHA6
            | REQUEST_SCHEMA_V1_ALPHA7
    ) {
        questions.push(review_result::OUTPUT_INSTRUCTION.to_owned());
    }
    questions
}

fn normalize_strings(values: &mut [String], label: &str) -> Result<(), String> {
    require_non_empty_strings(values, label)?;
    values.sort();
    for pair in values.windows(2) {
        if pair[0] == pair[1] {
            return Err(format!("{label} contains duplicate value `{}`", pair[0]));
        }
    }
    Ok(())
}

fn require_non_empty_strings(values: &[String], label: &str) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    for value in values {
        compact_value(value, label)?;
    }
    Ok(())
}

fn compact_token(value: &str, label: &str) -> Result<(), String> {
    compact_value(value, label)?;
    if value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(format!("{label} must be one compact token"));
    }
    Ok(())
}

fn compact_value(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value != value.trim() || value.len() > MAX_VALUE_BYTES {
        return Err(format!(
            "{label} must be non-empty, trimmed, and at most {MAX_VALUE_BYTES} bytes"
        ));
    }
    Ok(())
}

fn require_repository_relative(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} must be a normalized repository-relative path"
        ));
    }
    Ok(())
}

fn require_path_component(value: &str, label: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!("{label} must be one normalized path component"));
    }
    Ok(())
}

fn require_absolute(value: &str, label: &str) -> Result<(), String> {
    compact_value(value, label)?;
    let path = Path::new(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!("{label} must be a normalized absolute path"));
    }
    Ok(())
}

fn require_budget(value: usize, label: &str) -> Result<(), String> {
    if value == 0 || value > MAX_TOKEN_BUDGET {
        return Err(format!("{label} must be between 1 and {MAX_TOKEN_BUDGET}"));
    }
    Ok(())
}
