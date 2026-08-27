use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{bounded_file, git, slice_contract, slice_worktree};

const RESULT_SCHEMA: &str = "yo.slice-status/v1alpha2";
const JSON_LIMIT: usize = 8 * 1024 * 1024;
const MAX_JSON_FILES: usize = 256;
const MAX_SCAN_DEPTH: usize = 6;

#[derive(Clone, Debug)]
pub(crate) struct SliceState {
    pub(crate) worktree: PathBuf,
    pub(crate) branch: String,
    pub(crate) head: String,
    pub(crate) bound: slice_contract::BoundSlice,
    pub(crate) clean: bool,
}

#[derive(Default)]
struct Artifacts {
    validation_summaries: usize,
    gate_requests: usize,
    claims: usize,
    delivery_receipts: usize,
    review_rounds: usize,
    durable_requests: u64,
    prior_findings: usize,
}

struct ReviewLineage {
    packets: usize,
    latest_candidate: Option<String>,
    status: &'static str,
    current_review_ids: BTreeSet<String>,
    latest_review_ids: BTreeSet<String>,
}

#[derive(Default)]
struct ScanBudget {
    json_files: usize,
}

#[derive(Serialize)]
struct ResultDocument {
    schema: &'static str,
    ok: bool,
    slice: String,
    branch: String,
    base_commit: String,
    candidate_commit: String,
    clean: bool,
    review_lineage: &'static str,
    review_packets: usize,
    review_rounds: usize,
    latest_packet_candidate: Option<String>,
    validation_summaries: usize,
    gate_requests: usize,
    delivery_claims: usize,
    delivery_receipts: usize,
    durable_external_requests: u64,
    next_action: &'static str,
}

pub(crate) fn run(repository: &Path, slice: &str) -> Result<(), String> {
    let state = locate(repository, slice)?;
    let workspace = slice_worktree::workspace_root(repository)?;
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let mut budget = ScanBudget::default();
    let reviews = scan_review_lineage(&state, &workspace, &mut budget)?;
    let artifacts = scan_coordination(
        &coordination,
        &state.head,
        &reviews.current_review_ids,
        &reviews.latest_review_ids,
        &mut budget,
    )?;
    let next_action = next_action(&state, &reviews, &artifacts);
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        slice: slice.to_owned(),
        branch: state.branch,
        base_commit: state.bound.base,
        candidate_commit: state.head,
        clean: state.clean,
        review_lineage: reviews.status,
        review_packets: reviews.packets,
        review_rounds: artifacts.review_rounds,
        latest_packet_candidate: reviews.latest_candidate,
        validation_summaries: artifacts.validation_summaries,
        gate_requests: artifacts.gate_requests,
        delivery_claims: artifacts.claims,
        delivery_receipts: artifacts.delivery_receipts,
        durable_external_requests: artifacts.durable_requests,
        next_action,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode compact Slice status: {error}"))?
    );
    Ok(())
}

fn next_action(state: &SliceState, reviews: &ReviewLineage, artifacts: &Artifacts) -> &'static str {
    if !state.clean {
        "clean_candidate"
    } else if reviews.status == "broken" {
        "restore_review_lineage"
    } else if reviews.current_review_ids.is_empty() {
        if reviews
            .latest_candidate
            .as_deref()
            .is_some_and(|candidate| candidate != state.head)
            && artifacts.prior_findings > 0
        {
            "review_delta"
        } else {
            "build_review"
        }
    } else if artifacts.review_rounds == 0 {
        "deliver_current_review"
    } else if artifacts.gate_requests == 0 {
        "prepare_gate"
    } else {
        "run_gate"
    }
}

pub(crate) fn locate(repository: &Path, slice: &str) -> Result<SliceState, String> {
    validate_slice_name(slice)?;
    let mut matches = Vec::new();
    for worktree in slice_worktree::worktrees(repository)? {
        let Some(branch) = worktree.branch.as_deref() else {
            continue;
        };
        if !branch_names_slice(branch, slice) {
            continue;
        }
        let bound = slice_contract::bound_slice(&worktree.path)?;
        if bound.slice == slice {
            matches.push((worktree, bound));
        }
    }
    let (worktree, bound) = match matches.as_slice() {
        [(worktree, bound)] => (worktree, bound),
        [] => return Err(format!("no registered Slice worktree found for `{slice}`")),
        _ => {
            return Err(format!(
                "multiple registered Slice worktrees match `{slice}`"
            ));
        },
    };
    let clean = git::output_bytes_in(
        &worktree.path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        false,
    )?
    .is_empty();
    Ok(SliceState {
        worktree: worktree.path.clone(),
        branch: worktree
            .branch
            .clone()
            .expect("matched Slice worktree has a branch"),
        head: worktree.head.clone(),
        bound: bound.clone(),
        clean,
    })
}

