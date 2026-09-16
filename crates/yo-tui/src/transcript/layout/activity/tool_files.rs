use serde_json::{Value, from_str};

pub(super) fn mutation_result_markdown(tool: &str, source: &str) -> Option<String> {
    let (field, label) = match tool {
        "edit_file" => ("replacements", "Applied replacements"),
        "write_file" => ("bytes", "Written bytes"),
        _ => return None,
    };
    if source.len() > 16 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let result = value.as_object()?;
    if result.len() != 3 || result.get("status")?.as_str()? != "ok" {
        return None;
    }
    let count = result.get(field)?.as_u64()?;
    let path = result.get("path")?.as_str()?;
    Some(literal_block("text", &format!("{path}\n{label}: {count}")))
}

pub(super) fn edit_markdown(arguments: &Value) -> Option<(String, bool)> {
    let array = arguments.get("edits");
    let replacements = if let Some(edits) = array {
        if arguments.get("oldText").is_some() || arguments.get("newText").is_some() {
            return None;
        }
        let edits = edits.as_array()?;
        if edits.is_empty() || edits.len() > 256 {
            return None;
        }
        edits
            .iter()
            .map(|edit| {
                if edit.as_object()?.len() != 2 {
                    return None;
                }
                Some((
                    edit.get("oldText")?.as_str()?,
                    edit.get("newText")?.as_str()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        vec![(
            arguments.get("oldText")?.as_str()?,
            arguments.get("newText")?.as_str()?,
        )]
    };
    let mut bytes = 0usize;
    for (old, new) in &replacements {
        bytes = bytes.checked_add(old.len())?.checked_add(new.len())?;
        if old.is_empty() || bytes > 256 * 1024 {
            return None;
        }
    }
    let mut sections = Vec::new();
    for (index, (old, new)) in replacements.into_iter().enumerate() {
        let mut diff = String::new();
        for (prefix, source) in [('-', old), ('+', new)] {
            for line in source.split_inclusive('\n') {
                diff.push(prefix);
                diff.push_str(line);
                if !line.ends_with('\n') {
                    diff.push_str(if prefix == '-' {
                        "\n\\ Old text has no trailing newline\n"
                    } else {
                        "\n\\ New text has no trailing newline\n"
                    });
                }
            }
        }
        sections.push(format!(
            "**Replacement {}**\n\n{}",
            index + 1,
            literal_block("diff", &diff)
        ));
    }
    Some((sections.join("\n\n"), array.is_some()))
}

pub(super) fn batch_read_markdown(source: &str) -> Option<String> {
    // Only this native tool's bounded, complete result shape becomes file panels.
    // Unknown fields or truncated JSON retain the entire literal source.
    if source.len() > 256 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let root = value.as_object()?;
    if root.len() != 1 {
        return None;
    }
    let results = root.get("results")?.as_array()?;
    if results.is_empty() || results.len() > 8 {
        return None;
    }
    let mut sections = Vec::new();
    for result in results {
        let item = result.as_object()?;
        let path = item.get("path")?.as_str()?;
        let status = item.get("status")?.as_str()?;
        match status {
            "error" => {
                if item.len() != 3 {
                    return None;
                }
                let error = item.get("error")?.as_str()?;
                sections.push(literal_block(
                    "text",
                    &format!("Read failed · {path}\n{error}"),
                ));
            },
            "ok" => {
                if item.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "path" | "status" | "start" | "end" | "total" | "content" | "next_offset"
                    )
                }) {
                    return None;
                }
                let start = item.get("start")?.as_u64()?;
                let end = item.get("end")?.as_u64()?;
                let total = item.get("total")?.as_u64()?;
                let content = item.get("content")?.as_str()?;
                let next = match item.get("next_offset") {
                    Some(value) => Some(value.as_u64()?),
                    None => None,
                };
                if total == 0 {
                    if start != 0 || end != 0 || !content.is_empty() || next.is_some() {
                        return None;
                    }
                    sections.push(literal_block("text", &format!("{path}\n(empty file)")));
                } else {
                    if start == 0
                        || end < start
                        || end > total
                        || content.lines().count() as u64 != end - start + 1
                        || next != (end < total).then(|| end + 1)
                    {
                        return None;
                    }
                    sections.push(literal_block(
                        "text",
                        &format!("{path}\nLines {start}–{end} of {total}"),
                    ));
                    sections.push(literal_block(file_language(path), content));
                    if let Some(next) = next {
                        sections.push(literal_block(
                            "text",
                            &format!("Continue reading at line {next}"),
                        ));
                    }
                }
            },
            _ => return None,
        }
    }
    Some(sections.join("\n\n"))
}

pub(super) fn file_language(path: &str) -> &'static str {
    // Presentation-only suffix inference: never resolve paths or interpret Markdown,
    // diagrams or image syntax contained in file output.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension)
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "html" | "htm" => "html",
        "css" => "css",
        "xml" => "xml",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        _ => "text",
    }
}

pub(super) fn literal_block(language: &str, source: &str) -> String {
    // A tool's literal fences must not escape into image/link presentation markup.
    let longest = source
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{source}\n{fence}")
}
