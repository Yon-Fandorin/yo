use super::super::{
    capture::{capture_context_from_result, captured},
    model::{CheckpointIdentity, ContextResult},
};
use crate::{
    review_protocol::{artifact, digest},
    test_support::TestRepository,
};

struct ContextFixture {
    repository: TestRepository,
    request: crate::review_protocol::Captured,
    result: ContextResult,
    context_bytes: Vec<u8>,
    manifest_bytes: Vec<u8>,
}

fn context_fixture(label: &str) -> ContextFixture {
    let repository = TestRepository::new(label);
    let context_bytes = b"bounded context\n".to_vec();
    let context = captured(
        ".local-exclude/build/context.md".to_owned(),
        context_bytes.clone(),
    )
    .unwrap();
    repository.write(&context.path, std::str::from_utf8(&context.bytes).unwrap());
    let build_id = format!("sha256:{}", "b".repeat(64));
    let checkpoint = CheckpointIdentity {
        id: format!("sha256:{}", "c".repeat(64)),
        hash: format!("sha256:{}", "d".repeat(64)),
        authority_basis_commit: "1".repeat(40),
    };
    let mut manifest_bytes = serde_json::to_vec(&serde_json::json!({
        "schema": "methexis.context-manifest/v1alpha1",
        "build_id": build_id,
        "plan": {
            "checkpoint": checkpoint,
            "units": [{"id": "methexis.review.bounded-packet"}],
            "tokenizer_profile": "o200k_base/v1"
        },
        "context": {
            "path": "context.md",
            "hash": context.hash
        }
    }))
    .unwrap();
    manifest_bytes.push(b'\n');
    let manifest = captured(
        ".local-exclude/build/manifest.json".to_owned(),
        manifest_bytes.clone(),
    )
    .unwrap();
    repository.write(
        &manifest.path,
        std::str::from_utf8(&manifest.bytes).unwrap(),
    );
    let request = captured(".local-exclude/request.json".to_owned(), b"{}\n".to_vec()).unwrap();
    let result = ContextResult {
        schema: "methexis.context-result/v1alpha1".to_owned(),
        ok: true,
        operation: "resolve_context".to_owned(),
        authority: "trusted_integration".to_owned(),
        trusted_commit: "1".repeat(40),
        build_id,
        context: artifact(&context),
        manifest: artifact(&manifest),
        checkpoint: None,
        activation_request: None,
        predecessor_active_record_hash: None,
        proposed_active_record_hash: None,
    };
    ContextFixture {
        repository,
        request,
        result,
        context_bytes,
        manifest_bytes,
    }
}

// 정상 Methexis physical result path가 safe relative sibling이고 manifest가 context.md를
// 소유하면 기존 capture 결과를 그대로 허용합니다.
#[test]
fn context_capture_accepts_manifest_bound_relative_siblings() {
    let fixture = context_fixture("review-context-safe-path");

    let captured =
        capture_context_from_result(&fixture.repository.path, fixture.request, fixture.result)
            .unwrap();

    assert_eq!(captured.context.bytes, fixture.context_bytes);
    assert_eq!(captured.manifest.bytes, fixture.manifest_bytes);
}

// hash가 일치하는 실제 파일이 있어도 absolute 또는 parent-traversing result spelling은
// repository join 전에 context와 manifest 각각에서 닫힙니다.
#[test]
fn context_capture_rejects_absolute_and_traversing_result_paths() {
    for (label, target, expected) in [
        ("absolute-context", "context", "context path"),
        ("traversing-context", "context", "context path"),
        ("absolute-manifest", "manifest", "manifest path"),
        ("traversing-manifest", "manifest", "manifest path"),
    ] {
        let mut fixture = context_fixture(&format!("review-context-{label}"));
        let unsafe_path = if label.starts_with("absolute") {
            fixture.repository.path.join(if target == "context" {
                ".local-exclude/build/context.md"
            } else {
                ".local-exclude/build/manifest.json"
            })
        } else {
            if target == "context" {
                fixture.repository.write(
                    ".local-exclude/context.md",
                    std::str::from_utf8(&fixture.context_bytes).unwrap(),
                );
                ".local-exclude/build/../context.md".into()
            } else {
                fixture.repository.write(
                    ".local-exclude/manifest.json",
                    std::str::from_utf8(&fixture.manifest_bytes).unwrap(),
                );
                ".local-exclude/build/../manifest.json".into()
            }
        };
        if target == "context" {
            fixture.result.context.path = unsafe_path.to_string_lossy().into_owned();
        } else {
            fixture.result.manifest.path = unsafe_path.to_string_lossy().into_owned();
        }

        let error =
            capture_context_from_result(&fixture.repository.path, fixture.request, fixture.result)
                .err()
                .expect("unsafe result path must fail capture");

        assert!(error.contains(expected), "{label}: {error}");
        assert!(error.contains("safe relative repository path"));
    }
}

#[cfg(unix)]
// safe relative spelling이라도 context 또는 manifest leaf가 symlink이면 bounded regular
// file capture가 대상을 따라가지 않고 두 artifact 모두 거부합니다.
#[test]
fn context_capture_rejects_symlinked_result_artifacts() {
    use std::os::unix::fs::symlink;

    for target in ["context", "manifest"] {
        let mut fixture = context_fixture(&format!("review-context-{target}-symlink"));
        let link = format!(".local-exclude/build/{target}-link");
        let original = if target == "context" {
            "context.md"
        } else {
            "manifest.json"
        };
        symlink(original, fixture.repository.path.join(&link)).unwrap();
        if target == "context" {
            fixture.result.context.path = link;
        } else {
            fixture.result.manifest.path = link;
        }

        let error =
            capture_context_from_result(&fixture.repository.path, fixture.request, fixture.result)
                .err()
                .expect("symlinked result artifact must fail capture");

        assert!(
            error.to_lowercase().contains("symlink"),
            "{target}: {error}"
        );
    }
}

// 두 result artifact가 모두 safe regular file이고 hash도 맞아도 context가 manifest의
// physical sibling context.md가 아니면 logical ownership 결속에서 실패합니다.
#[test]
fn context_capture_rejects_a_context_outside_the_manifest_owned_sibling() {
    let mut fixture = context_fixture("review-context-logical-mismatch");
    fixture.repository.write(
        ".local-exclude/build/other.md",
        std::str::from_utf8(&fixture.context_bytes).unwrap(),
    );
    fixture.result.context.path = ".local-exclude/build/other.md".to_owned();
    fixture.result.context.hash = digest(&fixture.context_bytes);

    let error =
        capture_context_from_result(&fixture.repository.path, fixture.request, fixture.result)
            .err()
            .expect("logical path mismatch must fail capture");

    assert_eq!(error, "ContextBuild result and manifest identities differ");
}