fn branch_names_slice(branch: &str, slice: &str) -> bool {
    branch == format!("refs/heads/slice/direct/{slice}")
        || branch
            .strip_prefix("refs/heads/slice/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(wave, name)| !wave.is_empty() && name == slice)
}

fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        Err("Slice name must be one non-empty branch segment".to_owned())
    } else {
        Ok(())
    }
}

fn scan_coordination(
    root: &Path,
    candidate: &str,
    current_review_ids: &BTreeSet<String>,
    latest_review_ids: &BTreeSet<String>,
    budget: &mut ScanBudget,
) -> Result<Artifacts, String> {
    if !root.exists() {
        return Ok(Artifacts::default());
    }
    let mut files = Vec::new();
    collect_json(root, 0, &mut files, budget)?;
    let mut found = Artifacts::default();
    let mut values = Vec::new();
    let mut current_request_ids = BTreeSet::new();
    for path in files {
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "Slice coordination JSON")?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(schema) = value.get("schema").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if schema.starts_with("yo.validation-run-summary/") {
            if value.get("head_commit").and_then(serde_json::Value::as_str) == Some(candidate) {
                found.validation_summaries += 1;
            }
        } else if schema.starts_with("yo.slice-gate-request/") {
            if value
                .get("candidate_commit")
                .and_then(serde_json::Value::as_str)
                == Some(candidate)
            {
                found.gate_requests += 1;
            }
        } else if schema == "yo.slice-review-findings/v1"
            && value
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|review_id| latest_review_ids.contains(review_id))
        {
            found.prior_findings += 1;
        } else if schema.contains("delivery-claim/")
            && value
                .get("candidate_commit")
                .and_then(serde_json::Value::as_str)
                == Some(candidate)
        {
            found.claims += 1;
            if let Some(request_id) = value.get("request_id").and_then(serde_json::Value::as_str) {
                current_request_ids.insert(request_id.to_owned());
            }
        }
        values.push(value);
    }
    let mut completed_reviews = BTreeSet::new();
    for value in values {
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if schema.contains("delivery-receipt/")
            && let Some(review_id) = value
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .filter(|review_id| current_review_ids.contains(*review_id))
        {
            found.delivery_receipts += 1;
            completed_reviews.insert(review_id.to_owned());
        }
        if value
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|request_id| current_request_ids.contains(request_id))
        {
            found.durable_requests += value
                .get("durable_host_request_count")
                .or_else(|| value.get("durable_provider_request_count"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
        }
    }
    found.review_rounds = completed_reviews.len();
    Ok(found)
}

fn scan_review_lineage(
    state: &SliceState,
    workspace: &Path,
    budget: &mut ScanBudget,
) -> Result<ReviewLineage, String> {
    let mut files = Vec::new();
    for root in [
        state.worktree.join(".local-exclude/methexis"),
        workspace.join(".local-exclude/methexis"),
    ] {
        if root.exists() {
            collect_json(&root, 0, &mut files, budget)?;
        }
    }
    files.sort();
    files.dedup();
    let mut candidates = BTreeSet::new();
    let mut review_ids = BTreeMap::<String, BTreeSet<String>>::new();
    let mut current_review_ids = BTreeSet::new();
    let mut broken = false;
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
            continue;
        }
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "published review manifest")?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if value
            .pointer("/plan/slice_contract/hash")
            .and_then(serde_json::Value::as_str)
            != Some(state.bound.contract_id.as_str())
        {
            continue;
        }
        let candidate = value
            .pointer("/plan/candidate_commit")
            .or_else(|| value.pointer("/plan/replacement_candidate_commit"))
            .and_then(serde_json::Value::as_str);
        let Some(candidate) = candidate else { continue };
        if !is_commit(candidate) {
            broken = true;
            continue;
        }
        candidates.insert(candidate.to_owned());
        if let Some(review_id) = value
            .get("review_id")
            .or_else(|| value.get("review_delta_id"))
            .and_then(serde_json::Value::as_str)
        {
            review_ids
                .entry(candidate.to_owned())
                .or_default()
                .insert(review_id.to_owned());
            if candidate == state.head {
                current_review_ids.insert(review_id.to_owned());
            }
        }
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            broken = true;
        }
    }
    let mut latest = None;
    let mut smallest_distance = u64::MAX;
    for candidate in &candidates {
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            continue;
        }
        let count = git::trusted_output_in(
            &state.worktree,
            &[
                "rev-list",
                "--count",
                &format!("{candidate}..{}", state.head),
            ],
        )?;
        let distance = count
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("Git returned invalid review distance: {error}"))?;
        if distance < smallest_distance {
            smallest_distance = distance;
            latest = Some(candidate.clone());
        }
    }
    let latest_review_ids = latest
        .as_ref()
        .and_then(|candidate| review_ids.get(candidate))
        .cloned()
        .unwrap_or_default();
    Ok(ReviewLineage {
        packets: candidates.len(),
        latest_candidate: latest,
        status: if broken { "broken" } else { "preserved" },
        current_review_ids,
        latest_review_ids,
    })
}

