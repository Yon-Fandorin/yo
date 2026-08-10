use super::*;
use crate::review_delta::model::{Disposition, PriorFinding};

fn hash(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn commit(byte: u8) -> String {
    format!("{byte:02x}").repeat(20)
}

fn prior(evidence: Vec<review_packet::VerifiedEvidence>) -> VerifiedReview {
    VerifiedReview {
        review_id: hash(1),
        manifest_path: "manifest.json".to_owned(),
        manifest_hash: hash(2),
        packet_path: "packet.md".to_owned(),
        packet_hash: hash(3),
        base_commit: commit(1),
        candidate_commit: commit(2),
        trusted_commit: commit(1),
        slice_contract_path: "slice-contract.json".to_owned(),
        slice_contract_hash: hash(4),
        validation_evidence: evidence,
        review_lenses: vec!["fresh-context".to_owned()],
        review_questions: vec!["Are the findings resolved?".to_owned()],
    }
}

fn finding(id: &str) -> FindingDisposition {
    FindingDisposition {
        finding_id: id.to_owned(),
        disposition: Disposition::Resolved,
        summary: "The replacement candidate covers this case.".to_owned(),
    }
}

fn prior_findings(ids: &[&str]) -> Captured {
    let value = PriorFindings {
        schema: PRIOR_FINDINGS_SCHEMA.to_owned(),
        review_id: hash(1),
        candidate_commit: commit(2),
        findings: ids
            .iter()
            .map(|id| PriorFinding {
                finding_id: (*id).to_owned(),
                summary: format!("Finding {id}"),
            })
            .collect(),
    };
    captured(
        "prior-findings.json".to_owned(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap()
}

// continuation에 finding, 양수 budget, replacement 전용 evidence가 모두 있어야
// 변경 후보를 검토하지 않은 빈 성공을 만들 수 없음을 확인한다.
#[test]
fn request_requires_findings_budget_and_affected_evidence() {
    let mut request = Request {
        schema: REQUEST_SCHEMA.to_owned(),
        prior_manifest_path: "manifest.json".to_owned(),
        prior_manifest_hash: hash(1),
        prior_findings_path: "findings.json".to_owned(),
        prior_findings_hash: hash(2),
        finding_dispositions: Vec::new(),
        reused_validation_evidence: Vec::new(),
        affected_validation_evidence: Vec::new(),
        delivery_profile: DELIVERY_PROFILE.to_owned(),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        max_managed_payload_tokens: 100,
    };

    assert_eq!(
        validate_request(&request).unwrap_err(),
        "at least one finding disposition is required"
    );
    request.finding_dispositions.push(finding("F1"));
    assert!(
        validate_request(&request)
            .unwrap_err()
            .contains("affected validation evidence")
    );
    request.affected_validation_evidence.push(EvidenceRequest {
        name: "focused".to_owned(),
        path: "focused.txt".to_owned(),
    });
    request.max_managed_payload_tokens = 0;
    assert_eq!(
        validate_request(&request).unwrap_err(),
        "managed payload token budget must be positive"
    );
}

// reviewer가 작성한 findings artifact의 모든 ID와 오직 그 ID만 disposition에
// 대응해야 적격 continuation이 됨을 확인한다.
#[test]
fn dispositions_must_match_the_exact_prior_finding_set() {
    let exact = prior_findings(&["F1", "F2"]);
    assert!(require_exact_finding_set(&exact, &[finding("F2"), finding("F1")]).is_ok());
    assert!(
        require_exact_finding_set(&exact, &[finding("F1")])
            .unwrap_err()
            .contains("exact prior finding ID set")
    );
    assert!(
        require_exact_finding_set(&exact, &[finding("F1"), finding("F3")])
            .unwrap_err()
            .contains("exact prior finding ID set")
    );
}

// 이전 evidence 이름을 빠짐없이 reused 또는 affected로 분류하고 두 분류가
// 겹치지 않아야 함을 확인한다.
#[test]
fn validation_capture_requires_complete_non_overlapping_classification() {
    let root = crate::test_support::unique_path("review-delta-evidence");
    std::fs::create_dir_all(&root).unwrap();
    let stable_path = root.join("stable.txt");
    let changed_path = root.join("changed.txt");
    std::fs::write(&stable_path, b"stable green\n").unwrap();
    std::fs::write(&changed_path, b"new focused green\n").unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: "stable".to_owned(),
        path: stable_path.to_string_lossy().into_owned(),
        hash: digest(b"stable green\n"),
    }]);

    assert!(
        capture_validation(&root, &prior, &[], &[])
            .unwrap_err()
            .contains("must be classified")
    );
    assert!(
        capture_validation(
            &root,
            &prior,
            &["stable".to_owned()],
            &[EvidenceRequest {
                name: "stable".to_owned(),
                path: changed_path.to_string_lossy().into_owned(),
            }],
        )
        .unwrap_err()
        .contains("both reused and affected")
    );
    let (reused, affected) =
        capture_validation(&root, &prior, &["stable".to_owned()], &[]).unwrap();
    assert_eq!(reused.len(), 1);
    assert!(affected.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

// 다음 evidence를 보관하는 순간 누적 제한을 넘으면 즉시 거부함을 확인한다.
#[test]
fn aggregate_evidence_limit_is_checked_incrementally() {
    let mut total = MAX_AGGREGATE_EVIDENCE_BYTES - 1;
    assert!(add_evidence_bytes(&mut total, 1).is_ok());
    assert!(
        add_evidence_bytes(&mut total, 1)
            .unwrap_err()
            .contains("aggregate validation evidence")
    );
}

// continuation은 prior identity와 exact findings를 포함하되 provider-neutral delta에
// 이전 packet 본문 전체를 다시 싣지 않음을 확인한다.
#[test]
fn packet_keeps_prior_identity_without_replaying_the_prior_packet() {
    let prior = prior(Vec::new());
    let prior_findings = prior_findings(&["F1"]);
    let inputs = Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest: captured("manifest.json".to_owned(), b"prior manifest".to_vec()).unwrap(),
        prior_packet: captured(
            "packet.md".to_owned(),
            b"PRIOR_PACKET_BODY_MUST_NOT_BE_REPLAYED".to_vec(),
        )
        .unwrap(),
        prior_findings,
        prior,
        replacement_candidate: commit(3),
        delta: captured(
            "git-delta.patch".to_owned(),
            b"diff --git a/file b/file\n".to_vec(),
        )
        .unwrap(),
        slice_contract: captured("slice-contract.json".to_owned(), b"slice contract".to_vec())
            .unwrap(),
        findings: vec![finding("F1")],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "focused".to_owned(),
            artifact: captured("focused.txt".to_owned(), b"focused green\n".to_vec()).unwrap(),
        }],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    };
    let plan = build_plan(&inputs);
    let packet = render_packet(&hash(9), &plan, &inputs).unwrap();
    let text = std::str::from_utf8(&packet).unwrap();

    assert!(text.contains(&inputs.prior.review_id));
    assert!(text.contains("F1"));
    assert!(text.contains("focused green"));
    assert!(text.contains("diff --git"));
    assert!(!text.contains("PRIOR_PACKET_BODY_MUST_NOT_BE_REPLAYED"));
}

