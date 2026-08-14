use std::{path::Path, process::Command};

use serde_json::json;

use super::{
    super::{
        REVIEW_ID_DOMAIN,
        canonical::build_plan,
        model::{
            PREFLIGHT_RESULT_SCHEMA_V1, PreflightPacket, PreflightResultRecord,
            SECTION_TOKEN_ACCOUNTING,
        },
        preflight,
        render::{render_packet, render_packet_with_measurements},
        set_preflight_test_hook,
    },
    support::{sample_inputs, sample_inputs_v1_alpha1},
};
use crate::{
    review_protocol::domain_digest,
    slice_contract,
    test_support::{TestRepository, unique_path},
};

const CHILD_REQUEST: &str = "YO_XTASK_PREFLIGHT_CHILD_REQUEST";
const CHILD_OUTPUT: &str = "YO_XTASK_PREFLIGHT_CHILD_OUTPUT";
const CHILD_STATUS: &str = "YO_XTASK_PREFLIGHT_CHILD_STATUS";
const CHILD_MUTATION: &str = "YO_XTASK_PREFLIGHT_CHILD_MUTATION";
const CHILD_FILTER: &str = "preflight_production_boundary_is_non_publishing_and_fails_closed";

// preflight 계측을 켜도 publication이 사용하는 canonical packet bytes는 하나도 바뀌지 않고,
// 각 section의 content와 wrapper 비용이 실제 렌더링 결과에서 별도로 관찰된다.
#[test]
fn preflight_measurement_preserves_canonical_packet_and_exposes_section_costs() {
    let inputs = sample_inputs("/tmp/validation.json");
    let plan = build_plan(&inputs);
    let review_id = domain_digest(
        REVIEW_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("plan serializes"),
    );

    let canonical = render_packet(&review_id, &plan, &inputs).unwrap();
    let measured = render_packet_with_measurements(&review_id, &plan, &inputs).unwrap();

    assert_eq!(measured.bytes, canonical);
    assert_eq!(measured.sections.len(), 9);
    assert_eq!(measured.sections[0].kind, "review_plan");
    assert_eq!(measured.sections[3].kind, "context");
    assert_eq!(measured.sections[8].kind, "git_diff");
    for section in measured.sections {
        assert!(section.content_bytes > 0);
        assert!(section.content_tokens_independent > 0);
        assert!(section.rendered_bytes > section.content_bytes);
        assert!(section.rendered_tokens_independent > section.content_tokens_independent);
    }
}

// v1alpha1 preflight은 complete packet budget과 별도로 exact input-prefix 진단을
// 보고하고 plan/diff를 prefix 밖에 두어 cache hit으로 과장하지 않는다.
#[test]
fn v1_alpha1_preflight_exposes_non_additive_input_prefix() {
    let inputs = sample_inputs_v1_alpha1("/tmp/validation.json");
    let plan = build_plan(&inputs);
    let review_id = domain_digest(REVIEW_ID_DOMAIN, &serde_json::to_vec(&plan).unwrap());
    let measured = render_packet_with_measurements(&review_id, &plan, &inputs).unwrap();
    let prefix = measured.input_prefix.expect("v1alpha1 prefix exists");

    assert_eq!(measured.sections[0].kind, "context_request");
    assert_eq!(measured.sections[3].kind, "repository_authority");
    assert_eq!(measured.sections[4].kind, "review_plan");
    assert_eq!(
        prefix.hash,
        crate::review_protocol::digest(&measured.bytes[..prefix.bytes])
    );
    assert_eq!(
        prefix.standalone_tokens,
        super::super::render::count_tokens(&measured.bytes[..prefix.bytes]).unwrap()
    );
}

