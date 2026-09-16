use crate::validation_summary::argv_hash;

// 각 argv의 byte 길이와 NUL 경계를 framing하여 단순 연결의 인자 경계 충돌을 막는다.
#[test]
fn argv_hash_is_boundary_aware() {
    assert_ne!(
        argv_hash(&["ab".to_owned(), "c".to_owned()]),
        argv_hash(&["a".to_owned(), "bc".to_owned()])
    );
}

// Rust 검증기의 framing을 bounded shell runner의 canonical test vector와 맞춘다.
#[test]
fn argv_hash_matches_the_bounded_runner_framing() {
    let argv = [
        "bash".to_owned(),
        "-c".to_owned(),
        "printf \"visible only in the full log\\n\"; printf \"diagnostic\\n\" >&2".to_owned(),
    ];

    assert_eq!(
        argv_hash(&argv),
        "sha256:b2feeb2dc7a19ae550541f96076627745b156652ed171a1f7bc182cbdee19b74"
    );
}
