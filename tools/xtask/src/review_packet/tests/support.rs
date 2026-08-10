use super::super::*;

pub(super) fn sample_inputs(validation_path: &str) -> Inputs {
    let context_request = captured(
        "context-request.json".to_owned(),
        b"context request".to_vec(),
    )
    .unwrap();
    let context = captured("context.md".to_owned(), b"context".to_vec()).unwrap();
    let manifest = captured(
        "context-manifest.json".to_owned(),
        b"context manifest".to_vec(),
    )
    .unwrap();
    Inputs {
        base_commit: "0000000000000000000000000000000000000000".to_owned(),
        candidate_commit: "1111111111111111111111111111111111111111".to_owned(),
        diff: captured("git-diff.patch".to_owned(), b"diff".to_vec()).unwrap(),
        context: ContextCapture {
            result: ContextResult {
                schema: "methexis.context-result/v1alpha1".to_owned(),
                ok: true,
                operation: "resolve_context".to_owned(),
                authority: "trusted_integration".to_owned(),
                trusted_commit: "2222222222222222222222222222222222222222".to_owned(),
                build_id: "sha256:build".to_owned(),
                context: artifact(&context),
                manifest: artifact(&manifest),
            },
            request: context_request,
            context,
            manifest,
            active_checkpoint: CheckpointIdentity {
                id: "sha256:checkpoint".to_owned(),
                hash: "sha256:checkpoint-hash".to_owned(),
                authority_basis_commit: "3333333333333333333333333333333333333333".to_owned(),
            },
            included_ids: vec!["methexis.review.bounded-packet".to_owned()],
        },
        authorities: vec![captured("CONTRIBUTING.md".to_owned(), b"authority".to_vec()).unwrap()],
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        validation: vec![NamedCaptured {
            name: "validation".to_owned(),
            artifact: captured(validation_path.to_owned(), b"passed".to_vec()).unwrap(),
        }],
        lenses: vec!["fresh-context".to_owned()],
        questions: vec!["Is it correct?".to_owned()],
        required_knowledge_ids: vec!["methexis.review.bounded-packet".to_owned()],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    }
}
