pub(crate) fn body_start_line(content: &str, body: &str) -> u64 {
    let body_offset = content.len() - body.len();
    content[..body_offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

pub(super) struct BodyLine<'a> {
    pub(super) heading: Option<&'a str>,
    pub(super) has_content: bool,
    pub(super) forbidden_html: bool,
}

pub(super) fn classify_body_lines(body: &str) -> Vec<BodyLine<'_>> {
    let mut fence = None;
    let mut html_comment = false;
    body.lines()
        .map(|line| {
            let marker = (!html_comment).then(|| fence_marker(line)).flatten();
            let outside_fence = fence.is_none() && marker.is_none();
            let contains_comment_marker = line.contains("<!--") || line.contains("-->");
            let forbidden_html = outside_fence
                && (html_comment || contains_comment_marker || line.trim_start().starts_with('<'));
            let heading = if outside_fence
                && !html_comment
                && !contains_comment_marker
                && line.starts_with("## ")
            {
                Some(line)
            } else {
                None
            };

            match (fence, marker) {
                (None, Some(opening)) => fence = Some(opening),
                (Some((character, minimum)), Some((candidate, length)))
                    if character == candidate
                        && length >= minimum
                        && fence_closing_line(line, character, length) =>
                {
                    fence = None;
                },
                _ => {},
            }

            if outside_fence {
                update_html_comment_state(line, &mut html_comment);
            }

            BodyLine {
                heading,
                has_content: !forbidden_html && !line.trim().is_empty(),
                forbidden_html,
            }
        })
        .collect()
}

pub(crate) fn body_has_forbidden_html(body: &str) -> bool {
    classify_body_lines(body)
        .iter()
        .any(|line| line.forbidden_html)
}

fn update_html_comment_state(mut line: &str, html_comment: &mut bool) {
    loop {
        if *html_comment {
            let Some(end) = line.find("-->") else {
                return;
            };
            *html_comment = false;
            line = &line[end + 3..];
        } else {
            let Some(start) = line.find("<!--") else {
                return;
            };
            *html_comment = true;
            line = &line[start + 4..];
        }
    }
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let candidate = line.as_bytes();
    let indentation = candidate.iter().take_while(|byte| **byte == b' ').count();
    if indentation > 3 {
        return None;
    }
    let marker = *candidate.get(indentation)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = candidate[indentation..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    (length >= 3).then_some((marker, length))
}

fn fence_closing_line(line: &str, marker: u8, length: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    trimmed
        .as_bytes()
        .get(length..)
        .is_some_and(|remainder| remainder.iter().all(u8::is_ascii_whitespace))
        && trimmed.as_bytes().first() == Some(&marker)
}