// 후속 continuation은 매번 원본 candidate부터 diff하지 않고 직전 replacement를
// chain head로 사용함을 확인한다.
#[test]
fn later_continuation_starts_from_the_previous_replacement() {
    let mut chain_head = prior(Vec::new());
    chain_head.review_id = hash(8);
    chain_head.candidate_commit = commit(3);
    chain_head.manifest_path = "first-delta/manifest.json".to_owned();
    chain_head.packet_path = "first-delta/packet.md".to_owned();
    let inputs = Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest: captured("first-delta/manifest.json".to_owned(), b"manifest".to_vec())
            .unwrap(),
        prior_packet: captured("first-delta/packet.md".to_owned(), b"packet".to_vec()).unwrap(),
        prior_findings: {
            let mut value = prior_findings(&["F2"]);
            let mut parsed: PriorFindings = serde_json::from_slice(&value.bytes).unwrap();
            parsed.review_id = hash(8);
            parsed.candidate_commit = commit(3);
            value.bytes = serde_json::to_vec(&parsed).unwrap();
            value.hash = digest(&value.bytes);
            value
        },
        prior: chain_head,
        replacement_candidate: commit(4),
        delta: captured("git-delta.patch".to_owned(), b"second hop".to_vec()).unwrap(),
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        findings: vec![finding("F2")],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "second-hop".to_owned(),
            artifact: captured("second-hop.txt".to_owned(), b"green".to_vec()).unwrap(),
        }],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    };

    let plan = build_plan(&inputs);
    assert_eq!(plan.prior_review_id, hash(8));
    assert_eq!(plan.prior_candidate_commit, commit(3));
    assert_eq!(plan.replacement_candidate_commit, commit(4));
}

