use sha2::{Digest, Sha256};

use super::*;

fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

// summary usage는 exact source attribution과 네 token 값·관계를 모두 보존해야 하므로
// 누락·모순·상한 초과·input을 넘는 cache-read 표기를 생성 단계에서 거부합니다.
#[test]
fn rejects_incomplete_or_inconsistent_summary_usage_receipts() {
    let mut missing = summary_usage().value().clone();
    missing["usage"]
        .as_object_mut()
        .unwrap()
        .remove("input_tokens");
    assert!(ContextSummaryUsage::try_new(missing).is_err());

    let mut inconsistent = summary_usage().value().clone();
    inconsistent["usage"]["total_tokens"] = serde_json::json!(119);
    assert!(ContextSummaryUsage::try_new(inconsistent).is_err());

    let mut oversized_source = summary_usage().value().clone();
    oversized_source["response_id"] = serde_json::json!("r".repeat(257));
    assert!(ContextSummaryUsage::try_new(oversized_source).is_err());

    let mut oversized_cache = summary_usage().value().clone();
    oversized_cache["cache_read_input_tokens"] = serde_json::json!({
        "availability": "reported",
        "tokens": 101,
        "source_profile": "test-cache/v1"
    });
    assert!(ContextSummaryUsage::try_new(oversized_cache).is_err());
}
// artifact disclosure는 summarized replay의 실제 visible bytes와 hash·byte count가
// 일치해야 하므로 같은 delta 좌표에 임의 hash를 붙인 receipt를 거부합니다.
#[test]
fn rejects_an_artifact_receipt_not_bound_to_visible_source_bytes() {
    let receipt = ContextArtifactReceipt::try_new(
        content_hash(b"different"),
        9,
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("artifact receipt"));
}

// artifact receipt는 summarized replay group의 exact tool output bytes와 canonical text
// media kind에만 결속되며 ordinary assistant text는 같은 hash여도 source가 아닙니다.
#[test]
fn accepts_only_exact_summarized_tool_output_artifacts() {
    let output = "large tool output";
    let mut commits = current_history_with(
        ReplayProfile::SemanticOnly,
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "inspect".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: output.to_owned(),
            },
        ],
    );
    let receipt = ContextArtifactReceipt::try_new(
        content_hash(output.as_bytes()),
        u64::try_from(output.len()).unwrap(),
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));
    recover(&commits).unwrap();

    let assistant_receipt = ContextArtifactReceipt::try_new(
        content_hash(b"done"),
        4,
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let mut assistant_history = current_history();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    assistant_history.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![assistant_receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));
    let error = recover(&assistant_history).unwrap_err();
    assert!(error.to_string().contains("artifact receipt"));
}

// summarized provider-private item은 source delta의 exact schema·payload byte count로 loss를
// 하나씩 공개해야 하므로 visible loss만 기록한 checkpoint를 거부합니다.
#[test]
fn rejects_missing_provider_private_loss_disclosure() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let mut commits = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "done".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant { envelope: private },
        ],
    );
    let visible_loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![visible_loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("loss disclosure"));
}

// exact private loss가 visible summarized group과 일치하면 synthetic body만 남은 root도
// private-profile source checkpoint로 안전하게 복구됩니다.
#[test]
fn accepts_exact_provider_private_loss_disclosure() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let mut commits = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "done".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant { envelope: private },
        ],
    );
    let visible_loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    let private_loss = ContextLoss::provider_private_dropped(
        "kimi.assistant-message/v1alpha1",
        2,
        JournalSequence::new(8),
    )
    .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![visible_loss, private_loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.model_replay().items().len(), 1);
}
