use super::{ACTIVATE_REQUEST_SCHEMA, ActivationInput, semantic_hash};

const CHECKPOINT_ID: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const CHECKPOINT_HASH: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const FIRST_PREDECESSOR: &str =
    "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const SECOND_PREDECESSOR: &str =
    "sha256:4444444444444444444444444444444444444444444444444444444444444444";

fn activation_hash(predecessor: &str) -> String {
    semantic_hash(&ActivationInput {
        schema: ACTIVATE_REQUEST_SCHEMA,
        checkpoint_id: CHECKPOINT_ID,
        checkpoint_hash: CHECKPOINT_HASH,
        replace_active_hash: Some(predecessor),
    })
}

// Checkpoint가 같아도 CAS 전임자가 다르면 서로 다른 요청이므로 request identity가
// 반드시 달라져, 재생된 replacement 요청을 같은 승인 입력으로 오인하지 않는다.
#[test]
fn activation_request_identity_includes_the_compare_and_swap_predecessor() {
    assert_ne!(
        activation_hash(FIRST_PREDECESSOR),
        activation_hash(SECOND_PREDECESSOR)
    );
}
