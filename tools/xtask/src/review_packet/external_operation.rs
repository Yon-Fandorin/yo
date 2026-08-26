use std::collections::BTreeSet;

use serde::Deserialize;

use crate::review_protocol::require_commit;

pub(crate) const NAME_PREFIX: &str = "external-operation/";
pub(crate) const SCHEMA: &str = "yo.external-operation-evidence/v1";

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

#[derive(Clone, Copy, Deserialize)]
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
    parse_and_validate(name, label, bytes, candidate_commit).map(drop)
}

pub(crate) fn validate_for_gate(
    name: &str,
    bytes: &[u8],
    candidate_commit: &str,
    expected_argv: &[String],
    requested_reuse: bool,
) -> Result<(), String> {
    let label = name.strip_prefix(NAME_PREFIX).ok_or_else(|| {
        format!("schema `{SCHEMA}` requires an `{NAME_PREFIX}<label>` validation name")
    })?;
    let evidence = parse_and_validate(name, label, bytes, candidate_commit)?;
    if requested_reuse {
        return Err("external-operation evidence cannot be reused".to_owned());
    }
    if evidence.operation.argv != expected_argv {
        return Err("external-operation argv does not match the gate request".to_owned());
    }
    Ok(())
}

fn parse_and_validate(
    name: &str,
    label: &str,
    bytes: &[u8],
    candidate_commit: &str,
) -> Result<Evidence, String> {
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
    for observation in &evidence.observations {
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
    Ok(evidence)
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate_for_gate;

    fn evidence(candidate: &str, argv: &[String]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": "yo.external-operation-evidence/v1",
            "candidate_commit": candidate,
            "operation": {
                "working_directory": ".",
                "argv": argv,
                "expected_exit": {"kind": "code", "value": 0},
                "observed_exit": {"kind": "code", "value": 0}
            },
            "counterfactual": "the operation must fail when behavior regresses",
            "observations": [{
                "name": "HEAD",
                "expected_relation": "equal",
                "before": candidate,
                "after": candidate
            }]
        }))
        .unwrap()
    }

    // gate 전용 검증은 기존 evidence 의미를 다시 검증한 뒤 exact argv와 비재사용
    // 선언까지 결속하여 다른 command의 green 결과로 바뀌지 않게 한다.
    #[test]
    fn gate_validation_binds_exact_argv_and_rejects_reuse() {
        let candidate = "a".repeat(40);
        let argv = vec!["cargo".to_owned(), "test".to_owned()];
        let bytes = evidence(&candidate, &argv);

        validate_for_gate(
            "external-operation/focused",
            &bytes,
            &candidate,
            &argv,
            false,
        )
        .unwrap();
        assert!(
            validate_for_gate(
                "external-operation/focused",
                &bytes,
                &candidate,
                &["cargo".to_owned(), "check".to_owned()],
                false,
            )
            .unwrap_err()
            .contains("argv does not match")
        );
        assert!(
            validate_for_gate(
                "external-operation/focused",
                &bytes,
                &candidate,
                &argv,
                true,
            )
            .unwrap_err()
            .contains("cannot be reused")
        );
    }
}
