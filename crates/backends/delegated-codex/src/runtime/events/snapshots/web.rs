use serde_json::{Map, Value, json, to_string_pretty};
use yo_core::ToolOutput;

pub(super) fn web_search_snapshot(item: &Value) -> String {
    let mut plain_text = web_search_text(item);
    // app-server는 의도적으로 결과 항목을 불투명하게 둡니다. 새 결과 종류와 필드를
    // 범용 인용 스키마를 임의로 만들지 않고 보존합니다.
    let result = item
        .get("results")
        .filter(|value| !value.is_null())
        .map(|results| {
            plain_text.push_str(&format!("\nResults:\n{results:#}"));
            json!({"results":results})
        });
    let arguments = ["query", "action"]
        .into_iter()
        .filter_map(|key| item.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect::<Map<_, _>>();
    ToolOutput {
        tool: "webSearch".to_owned(),
        server: None,
        arguments: Some(Value::Object(arguments)),
        result,
        content_items: None,
        error: None,
        plain_text: plain_text.clone(),
    }
    .to_snapshot()
    .unwrap_or(plain_text)
}

fn web_search_text(item: &Value) -> String {
    let action = item.get("action").filter(|value| !value.is_null());
    let action_type = action
        .and_then(|action| action.get("type"))
        .and_then(Value::as_str);
    let mut lines = vec![
        match action_type {
            Some("openPage") => "Open web page",
            Some("findInPage") => "Find in web page",
            Some("search") => "Web search",
            None if action.is_none() => "Web search",
            _ => "Web search · other action",
        }
        .to_owned(),
    ];
    let mut queries = Vec::new();
    if let Some(query) = action
        .and_then(|action| action.get("query"))
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
    {
        queries.push(query);
    }
    if let Some(values) = action
        .and_then(|action| action.get("queries"))
        .and_then(Value::as_array)
    {
        for query in values
            .iter()
            .filter_map(Value::as_str)
            .filter(|query| !query.is_empty())
        {
            if !queries.contains(&query) {
                queries.push(query);
            }
        }
    }
    lines.extend(queries.into_iter().map(|query| format!("Query: {query}")));
    for (field, label) in [("url", "URL"), ("pattern", "Find")] {
        if let Some(value) = action
            .and_then(|action| action.get(field))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("{label}: {value}"));
        }
    }
    // 사용 가능한 작업 세부 정보가 없으면 Codex는 항목 수준 질의를 유지합니다.
    if lines.len() == 1
        && let Some(query) = item
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !query.is_empty())
    {
        lines.push(format!("Query: {query}"));
    }
    if let Some(action) = action {
        let known_fields: &[&str] = match action_type {
            Some("search") => &["type", "query", "queries"],
            Some("openPage") => &["type", "url"],
            Some("findInPage") => &["type", "url", "pattern"],
            _ => &[],
        };
        let mut extra = action.clone();
        if let Some(fields) = extra.as_object_mut() {
            for field in known_fields {
                let valid = match *field {
                    "queries" => fields.get(*field).is_some_and(|value| {
                        value.is_null()
                            || value
                                .as_array()
                                .is_some_and(|values| values.iter().all(Value::is_string))
                    }),
                    _ => fields
                        .get(*field)
                        .is_some_and(|value| value.is_null() || value.is_string()),
                };
                if valid {
                    fields.remove(*field);
                }
            }
        }
        if !extra.as_object().is_some_and(Map::is_empty) {
            lines.push(to_string_pretty(&extra).expect("JSON is serializable"));
        }
    }
    if lines.len() == 1 {
        lines.push("Details not reported".to_owned());
    }
    lines.join("\n")
}
