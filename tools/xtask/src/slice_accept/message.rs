use std::{path::Path, str};

use crate::{bounded_file, review_protocol, slice_gate};

pub(super) const MESSAGE_LIMIT: usize = 64 * 1024;

pub(crate) fn prepare_commit_message(
    repository: &Path,
    gate_request: &Path,
    message_source: &Path,
    output: &Path,
) -> Result<(), String> {
    let gate = slice_gate::ready(repository, gate_request)?;
    let source = bounded_file::read_regular(
        message_source,
        MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    let message = compose_message(&source, &gate.commit_trailers)?;

    let current_gate = slice_gate::ready(repository, gate_request)?;
    let current_source = bounded_file::read_regular(
        message_source,
        MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    if current_gate != gate || current_source != source {
        return Err("post-gate commit inputs changed before publication".to_owned());
    }

    let created = bounded_file::publish_new_or_exact(
        output,
        &message,
        MESSAGE_LIMIT,
        "prepared accepted commit message",
    )?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": "yo.slice-commit-message-publication/v1alpha1",
            "ok": true,
            "status": if created { "written" } else { "reused" },
            "slice": gate.slice,
            "candidate_commit": gate.candidate_commit,
            "diff_hash": gate.diff_hash,
            "message_path": output,
            "message_hash": review_protocol::digest(&message)
        }))
        .map_err(|error| format!("cannot encode commit message publication: {error}"))?
    );
    Ok(())
}

pub(super) fn compose_message(source: &[u8], trailers: &[String]) -> Result<Vec<u8>, String> {
    let source = str::from_utf8(source)
        .map_err(|error| format!("accepted commit message source must be UTF-8: {error}"))?;
    if source.contains('\0') || source.contains('\r') {
        return Err("accepted commit message source must use LF text without NUL bytes".to_owned());
    }
    let source = source.trim_end_matches('\n');
    if source.trim().is_empty() {
        return Err("accepted commit message source must not be blank".to_owned());
    }
    if source
        .lines()
        .any(|line| line.starts_with("Slice-Review:") || line.starts_with("Review-Coverage:"))
    {
        return Err(
            "accepted commit message source must omit gate-derived review trailers".to_owned(),
        );
    }
    if trailers.is_empty() {
        return Err("ready gate returned no commit trailers".to_owned());
    }
    let mut message = String::with_capacity(
        source.len() + trailers.iter().map(String::len).sum::<usize>() + trailers.len() + 3,
    );
    message.push_str(source);
    message.push_str("\n\n");
    message.push_str(&trailers.join("\n"));
    message.push('\n');
    if message.len() > MESSAGE_LIMIT {
        return Err(format!(
            "prepared accepted commit message exceeds the {MESSAGE_LIMIT}-byte limit"
        ));
    }
    Ok(message.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::compose_message;

    // 게이트가 소유하는 review trailer만 기계적으로 덧붙이고 사람이 작성한 의미 설명과
    // Developer Docs 판단은 그대로 보존한다.
    #[test]
    fn commit_message_appends_exact_gate_trailers() {
        let message = compose_message(
            b"feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n",
            &[
                "Slice-Review: fresh-context - completed - codex/test - clear".to_owned(),
                "Review-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc"
                    .to_owned(),
            ],
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(message).unwrap(),
            "feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n\nSlice-Review: fresh-context - completed - codex/test - clear\nReview-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc\n"
        );
    }

    // 사람이 준비한 원문에 이전 후보의 review trailer가 섞이면 ready 게이트의 exact
    // trailer와 공존시키지 않고 입력 경계에서 바로 거부한다.
    #[test]
    fn commit_message_rejects_caller_review_trailers() {
        for trailer in ["Slice-Review:", "Review-Coverage:"] {
            let source = format!("feat: stale\n\n{trailer} old\n");
            let error = compose_message(
                source.as_bytes(),
                &["Slice-Review: none - exact".to_owned()],
            )
            .unwrap_err();
            assert!(error.contains("must omit gate-derived review trailers"));
        }
    }
}
