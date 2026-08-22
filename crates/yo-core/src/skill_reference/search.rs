use super::SkillReferenceCandidate;
use crate::normalized_search_key;

const RESULT_CAP: usize = 40;

pub fn search_skill_reference_candidates(
    inventory: &[SkillReferenceCandidate],
    query: &str,
) -> Vec<SkillReferenceCandidate> {
    let query = normalized_search_key(query);
    let mut matches = inventory
        .iter()
        .filter_map(|candidate| {
            let name = normalized_search_key(candidate.reference().name());
            let display = normalized_search_key(candidate.display_name());
            let description = normalized_search_key(candidate.description());
            let rank = if query.is_empty() {
                0
            } else if name == query || display == query {
                1
            } else if name.starts_with(&query) || display.starts_with(&query) {
                2
            } else if name.contains(&query) || display.contains(&query) {
                3
            } else if description.contains(&query) {
                4
            } else {
                return None;
            };
            Some((rank, display, candidate.reference().identity(), candidate))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| (left.0, &left.1, left.2).cmp(&(right.0, &right.1, right.2)));
    matches
        .into_iter()
        .take(RESULT_CAP)
        .map(|(_, _, _, candidate)| candidate.clone())
        .collect()
}
