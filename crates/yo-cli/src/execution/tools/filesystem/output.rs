pub(super) fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(2));
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0000}'..='\u{001F}' => {
                use std::fmt::Write as _;
                write!(&mut output, "\\u{:04x}", u32::from(character))
                    .expect("writing to a String cannot fail");
            },
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

pub(super) fn error(path: &str, class: &str) -> String {
    format!(
        "{{\"path\":{},\"status\":\"error\",\"error\":{}}}",
        json_string(path),
        json_string(class)
    )
}

#[cfg(test)]
mod tests {
    use super::json_string;

    // 모델에 저장되는 compact JSON 문자열은 지정된 짧은 escape와 lowercase control
    // escape만 사용하고 나머지 Unicode는 원래 UTF-8로 유지합니다.
    #[test]
    fn json_string_uses_the_closed_escape_rule() {
        assert_eq!(
            json_string("한글\0\u{000b}\n\t\"\\"),
            "\"한글\\u0000\\u000b\\n\\t\\\"\\\\\""
        );
    }
}
