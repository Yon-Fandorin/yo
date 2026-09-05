#[derive(Clone, Debug)]
pub(super) struct ExactEdit {
    old_text: String,
    new_text: String,
}

impl ExactEdit {
    pub(super) fn new(old_text: String, new_text: String) -> Self {
        Self { old_text, new_text }
    }

    pub(super) fn old_len(&self) -> usize {
        self.old_text.len()
    }

    pub(super) fn new_len(&self) -> usize {
        self.new_text.len()
    }
}

pub(super) fn plan_replacements(
    original: &[u8],
    edits: &[ExactEdit],
) -> Result<Vec<usize>, &'static str> {
    let mut replacements = Vec::with_capacity(edits.len());
    for edit in edits {
        let matches = overlapping_matches(original, edit.old_text.as_bytes());
        match matches.as_slice() {
            [] => return Err("match_absent"),
            [position] => replacements.push(*position),
            _ => return Err("match_ambiguous"),
        }
    }
    let mut spans = replacements
        .iter()
        .zip(edits)
        .map(|(start, edit)| (*start, start + edit.old_text.len()))
        .collect::<Vec<_>>();
    spans.sort_unstable();
    if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("overlapping_edits");
    }
    Ok(replacements)
}

fn overlapping_matches(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let Some(first) = memchr::memmem::find(haystack, needle) else {
        return Vec::new();
    };
    let second =
        memchr::memmem::find(&haystack[first + 1..], needle).map(|second| first + 1 + second);
    second.map_or_else(|| vec![first], |second| vec![first, second])
}

pub(super) fn apply_replacements(
    original: &[u8],
    edits: &[ExactEdit],
    starts: &[usize],
) -> Vec<u8> {
    let mut result = original.to_vec();
    let mut ordered = starts.iter().copied().zip(edits).collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|(start, _)| std::cmp::Reverse(*start));
    for (start, edit) in ordered {
        result.splice(
            start..start + edit.old_text.len(),
            edit.new_text.as_bytes().iter().copied(),
        );
    }
    result
}
