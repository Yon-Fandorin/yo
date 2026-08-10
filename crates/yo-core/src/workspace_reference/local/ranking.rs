use super::super::{WorkspaceReferenceCandidate, normalized_search_key};

const RESULT_CAP: usize = 40;

pub(super) fn search(
    entries: &[WorkspaceReferenceCandidate],
    query: &str,
) -> Vec<WorkspaceReferenceCandidate> {
    let query = normalized_search_key(query.trim_end_matches('/'));
    let mut ranked = entries
        .iter()
        .filter_map(|candidate| {
            let path = candidate.reference().relative_path();
            let label = path.rsplit('/').next().unwrap_or(path);
            rank(path, label, &query).map(|score| {
                (
                    score,
                    normalized_search_key(candidate.reference().relative_path()),
                    candidate,
                )
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked
        .into_iter()
        .take(RESULT_CAP)
        .map(|(_, _, candidate)| candidate.clone())
        .collect()
}

pub(super) fn rank(path: &str, label: &str, query: &str) -> Option<(u8, usize, usize, usize)> {
    let depth = path.bytes().filter(|byte| *byte == b'/').count();
    if query.is_empty() {
        return Some((0, 0, depth, path.chars().count()));
    }
    let path = normalized_search_key(path);
    let label = normalized_search_key(label);
    let path_length = path.chars().count();
    if path == query || label == query {
        return Some((0, 0, depth, path_length));
    }
    if path.starts_with(query) || label.starts_with(query) {
        return Some((1, 0, depth, path_length));
    }
    if path.split('/').any(|segment| segment.starts_with(query)) {
        return Some((2, 0, depth, path_length));
    }
    if label.contains(query) {
        return Some((3, 0, depth, path_length));
    }
    if path.contains(query) {
        return Some((3, 0, depth, path_length));
    }
    subsequence_gaps(query, &path).map(|gaps| (4, gaps, depth, path_length))
}

fn subsequence_gaps(query: &str, candidate: &str) -> Option<usize> {
    let mut positions = candidate.chars().enumerate();
    let mut previous_index = None;
    let mut gaps = 0;
    for wanted in query.chars() {
        let (position, _) = positions.by_ref().find(|(_, found)| *found == wanted)?;
        if let Some(previous_index) = previous_index {
            gaps += position.saturating_sub(previous_index + 1);
        }
        previous_index = Some(position);
    }
    Some(gaps)
}