fn collect_json(
    directory: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
    budget: &mut ScanBudget,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "cannot inspect status path {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot scan status path {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read status path {}: {error}", directory.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect status entry {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_json(&path, depth + 1, output, budget)?;
        } else if metadata.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            if budget.json_files == MAX_JSON_FILES {
                return Err(format!(
                    "compact Slice status exceeded its global {MAX_JSON_FILES}-JSON-file scan limit"
                ));
            }
            budget.json_files += 1;
            output.push(path);
        }
    }
    Ok(())
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRepository;

    fn state(head: &str) -> SliceState {
        SliceState {
            worktree: PathBuf::from("/tmp/example"),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: head.to_owned(),
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base: "base".to_owned(),
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: PathBuf::from("binding"),
                contract_path: PathBuf::from("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        }
    }

    // 현재 후보 packet이 이미 있으면 같은 packet을 다시 만들라고 하지 않고 delivery로,
    // 이전 후보만 있으면 전체 packet 대신 review delta로 다음 행동을 구분합니다.
    #[test]
    fn next_action_reuses_current_review_or_selects_delta() {
        let current = ReviewLineage {
            packets: 1,
            latest_candidate: Some("current".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        };
        assert_eq!(
            next_action(&state("current"), &current, &Artifacts::default()),
            "deliver_current_review"
        );

        let prior = ReviewLineage {
            packets: 1,
            latest_candidate: Some("prior".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::new(),
            latest_review_ids: BTreeSet::from(["sha256:prior".to_owned()]),
        };
        assert_eq!(
            next_action(
                &state("current"),
                &prior,
                &Artifacts {
                    prior_findings: 1,
                    ..Artifacts::default()
                },
            ),
            "review_delta"
        );
        assert_eq!(
            next_action(&state("current"), &prior, &Artifacts::default()),
            "build_review"
        );
    }

    // receipt와 current-candidate gate가 있으면 review/gate 생성을 반복하지 않고 기존
    // exact gate 실행으로 바로 이어져 content-addressed reuse 경계를 보존합니다.
    #[test]
    fn next_action_reuses_existing_gate_inputs() {
        let reviews = ReviewLineage {
            packets: 1,
            latest_candidate: Some("current".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        };
        let artifacts = Artifacts {
            review_rounds: 1,
            gate_requests: 1,
            ..Artifacts::default()
        };

        assert_eq!(
            next_action(&state("current"), &reviews, &artifacts),
            "run_gate"
        );
    }

    // Slice 이름과 branch matcher는 임의 경로나 다른 Wave의 접두사를 느슨하게
    // 받아들이지 않고 direct 또는 정확한 한 Wave branch만 선택합니다.
    #[test]
    fn branch_matching_is_exact() {
        assert!(branch_names_slice(
            "refs/heads/slice/direct/example",
            "example"
        ));
        assert!(branch_names_slice(
            "refs/heads/slice/wave-a/example",
            "example"
        ));
        assert!(!branch_names_slice(
            "refs/heads/slice/wave-a/example/extra",
            "example"
        ));
        assert!(validate_slice_name("../example").is_err());
    }

    // 이미 발행한 후보를 reset으로 버리고 다른 child를 만들면 manifest 자체가 남아
    // 있어도 ancestry guard가 rewritten history를 preserved로 오인하지 않습니다.
    #[test]
    fn review_lineage_detects_replaced_candidate() {
        let repository = TestRepository::new("slice-status-lineage");
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write("tracked.txt", "reviewed\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "reviewed"]);
        let reviewed = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.git(["reset", "--hard", &base]);
        repository.write("tracked.txt", "replacement\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "replacement"]);
        let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write(
            ".local-exclude/methexis/review/manifest.json",
            &format!(
                "{{\"plan\":{{\"candidate_commit\":\"{reviewed}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
            ),
        );
        let state = SliceState {
            worktree: repository.path.clone(),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: replacement,
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base,
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: repository.path.join("binding"),
                contract_path: repository.path.join("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        };

        let reviews =
            scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
        assert_eq!(reviews.packets, 1);
        assert_eq!(reviews.latest_candidate, None);
        assert_eq!(reviews.status, "broken");
    }

    // 이전 후보의 validation/gate/claim 파일이 coordination에 남아 있어도 현재 HEAD의
    // 완료 수로 합산하지 않아 compact status가 stale progress를 만들지 않습니다.
    #[test]
    fn coordination_counts_only_candidate_bound_progress() {
        let root = crate::test_support::unique_path("slice-status-coordination");
        fs::create_dir_all(&root).unwrap();
        for (name, value) in [
            (
                "current-validation.json",
                serde_json::json!({
                    "schema": "yo.validation-run-summary/v1alpha2",
                    "head_commit": "current"
                }),
            ),
            (
                "stale-validation.json",
                serde_json::json!({
                    "schema": "yo.validation-run-summary/v1alpha2",
                    "head_commit": "stale"
                }),
            ),
            (
                "stale-gate.json",
                serde_json::json!({
                    "schema": "yo.slice-gate-request/v1alpha1",
                    "candidate_commit": "stale"
                }),
            ),
            (
                "current-receipt.json",
                serde_json::json!({
                    "schema": "yo.external-review-delivery-receipt/v1",
                    "review_id": "sha256:current-review"
                }),
            ),
            (
                "prior-findings.json",
                serde_json::json!({
                    "schema": "yo.slice-review-findings/v1",
                    "review_id": "sha256:current-review",
                    "candidate_commit": "reviewed"
                }),
            ),
            (
                "stale-receipt.json",
                serde_json::json!({
                    "schema": "yo.external-review-delivery-receipt/v1",
                    "review_id": "sha256:stale-review"
                }),
            ),
        ] {
            fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        }

        let artifacts = scan_coordination(
            &root,
            "current",
            &BTreeSet::from(["sha256:current-review".to_owned()]),
            &BTreeSet::from(["sha256:current-review".to_owned()]),
            &mut ScanBudget::default(),
        )
        .unwrap();
        assert_eq!(artifacts.validation_summaries, 1);
        assert_eq!(artifacts.gate_requests, 0);
        assert_eq!(artifacts.delivery_receipts, 1);
        assert_eq!(artifacts.review_rounds, 1);
        assert_eq!(artifacts.prior_findings, 1);
        fs::remove_dir_all(root).unwrap();
    }

    // original manifest의 review_id와 finding-resolution manifest의 review_delta_id는
    // 서로 다른 wire 필드이므로 현재 replacement 후보의 receipt 연결은 둘 다 읽습니다.
    #[test]
    fn review_lineage_identifies_current_delta_review() {
        let repository = TestRepository::new("slice-status-delta-review");
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write("tracked.txt", "replacement\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "replacement"]);
        let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write(
            ".local-exclude/methexis/delta/manifest.json",
            &format!(
                "{{\"review_delta_id\":\"sha256:delta\",\"plan\":{{\"replacement_candidate_commit\":\"{replacement}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
            ),
        );
        let state = SliceState {
            worktree: repository.path.clone(),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: replacement,
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base,
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: repository.path.join("binding"),
                contract_path: repository.path.join("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        };

        let reviews =
            scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
        assert_eq!(
            reviews.current_review_ids,
            BTreeSet::from(["sha256:delta".to_owned()])
        );
    }

    // manifest와 coordination은 하나의 전역 예산을 공유해 두 scan이 각각 256개를
    // 읽는 방식으로 문서화된 bounded 입력 한도를 우회하지 않습니다.
    #[test]
    fn json_scan_budget_is_global_across_roots() {
        let first = crate::test_support::unique_path("slice-status-budget-first");
        let second = crate::test_support::unique_path("slice-status-budget-second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        for index in 0..128 {
            fs::write(first.join(format!("{index:03}.json")), b"{}\n").unwrap();
            fs::write(second.join(format!("{index:03}.json")), b"{}\n").unwrap();
        }
        let mut budget = ScanBudget::default();
        let mut files = Vec::new();
        collect_json(&first, 0, &mut files, &mut budget).unwrap();
        collect_json(&second, 0, &mut files, &mut budget).unwrap();
        fs::write(second.join("overflow.json"), b"{}\n").unwrap();
        assert!(
            collect_json(&second, 0, &mut Vec::new(), &mut budget)
                .unwrap_err()
                .contains("global 256-JSON-file")
        );
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }
}
