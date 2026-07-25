//! Constrained Markdown validation mirrored from the Methexis Pilot contract.

pub(crate) fn validate_body(body: &str, required_sections: &[&str]) -> Result<(), String> {
    let lines = classify_body_lines(body);
    if lines.iter().any(|line| line.forbidden_html) {
        return Err("canonical Markdown bodies must not contain raw HTML or comments".to_owned());
    }
    for name in required_sections {
        require_section(&lines, name)?;
    }
    Ok(())
}

pub(crate) fn has_forbidden_html(body: &str) -> bool {
    classify_body_lines(body)
        .iter()
        .any(|line| line.forbidden_html)
}

fn require_section(lines: &[BodyLine<'_>], name: &str) -> Result<(), String> {
    let heading = format!("## {name}");
    let positions = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.heading == Some(heading.as_str())).then_some(index))
        .collect::<Vec<_>>();
    match positions.as_slice() {
        [] => Err(format!("canonical body requires section `{heading}`")),
        [position] => {
            let has_content = lines[position + 1..]
                .iter()
                .take_while(|line| line.heading.is_none())
                .any(|line| line.has_content);
            if has_content {
                Ok(())
            } else {
                Err(format!(
                    "canonical body section `{heading}` must not be empty"
                ))
            }
        },
        _ => Err(format!(
            "canonical body section `{heading}` appears more than once"
        )),
    }
}

struct BodyLine<'a> {
    heading: Option<&'a str>,
    has_content: bool,
    forbidden_html: bool,
}

fn classify_body_lines(body: &str) -> Vec<BodyLine<'_>> {
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