// capture와 최종 publish에서 쓰는 branch guard가 같은 commit을 가리키더라도
// 다른 branch identity는 거부함을 확인한다.
#[test]
fn expected_branch_identity_is_not_just_a_commit_check() {
    let repository = crate::test_support::TestRepository::new("review-delta-branch");
    repository.write("file.txt", "base\n");
    repository.git(["add", "file.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    repository.git(["switch", "-c", "slice/direct/review-delta"]);
    require_expected_branch(&repository.path, "refs/heads/develop", "review-delta").unwrap();
    repository.git(["switch", "-c", "unrelated"]);
    assert!(
        require_expected_branch(&repository.path, "refs/heads/develop", "review-delta",)
            .unwrap_err()
            .contains("expected refs/heads/slice/direct/review-delta")
    );
}

// 생성과 재생이 같은 validator를 사용해, 자체 일관된 bytes만으로 producer의
// 적격성 규칙을 우회할 수 없음을 확인한다.
#[test]
fn transition_validator_rejects_noncanonical_or_unrelated_evidence() {
    let candidate = commit(3);
    let previous = review_packet::VerifiedEvidence {
        name: "baseline".to_owned(),
        path: "baseline.txt".to_owned(),
        hash: digest(b"old evidence"),
    };
    let prior = prior(vec![previous.clone()]);
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();
    let bound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(
            "new-baseline.txt".to_owned(),
            format!("Candidate: {candidate}\npassed\n").into_bytes(),
        )
        .unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            std::slice::from_ref(&bound),
        )
        .is_ok()
    );

    let mut blank = finding("F1");
    blank.summary.clear();
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[blank],
            &[],
            std::slice::from_ref(&bound),
        )
        .is_err()
    );
    let extra_reused = NamedCaptured {
        name: "unrelated".to_owned(),
        artifact: captured("extra.txt".to_owned(), b"extra".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[extra_reused],
            std::slice::from_ref(&bound),
        )
        .unwrap_err()
        .contains("unknown reused")
    );
    let unchanged = NamedCaptured {
        name: previous.name.clone(),
        artifact: captured(previous.path.clone(), b"old evidence".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            &[unchanged],
        )
        .unwrap_err()
        .contains("unchanged")
    );
    let unbound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured("new.txt".to_owned(), b"passed".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            &[unbound],
        )
        .unwrap_err()
        .contains("does not bind")
    );
}

