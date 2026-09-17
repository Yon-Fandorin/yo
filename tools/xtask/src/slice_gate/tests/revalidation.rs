use std::{fs, path::PathBuf};

use super::{super::set_final_revalidate_test_hook, Fixture};

// 최초 capture 뒤 반환 직전에 evidence가 바뀌면 final revalidation이 같은 hash를
// 다시 읽어, 이미 만든 in-memory green 결과를 ready로 반환하지 않는다.
#[test]
fn final_revalidation_rejects_evidence_changed_after_capture() {
    let fixture = Fixture::new();
    let path = PathBuf::from(
        fixture.request["review_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    set_final_revalidate_test_hook(move || {
        fs::write(path, b"changed before final revalidation\n").map_err(|error| error.to_string())
    });

    assert!(fixture.evaluate().unwrap_err().contains("hash changed"));
}

// 최초 identity 확인 뒤 request 자체가 바뀌면 마지막 regular-file capture가 이를
// 감지하여 다른 승인/증거 선언으로 바뀐 bytes에 이전 평가를 적용하지 않는다.
#[test]
fn final_revalidation_rejects_request_changed_after_capture() {
    let fixture = Fixture::new();
    let request_path = fixture.artifacts.join("request.json");
    set_final_revalidate_test_hook(move || {
        fs::write(request_path, b"{}\n").map_err(|error| error.to_string())
    });

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("request changed during evaluation")
    );
}
