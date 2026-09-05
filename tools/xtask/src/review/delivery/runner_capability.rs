use std::path::Path;

use serde::Deserialize;

use crate::{bounded_file, grok_outer_sandbox};

const MANIFEST_PATH: &str = "crates/backends/delegated-grok/review-runner-capabilities.json";
const MANIFEST_LIMIT: usize = 16 * 1024;
const MANIFEST_SCHEMA: &str = "yo.delegated-review-runner-capabilities/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    host: String,
    execution_isolations: Vec<String>,
}

/// Requires the trusted current-develop source used to build `yo` to advertise
/// the non-native review boundary before an immutable delivery claim exists.
pub(super) fn require(
    integration: &Path,
    host: &str,
    execution_isolation: Option<&str>,
) -> Result<(), String> {
    let Some(execution_isolation) = execution_isolation else {
        return Ok(());
    };
    if execution_isolation == grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE {
        return Ok(());
    }
    if host != "grok" || execution_isolation != grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE {
        return Err(format!(
            "no trusted current-develop runner capability contract exists for `{host}` isolation `{execution_isolation}`"
        ));
    }

    let path = integration.join(MANIFEST_PATH);
    let bytes = bounded_file::read_regular(
        &path,
        MANIFEST_LIMIT,
        "trusted current-develop delegated review runner capabilities",
    )
    .map_err(|error| bootstrap_failure(execution_isolation, error))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
        bootstrap_failure(
            execution_isolation,
            format!("invalid capability manifest: {error}"),
        )
    })?;
    if manifest.schema != MANIFEST_SCHEMA
        || manifest.host != host
        || manifest.execution_isolations
            != [
                grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE,
                execution_isolation,
            ]
    {
        return Err(bootstrap_failure(
            execution_isolation,
            "capability manifest does not exactly advertise the selected Grok isolation",
        ));
    }
    Ok(())
}

fn bootstrap_failure(execution_isolation: &str, detail: impl std::fmt::Display) -> String {
    format!(
        "trusted current-develop Yo does not support selected execution isolation `{execution_isolation}`: {detail}; select a disjoint already-active reviewer before an immutable claim, then dogfood this isolation after integration"
    )
}

#[cfg(test)]
mod tests {
    use super::require;
    use crate::{grok_outer_sandbox, test_support::TestRepository};

    // native 실행은 기존 current-develop 계약이므로 새 manifest 없이 그대로 유지됩니다.
    #[test]
    fn native_isolation_preserves_the_existing_runner_contract() {
        let repository = TestRepository::new("review-runner-native-capability");
        require(
            &repository.path,
            "grok",
            Some(grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE),
        )
        .unwrap();
    }

    // outer isolation을 처음 구현하는 후보가 자기 코드를 runner로 사용하지 못하도록
    // trusted develop에 manifest가 없으면 claim 전에 명확한 bootstrap 실패를 냅니다.
    #[test]
    fn outer_isolation_requires_a_trusted_develop_capability() {
        let repository = TestRepository::new("review-runner-missing-outer-capability");
        let error = require(
            &repository.path,
            "grok",
            Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
        )
        .unwrap_err();
        assert!(error.contains("select a disjoint already-active reviewer"));
    }

    // exact manifest만 허용해 다른 host, schema, isolation 목록을 지원 증거로 확대 해석하지
    // 않습니다.
    #[test]
    fn outer_isolation_accepts_only_the_exact_manifest() {
        let repository = TestRepository::new("review-runner-exact-outer-capability");
        repository.write(
            "crates/backends/delegated-grok/review-runner-capabilities.json",
            &format!(
                "{{\"schema\":\"yo.delegated-review-runner-capabilities/v1alpha1\",\"host\":\"grok\",\"execution_isolations\":[\"{}\",\"{}\"]}}\n",
                grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE,
                grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE
            ),
        );
        require(
            &repository.path,
            "grok",
            Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
        )
        .unwrap();

        repository.write(
            "crates/backends/delegated-grok/review-runner-capabilities.json",
            "{\"schema\":\"yo.delegated-review-runner-capabilities/v1alpha1\",\"host\":\"codex\",\"execution_isolations\":[\"grok-native-read-only/v1alpha1\",\"yo-bwrap-read-only/v1alpha1\"]}\n",
        );
        assert!(
            require(
                &repository.path,
                "grok",
                Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
            )
            .is_err()
        );
    }
}
