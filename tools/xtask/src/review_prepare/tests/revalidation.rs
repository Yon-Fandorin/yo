use std::fs;

use super::{
    super::{
        files::{PreparedBytes, PreparedPaths, require_empty_directory},
        revalidation::require_prepared_requests_current,
    },
    TemporaryDirectory,
};

// delivery output에 claim이나 결과가 하나라도 있으면 준비 재실행이 이를 비우거나
// 덮어쓰지 않고 중단되어 exact-once 요청 경계를 보존합니다.
#[test]
fn nonempty_delivery_output_is_never_reprepared() {
    let directory = TemporaryDirectory::new("review-prepare-output");
    require_empty_directory(&directory.0).unwrap();
    fs::write(directory.0.join("claim.json"), b"claim\n").unwrap();
    assert!(
        require_empty_directory(&directory.0)
            .unwrap_err()
            .contains("not empty")
    );
}

// packet publication 뒤 생성 요청 하나가 바뀌어도 마지막 통합 경계가 성공 결과를
// 반환하지 않도록, 모든 준비 산출물의 정확한 바이트를 한 번에 다시 확인합니다.
#[test]
fn final_prepared_request_check_rejects_post_publication_drift() {
    let directory = TemporaryDirectory::new("review-prepare-final-drift");
    let context = directory.0.join("context.json");
    let review = directory.0.join("review.json");
    let egress = directory.0.join("egress.json");
    let admission = directory.0.join("admission.json");
    let delivery = directory.0.join("delivery.json");
    for (path, bytes) in [
        (&context, b"context\n".as_slice()),
        (&review, b"review\n".as_slice()),
        (&egress, b"egress\n".as_slice()),
        (&admission, b"admission\n".as_slice()),
        (&delivery, b"delivery\n".as_slice()),
    ] {
        fs::write(path, bytes).unwrap();
    }
    let paths = PreparedPaths {
        context: &context,
        review: &review,
        egress: &egress,
        admission: &admission,
        delivery: &delivery,
        delivery_output: &directory.0,
    };
    let bytes = PreparedBytes {
        context: b"context\n",
        review: b"review\n",
        egress: b"egress\n",
        admission: b"admission\n",
        delivery: b"delivery\n",
    };
    require_prepared_requests_current(&paths, &bytes).unwrap();

    fs::write(&review, b"changed\n").unwrap();
    assert!(
        require_prepared_requests_current(&paths, &bytes)
            .unwrap_err()
            .contains("Slice review packet request changed")
    );
}
