use std::collections::BTreeSet;

use serde::Deserialize;

use crate::review_protocol::require_commit;

pub(crate) const NAME_PREFIX: &str = "external-operation/";
const SCHEMA: &str = "yo.external-operation-evidence/v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    schema: String,
    candidate_commit: String,
    operation: Operation,
    counterfactual: String,
    observations: Vec<Observation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    working_directory: String,
    argv: Vec<String>,
    expected_exit: ExitStatus,
    observed_exit: ExitStatus,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ExitStatus {
    Code { value: i32 },
    Signal { value: i32 },
    Timeout,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    name: String,
    expected_relation: Relation,
    before: String,
    after: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Relation {
    Equal,
    Different,
}

pub(crate) fn is_evidence_name(name: &str) -> bool {
    name.starts_with(NAME_PREFIX)
}

pub(crate) fn validate(name: &str, bytes: &[u8], candidate_commit: &str) -> Result<(), String> {
    let Some(label) = name.strip_prefix(NAME_PREFIX) else {
        return Ok(());
    };
    if label.is_empty()
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "external-operation evidence names require a non-empty portable label".to_owned(),
        );
    }
    let evidence: Evidence = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid external-operation evidence `{name}`: {error}"))?;
    if evidence.schema != SCHEMA {
        return Err(format!(
            "external-operation evidence `{name}` must use schema `{SCHEMA}`"
        ));
    }
    require_commit(&evidence.candidate_commit, "external-operation candidate")?;
    if evidence.candidate_commit != candidate_commit {
        return Err(format!(
            "external-operation evidence `{name}` does not identify the exact candidate commit"
        ));
    }
    if evidence.counterfactual.trim().is_empty() {
        return Err(format!(
            "external-operation evidence `{name}` requires a counterfactual"
        ));
    }
    if evidence.operation.working_directory.trim().is_empty()
        || evidence.operation.working_directory.contains('\0')
        || evidence.operation.argv.is_empty()
        || evidence.operation.argv[0].trim().is_empty()
        || evidence
            .operation
            .argv
            .iter()
            .any(|value| value.contains('\0'))
    {
        return Err(format!(
            "external-operation evidence `{name}` requires a working directory and executable argv without NUL bytes"
        ));
    }
    validate_exit(name, &evidence.operation.expected_exit)?;
    validate_exit(name, &evidence.operation.observed_exit)?;
    if evidence.operation.expected_exit != evidence.operation.observed_exit {
        return Err(format!(
            "external-operation evidence `{name}` observed a different exit status than expected"
        ));
    }
    if evidence.observations.is_empty() {
        return Err(format!(
            "external-operation evidence `{name}` requires before/after observations"
        ));
    }
    let mut names = BTreeSet::new();
    for observation in evidence.observations {
        if observation.name.trim().is_empty()
            || !names.insert(observation.name.clone())
            || observation.before.trim().is_empty()
            || observation.after.trim().is_empty()
        {
            return Err(format!(
                "external-operation evidence `{name}` requires unique named non-empty observations"
            ));
        }
        let relation_matches = match observation.expected_relation {
            Relation::Equal => observation.before == observation.after,
            Relation::Different => observation.before != observation.after,
        };
        if !relation_matches {
            return Err(format!(
                "external-operation evidence `{name}` observation `{}` contradicts its expected relation",
                observation.name
            ));
        }
    }
    Ok(())
}

fn validate_exit(name: &str, status: &ExitStatus) -> Result<(), String> {
    let valid = match status {
        ExitStatus::Code { value } => *value >= 0,
        ExitStatus::Signal { value } => *value > 0,
        ExitStatus::Timeout => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "external-operation evidence `{name}` has an invalid exit status"
        ))
    }
}
