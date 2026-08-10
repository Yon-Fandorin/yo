use super::{
    JournalCommit, JournalRecord, JournalSequence, ReplaySequence, SequencedJournalRecord, decode,
    descriptor_with_path, encode, recover,
};

// descriptor-only 첫 commit은 semantic JournalSequence를 만들지 않으면서도 UUIDv7 Session,
// Host, 정규화 경로와 시작 시각을 v1 payload에서 손실 없이 왕복해야 한다.
#[test]
fn round_trips_the_initial_descriptor_without_a_semantic_cutoff() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let commit = JournalCommit::descriptor(descriptor.clone());

    let encoded = encode(&commit).expect("the descriptor commit encodes");
    let decoded = decode(&encoded).expect("the descriptor commit decodes");
    let recovered = recover(std::slice::from_ref(&decoded)).expect("the descriptor recovers");

    assert_eq!(decoded.journal_cutoff(), None);
    assert_eq!(recovered.descriptor(), Some(&descriptor));
    assert_eq!(recovered.journal_cutoff(), None);
}

// Unix workspace가 UTF-8이 아니어도 lossy 문자열로 바꾸지 않고 명시적인 unix_bytes
// 표현을 사용해야 다른 Host가 로컬 path 규칙을 적용하지 않고 원본 바이트를 보존한다.
#[test]
fn preserves_a_non_utf8_workspace_path_with_an_explicit_wire_encoding() {
    let descriptor = descriptor_with_path(vec![b'/', b'w', 0xff]);
    let encoded = encode(&JournalCommit::descriptor(descriptor.clone())).unwrap();

    assert!(encoded.contains("\"encoding\":\"unix_bytes\""));
    let decoded = decode(&encoded).unwrap();
    let JournalRecord::SessionDescriptor(decoded_descriptor) = decoded.records()[0].record() else {
        panic!("the first record remains a Session descriptor");
    };
    assert_eq!(decoded_descriptor, &descriptor);
}

// remote Host의 path를 reader filesystem에서 다시 resolve하면 안 되지만 `.`·`..`, 빈
// component, trailing separator처럼 canonicalize 결과가 만들지 않는 lexical alias는
// 같은 workspace를 여러 값으로 나누므로 손상된 descriptor로 거부해야 한다.
#[test]
fn rejects_lexically_noncanonical_workspace_paths_from_the_wire() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let encoded = encode(&JournalCommit::descriptor(descriptor)).unwrap();

    for path in [
        "/workspace/../other",
        "/workspace/.",
        "/workspace//child",
        "/workspace/",
    ] {
        let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        wire["records"][0]["descriptor"]["workspace_path"]["value"] =
            serde_json::Value::String(path.to_owned());

        let error = decode(&wire.to_string()).expect_err("lexical aliases are not canonical");

        assert!(error.to_string().contains("host-normalized"));
    }
}

// descriptor의 명시적 시작 시각이 UUIDv7 내부 millisecond와 다르면 두 개의 시작점을
// 만들게 되므로 semantic JSON decoder가 모순된 descriptor를 거부해야 한다.
#[test]
fn rejects_a_descriptor_start_time_that_disagrees_with_its_session_id() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let encoded = encode(&JournalCommit::descriptor(descriptor)).unwrap();
    let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
    wire["records"][0]["descriptor"]["start_time_unix_millis"] = serde_json::Value::from(1_u64);

    let error = decode(&wire.to_string()).expect_err("mismatched start times are corrupt");

    assert!(error.to_string().contains("does not match"));
}

// descriptor가 ReplaySequence 1이 아닌 곳에 나타나면 session prefix가 두 개로 갈라질 수
// 있으므로 codec이 physical append 전에 잘못 놓인 descriptor를 거부해야 한다.
#[test]
fn rejects_a_descriptor_outside_replay_sequence_one() {
    let commit = JournalCommit::incremental_through(
        JournalSequence::new(1),
        vec![SequencedJournalRecord::new(
            ReplaySequence::new(2),
            JournalRecord::SessionDescriptor(descriptor_with_path(b"/workspace".to_vec())),
        )],
    );

    let error = encode(&commit).expect_err("a misplaced descriptor is not durable");

    assert!(error.to_string().contains("replay-sequence-one"));
}