// preflight 성공 결과는 prospective identity와 budget만 보고하고 eligible packet/manifest
// 경로나 hash를 제공하지 않아 publication이나 완료된 review evidence로 오인할 수 없다.
#[test]
fn preflight_result_is_explicitly_non_publishing_and_has_no_artifact_paths() {
    let value = serde_json::to_value(PreflightResultRecord {
        schema: PREFLIGHT_RESULT_SCHEMA_V1,
        ok: true,
        operation: "preflight_slice_review_packet",
        status: "ready",
        artifacts_published: false,
        authority: None,
        review_id: "sha256:review".to_owned(),
        trusted_commit: "0".repeat(40),
        candidate_commit: "1".repeat(40),
        packet: PreflightPacket {
            bytes: 100,
            managed_payload_tokens: 25,
            max_managed_payload_tokens: 30,
        },
        section_token_accounting: SECTION_TOKEN_ACCOUNTING,
        sections: Vec::new(),
        input_prefix: None,
    })
    .unwrap();

    assert_eq!(value["artifacts_published"], false);
    assert_eq!(value["status"], "ready");
    assert_eq!(
        value["section_token_accounting"],
        "independently-tokenized-non-additive/v1"
    );
    assert!(value["packet"].get("path").is_none());
    assert!(value.get("manifest").is_none());
}

// 실제 preflight 진입점은 성공해도 eligible review 디렉터리를 만들지 않으며, capture 뒤
// validation 입력을 바꾼 실행은 final guard에서 결과 바이트 없이 실패한다.
#[test]
fn preflight_production_boundary_is_non_publishing_and_fails_closed() {
    if run_preflight_child() {
        return;
    }
    let fixture = PreflightFixture::new();

    let ready = fixture.invoke_child("ready", None);
    assert_eq!(ready.status, "ok\n");
    let result: serde_json::Value = serde_json::from_slice(&ready.output).unwrap();
    assert_eq!(result["artifacts_published"], false);
    assert_eq!(result["section_token_accounting"], SECTION_TOKEN_ACCOUNTING);
    assert!(result["packet"].get("path").is_none());
    assert!(!fixture.review_root.exists());

    let changed = fixture.invoke_child("changed", Some(&fixture.validation_path));
    assert!(
        changed
            .status
            .contains("validation evidence changed during review packet construction")
    );
    assert!(changed.output.is_empty());
    assert!(!fixture.review_root.exists());
}

fn run_preflight_child() -> bool {
    let Some(request_path) = std::env::var_os(CHILD_REQUEST) else {
        return false;
    };
    let output_path = std::env::var_os(CHILD_OUTPUT).expect("child output path is supplied");
    let status_path = std::env::var_os(CHILD_STATUS).expect("child status path is supplied");
    if let Some(path) = std::env::var_os(CHILD_MUTATION) {
        set_preflight_test_hook(move || {
            std::fs::write(Path::new(&path), b"changed after capture\n")
                .map_err(|error| format!("cannot mutate validation fixture: {error}"))
        });
    }

    let mut output = Vec::new();
    let result = preflight(Path::new("."), Path::new(&request_path), &mut output);
    std::fs::write(output_path, output).unwrap();
    let status = match result {
        Ok(()) => "ok\n".to_owned(),
        Err(error) => format!("error: {error}\n"),
    };
    std::fs::write(status_path, status).unwrap();
    true
}

struct PreflightFixture {
    repository: TestRepository,
    request_path: std::path::PathBuf,
    validation_path: std::path::PathBuf,
    review_root: std::path::PathBuf,
}

