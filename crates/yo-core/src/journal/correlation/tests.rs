use super::{
    super::{JournalEntry, SemanticRecord},
    context::{context_source_groups, transfer_context_groups},
};
use crate::{
    JournalSequence, ModelReplayItem, ModelReplayRole,
    journal::codec::{
        ContextCheckpoint, ContextLoss, ContextRetainedGroup, ContextStrategy, ContextSummaryUsage,
    },
};

#[cfg(test)]
// 두 번째 압축의 source는 새 요약 본문, 원본 import, child local tail 순서를 유지한다.
// 동일 checkpoint 좌표의 앞·뒤 local group을 합쳐 import 앞으로 옮기면 이 검증이 실패한다.
#[test]
fn imported_checkpoint_sources_preserve_root_order_and_private_origin_epochs() {
    use crate::{
        ModelReplayContract, ProviderPrivateReplayEnvelope, ReplayProfile, provider_private_schema,
    };

    let private = ModelReplayItem::ProviderPrivateAssistant {
        envelope: ProviderPrivateReplayEnvelope::new(
            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
            br#"{"reasoning_content":"original private bytes","content":"inherited"}"#.to_vec(),
        )
        .unwrap(),
    };
    let inherited = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "inherited".into(),
            refusal: None,
        },
        private,
    ];
    let tail = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "child tail".into(),
        refusal: None,
    };
    let body = "# Context Checkpoint\n## Current Objective\nContinue.\n## Active Constraints\nNone.\n## Decisions\nKeep imports.\n## Verified Progress\nDone.\n## Current State\nIdle.\n## Unknown or Unverified\nNone.\n## Next Actions\nContinue.\n## Critical References\nNone.";
    let usage = ContextSummaryUsage::try_new(serde_json::json!({
        "schema":"yo.model-usage-receipt/v1", "response_id":"summary", "round":1,
        "provider":"test", "account":"default", "model":"test", "connector":"openai-responses",
        "api_dialect":"openai-responses", "base_url":"https://example.invalid/",
        "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"reasoning_tokens":0},
        "cache_read_input_tokens":{"availability":"unsupported"}
    }))
    .unwrap();
    let checkpoint = ContextCheckpoint::try_new(
        1,
        1,
        2,
        JournalSequence::new(19),
        JournalSequence::new(18),
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        1000,
        900,
        100,
        ModelReplayContract::new("system", vec![]),
        body,
        vec![
            ContextRetainedGroup::try_imported(
                JournalSequence::new(2),
                1,
                inherited.clone(),
                vec![3],
            )
            .unwrap(),
            ContextRetainedGroup::try_new(
                JournalSequence::new(15),
                JournalSequence::new(18),
                vec![tail.clone()],
            )
            .unwrap(),
        ],
        Some(JournalSequence::new(2)),
        vec![],
        vec![
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(2),
                JournalSequence::new(2),
            )
            .unwrap(),
        ],
        usage,
    )
    .unwrap();
    let expected = checkpoint.replay_root().unwrap();
    let entries = vec![JournalEntry::new(
        JournalSequence::new(20),
        SemanticRecord::ContextCheckpoint(checkpoint),
    )];
    let groups = context_source_groups(&entries, 1, 2).unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].items, expected.items()[..1]);
    assert_eq!(groups[0].first_sequence, JournalSequence::new(20));
    assert_eq!(groups[1].items, inherited);
    assert_eq!(groups[1].fork_import, Some((JournalSequence::new(2), 1)));
    assert_eq!(groups[1].private_epochs, vec![3]);
    assert_eq!(groups[2].items, vec![tail]);
    assert_eq!(groups[2].first_sequence, JournalSequence::new(20));
    assert!(
        groups
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
    assert_eq!(
        groups[..2].iter().map(|group| group.first_sequence).min(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(
        groups[..2].iter().map(|group| group.last_sequence).max(),
        Some(JournalSequence::new(20))
    );
    // exact replacement를 두 번 거쳐도 import를 local run에 흡수하지 않는다.
    let transferred = transfer_context_groups(groups, JournalSequence::new(30));
    assert_eq!(transferred.len(), 3);
    assert_eq!(transferred[0].first_sequence, JournalSequence::new(30));
    assert_eq!(
        transferred[1].fork_import,
        Some((JournalSequence::new(2), 1))
    );
    assert_eq!(transferred[1].private_epochs, vec![3]);
    assert_eq!(transferred[2].first_sequence, JournalSequence::new(30));
    assert!(
        transferred
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
    let twice = transfer_context_groups(transferred, JournalSequence::new(40));
    assert_eq!(twice[0].first_sequence, JournalSequence::new(40));
    assert_eq!(twice[1].fork_import, Some((JournalSequence::new(2), 1)));
    assert_eq!(twice[2].first_sequence, JournalSequence::new(40));
    assert!(
        twice
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
}
