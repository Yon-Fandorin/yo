use super::model::{Artifacts, ReviewLineage, SliceState};

pub(super) fn next_action(
    state: &SliceState,
    reviews: &ReviewLineage,
    artifacts: &Artifacts,
) -> &'static str {
    if !state.clean {
        "clean_candidate"
    } else if reviews.status == "broken" {
        "restore_review_lineage"
    } else if reviews.current_review_ids.is_empty() {
        // 직접 검토도 gate의 입력이므로 packet 부재만으로 검토를 반복하지 않습니다.
        if artifacts.gate_requests > 0 {
            "run_gate"
        } else if reviews
            .latest_candidate
            .as_deref()
            .is_some_and(|candidate| candidate != state.head)
            && artifacts.prior_findings > 0
        {
            "review_delta"
        } else {
            "build_review"
        }
    } else if artifacts.review_rounds == 0 {
        artifacts.delivery.next_action
    } else if artifacts.gate_requests == 0 {
        "prepare_gate"
    } else {
        "run_gate"
    }
}

pub(super) fn next_invocation(
    slice: &str,
    action: &str,
    artifacts: &Artifacts,
) -> (Option<Vec<String>>, Option<String>) {
    match action {
        "run_gate" => artifacts
            .gate_request
            .as_ref()
            .map(|path| {
                (
                    Some(vec![
                        "cargo".to_owned(),
                        "xtask".to_owned(),
                        "slice".to_owned(),
                        "gate".to_owned(),
                        path.display().to_string(),
                    ]),
                    None,
                )
            })
            .unwrap_or_else(|| {
                (
                    None,
                    Some(format!(
                        "expected one current gate request, found {}; select an explicit request to evaluate",
                        artifacts.gate_requests
                    )),
                )
            }),
        "deliver_current_review" => artifacts
            .delivery_request
            .as_ref()
            .map(|path| {
                (
                    Some(vec![
                        "cargo".to_owned(),
                        "xtask".to_owned(),
                        "slice".to_owned(),
                        "review-deliver".to_owned(),
                        path.display().to_string(),
                    ]),
                    None,
                )
            })
            .unwrap_or_else(|| {
                (
                    None,
                    Some("no current immutable delivery request is published".to_owned()),
                )
            }),
        "await_current_delivery" => (
            Some(vec![
                "cargo".to_owned(),
                "xtask".to_owned(),
                "slice".to_owned(),
                "status".to_owned(),
                slice.to_owned(),
            ]),
            artifacts.delivery.blocking_reason.clone(),
        ),
        "interpret_review" | "reconcile_failed_delivery" | "reconcile_unknown_delivery" => {
            (None, artifacts.delivery.blocking_reason.clone())
        },
        _ => (
            None,
            Some(format!(
                "`{action}` requires an explicit content-addressed request before an exact argv exists"
            )),
        ),
    }
}