impl PreflightFixture {
    fn new() -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let path = unique_path("review-preflight-production");
        let clone = crate::git::command_in(source.parent().unwrap(), false)
            .args(["clone", "--quiet", "--shared", "--branch", "develop"])
            .arg(&source)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "git clone: {}",
            String::from_utf8_lossy(&clone.stderr)
        );
        let repository = TestRepository { path };
        repository.git(["config", "user.name", "xtask Test"]);
        repository.git(["config", "user.email", "xtask@example.invalid"]);
        let hooks = repository.path.join(".git/disabled-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        repository.git(["config", "core.hooksPath", hooks.to_str().unwrap()]);
        let base = crate::git::output_in(
            &repository.path,
            &["rev-parse", "refs/heads/develop"],
            false,
        )
        .unwrap()
        .trim()
        .to_owned();
        repository.git(["switch", "--quiet", "-c", "slice/direct/preflight-fixture"]);
        repository.write("preflight-fixture.txt", "candidate\n");
        repository.git(["add", "preflight-fixture.txt"]);
        repository.git(["commit", "--quiet", "-m", "preflight fixture candidate"]);

        let contract_path = repository.write(
            ".git/preflight-contract.json",
            &json_text(&json!({
                "schema": "yo.slice-contract/v1",
                "slice": "preflight-fixture",
                "base": base,
                "base_ref": "refs/heads/develop",
                "owned_contracts": ["repository.review-packet.preflight"],
                "dependencies": [],
                "allowed_write_set": ["preflight-fixture.txt"],
                "focused_checks": ["cargo test --locked -p xtask review_packet"],
                "slice_close_checks": ["cargo test --locked -p xtask"]
            })),
        );
        slice_contract::bind(&repository.path, &contract_path).unwrap();

        let context_request_path = repository.write(
            ".local-exclude/preflight-fixture/context-request.json",
            &json_text(&json!({
                "schema": "methexis.context-request/v1alpha1",
                "anchors": [{
                    "kind": "knowledge_id",
                    "value": "methexis.review.bounded-packet"
                }],
                "tokenizer_profile": "o200k_base/v1",
                "max_tokens": 16000
            })),
        );
        let validation_path = repository.write(
            ".local-exclude/preflight-fixture/validation.md",
            "validation passed\n",
        );
        let request_path = repository.write(
            ".local-exclude/preflight-fixture/review-request.json",
            &json_text(&json!({
                "schema": "yo.slice-review-packet-request/v1",
                "context_request_path": relative(&repository.path, &context_request_path),
                "required_knowledge_ids": ["methexis.review.bounded-packet"],
                "slice_contract_path": contract_path,
                "repository_authority_paths": ["CONTRIBUTING.md"],
                "validation_evidence": [{
                    "name": "fixture-validation",
                    "path": relative(&repository.path, &validation_path)
                }],
                "review_lenses": ["fresh-context", "code-quality"],
                "review_questions": ["Is the preflight boundary correct?"],
                "delivery_profile": "yo.slice-review-markdown/v1alpha2",
                "tokenizer_profile": "o200k_base/v1",
                "max_managed_payload_tokens": 90000
            })),
        );
        assert!(
            crate::git::output_in(&repository.path, &["status", "--porcelain"], false)
                .unwrap()
                .is_empty()
        );
        let review_root = repository
            .path
            .join(".local-exclude/methexis/slice-reviews");

        Self {
            repository,
            request_path,
            validation_path,
            review_root,
        }
    }

    fn invoke_child(&self, label: &str, mutation: Option<&Path>) -> ChildResult {
        let output_path = self
            .repository
            .path
            .join(format!(".git/{label}-output.json"));
        let status_path = self
            .repository
            .path
            .join(format!(".git/{label}-status.txt"));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg(CHILD_FILTER)
            .arg("--nocapture")
            .current_dir(&self.repository.path)
            .env(CHILD_REQUEST, &self.request_path)
            .env(CHILD_OUTPUT, &output_path)
            .env(CHILD_STATUS, &status_path);
        if let Some(path) = mutation {
            command.env(CHILD_MUTATION, path);
        }
        let child = command.output().unwrap();
        assert!(
            child.status.success(),
            "child test: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        ChildResult {
            output: std::fs::read(output_path).unwrap(),
            status: std::fs::read_to_string(status_path).unwrap(),
        }
    }
}

struct ChildResult {
    output: Vec<u8>,
    status: String,
}

fn json_text(value: &serde_json::Value) -> String {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    String::from_utf8(bytes).unwrap()
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}