// 동일한 게시 artifact의 여러 경로 표기를 하나의 repository-relative identity로
// 정규화해 alias가 별도 manifest identity를 만들지 않음을 확인한다.
#[test]
fn published_artifact_paths_are_canonicalized_before_identity_capture() {
    let root = crate::test_support::unique_path("review-delta-canonical-path");
    std::fs::create_dir_all(root.join("store")).unwrap();
    std::fs::write(root.join("store/manifest.json"), b"manifest\n").unwrap();
    let direct = capture_published(
        &root,
        &root.join("store/manifest.json"),
        "manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    let dotted = capture_published(
        &root,
        &root.join("store/../store/manifest.json"),
        "manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    assert_eq!(direct.path, "store/manifest.json");
    assert_eq!(direct.path, dotted.path);
    assert_eq!(direct.hash, dotted.hash);
    std::fs::remove_dir_all(root).unwrap();
}

fn repository_head(repository: &Path) -> String {
    git::output_in(repository, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned()
}

fn write_findings(path: &Path, review_id: &str, candidate: &str, finding_id: &str) -> Captured {
    let value = PriorFindings {
        schema: PRIOR_FINDINGS_SCHEMA.to_owned(),
        review_id: review_id.to_owned(),
        candidate_commit: candidate.to_owned(),
        findings: vec![PriorFinding {
            finding_id: finding_id.to_owned(),
            summary: "review finding".to_owned(),
        }],
    };
    let mut bytes = serde_json::to_vec_pretty(&value).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, &bytes).unwrap();
    captured(path.to_string_lossy().into_owned(), bytes).unwrap()
}

fn publish_delta_fixture(repository: &Path, inputs: &Inputs) -> (PathBuf, String, &'static str) {
    let plan = build_plan(inputs);
    let id = domain_digest(REVIEW_DELTA_ID_DOMAIN, &serde_json::to_vec(&plan).unwrap());
    let packet = render_packet(&id, &plan, inputs).unwrap();
    let manifest = build_manifest(
        id.clone(),
        plan,
        inputs,
        digest(&packet),
        count_tokens(&packet).unwrap(),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    manifest_bytes.push(b'\n');
    let directory = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(id.strip_prefix("sha256:").unwrap());
    let status = storage::publish(&directory, &packet, &manifest_bytes, || Ok(())).unwrap();
    (
        directory.join("manifest.json"),
        digest(&manifest_bytes),
        status,
    )
}

fn delta_inputs(
    repository: &Path,
    prior: VerifiedReview,
    replacement: &str,
    finding_id: &str,
    evidence_body: &[u8],
    evidence_suffix: &str,
) -> Inputs {
    let prior_manifest = capture_published(
        repository,
        &resolve_input_path(repository, &prior.manifest_path),
        "prior manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    let prior_packet = capture_published(
        repository,
        &resolve_input_path(repository, &prior.packet_path),
        "prior packet",
        MAX_PACKET_BYTES,
    )
    .unwrap();
    let findings_path = repository.join(format!(".local-exclude/findings-{evidence_suffix}.json"));
    let prior_findings = write_findings(
        &findings_path,
        &prior.review_id,
        &prior.candidate_commit,
        finding_id,
    );
    let evidence_path = repository.join(format!(".local-exclude/evidence-{evidence_suffix}.txt"));
    std::fs::write(&evidence_path, evidence_body).unwrap();
    let contract_path = resolve_input_path(repository, &prior.slice_contract_path);
    Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest,
        prior_packet,
        prior_findings,
        delta: captured(
            "git-delta.patch".to_owned(),
            capture_delta(repository, &prior.candidate_commit, replacement).unwrap(),
        )
        .unwrap(),
        slice_contract: capture_file(&contract_path, "contract").unwrap(),
        findings: vec![finding(finding_id)],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "baseline".to_owned(),
            artifact: capture_file(&evidence_path, "evidence").unwrap(),
        }],
        prior,
        replacement_candidate: replacement.to_owned(),
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 20_000,
    }
}

