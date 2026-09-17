use serde_json::Value;

pub(in crate::runtime::events) fn file_change_snapshot(item: &Value) -> Option<String> {
    let changes = item.get("changes")?.as_array()?;
    let lines = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path")?.as_str()?;
            let kind = change.get("kind");
            let name = kind
                .and_then(|kind| kind.as_str().or_else(|| kind.get("type")?.as_str()))
                .unwrap_or("update");
            let mut text = format!("{name}: {path}");
            if let Some(destination) = kind
                .and_then(|kind| kind.get("move_path"))
                .and_then(Value::as_str)
            {
                text.push_str(&format!(" -> {destination}"));
            }
            if let Some(diff) = change
                .get("diff")
                .and_then(Value::as_str)
                .filter(|diff| !diff.is_empty())
            {
                text.push('\n');
                match name {
                    "add" | "delete" => {
                        // app-server는 추가/삭제에 전체 파일 콘텐츠를 보내고,
                        // update만 unified diff를 보냅니다.
                        let prefix = if name == "add" { '+' } else { '-' };
                        for line in diff.split_inclusive('\n') {
                            text.push(prefix);
                            text.push_str(line);
                        }
                        if !diff.ends_with('\n') {
                            text.push_str("\n\\ No newline at end of file");
                        }
                    },
                    _ => text.push_str(diff),
                }
            }
            Some(text)
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}