// synthetic original review에서 두 번의 published delta를 재생해 중앙 verifier가
// 실제 재귀 chain과 alias reuse를 수락하고 canonical-but-ineligible evidence는
// 거부하는지 끝까지 확인한다.
#[test]
fn recursive_chain_verifier_replays_two_hops_and_rejects_ineligible_artifacts() {
    let repository = crate::test_support::TestRepository::new("review-delta-chain-e2e");
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("owned.txt", "candidate a\n");
    repository.git(["add", ".gitignore", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate a"]);
    let candidate_a = repository_head(&repository.path);
    repository.git(["switch", "-c", "slice/direct/review-delta-chain"]);

    let seed_directory = repository
        .path
        .join(".local-exclude/methexis/slice-reviews/seed");
    std::fs::create_dir_all(&seed_directory).unwrap();
    let seed_manifest_path = seed_directory.join("manifest.json");
    let seed_manifest_bytes = b"{\"schema\":\"yo.slice-review-manifest/v1\"}\n";
    std::fs::write(&seed_manifest_path, seed_manifest_bytes).unwrap();
    let seed_packet_path = seed_directory.join("packet.md");
    std::fs::write(&seed_packet_path, b"seed packet\n").unwrap();
    let contract_path = repository.path.join(".local-exclude/contract.json");
    std::fs::write(&contract_path, b"contract\n").unwrap();
    let baseline_path = repository.path.join(".local-exclude/evidence-a.txt");
    std::fs::write(&baseline_path, format!("Candidate: {candidate_a}\n")).unwrap();
    let seed = VerifiedReview {
        review_id: hash(90),
        manifest_path: relative(&repository.path, &seed_manifest_path),
        manifest_hash: digest(seed_manifest_bytes),
        packet_path: relative(&repository.path, &seed_packet_path),
        packet_hash: digest(b"seed packet\n"),
        base_commit: candidate_a.clone(),
        candidate_commit: candidate_a.clone(),
        trusted_commit: candidate_a.clone(),
        slice_contract_path: contract_path.to_string_lossy().into_owned(),
        slice_contract_hash: digest(b"contract\n"),
        validation_evidence: vec![review_packet::VerifiedEvidence {
            name: "baseline".to_owned(),
            path: baseline_path.to_string_lossy().into_owned(),
            hash: digest(format!("Candidate: {candidate_a}\n").as_bytes()),
        }],
        review_lenses: vec!["fresh-context".to_owned()],
        review_questions: vec!["Is the chain eligible?".to_owned()],
    };
    let verify_seed = |_: &Path, path: &Path, expected: &str| {
        if std::fs::canonicalize(path).unwrap()
            == std::fs::canonicalize(&seed_manifest_path).unwrap()
            && expected == seed.manifest_hash
        {
            Ok(seed.clone())
        } else {
            Err("unexpected seed review".to_owned())
        }
    };

    repository.write("owned.txt", "candidate b\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate b"]);
    let candidate_b = repository_head(&repository.path);
    let first_inputs = delta_inputs(
        &repository.path,
        seed.clone(),
        &candidate_b,
        "F1",
        format!("Candidate: {candidate_b}\npassed\n").as_bytes(),
        "b",
    );
    validate_transition(
        &first_inputs.prior,
        &candidate_b,
        &first_inputs.delta,
        &first_inputs.findings,
        &first_inputs.reused_validation,
        &first_inputs.affected_validation,
    )
    .unwrap();
    let (first_manifest, first_hash, created) =
        publish_delta_fixture(&repository.path, &first_inputs);
    assert_eq!(created, "created");
    assert_eq!(
        publish_delta_fixture(&repository.path, &first_inputs).2,
        "reused"
    );
    let first = verify_chain_head_with(
        &repository.path,
        &first_manifest,
        &first_hash,
        &mut BTreeSet::new(),
        0,
        &verify_seed,
    )
    .unwrap();
    assert_eq!(first.candidate_commit, candidate_b);

    repository.write("owned.txt", "candidate c\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate c"]);
    let candidate_c = repository_head(&repository.path);
    let second_inputs = delta_inputs(
        &repository.path,
        first,
        &candidate_c,
        "F2",
        format!("Candidate: {candidate_c}\npassed\n").as_bytes(),
        "c",
    );
    let (second_manifest, second_hash, _) = publish_delta_fixture(&repository.path, &second_inputs);
    let alias = second_manifest.parent().unwrap().join("./manifest.json");
    let second = verify_chain_head_with(
        &repository.path,
        &alias,
        &second_hash,
        &mut BTreeSet::new(),
        0,
        &verify_seed,
    )
    .unwrap();
    assert_eq!(second.candidate_commit, candidate_c);
    assert_eq!(
        second.manifest_path,
        relative(&repository.path, &second_manifest)
    );

    let invalid_inputs = delta_inputs(
        &repository.path,
        seed.clone(),
        &candidate_c,
        "F3",
        b"passed without candidate binding\n",
        "invalid",
    );
    let (invalid_manifest, invalid_hash, _) =
        publish_delta_fixture(&repository.path, &invalid_inputs);
    assert!(
        verify_chain_head_with(
            &repository.path,
            &invalid_manifest,
            &invalid_hash,
            &mut BTreeSet::new(),
            0,
            &verify_seed,
        )
        .unwrap_err()
        .contains("does not bind")
    );
}
