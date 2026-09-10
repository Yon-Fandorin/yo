use super::*;

// 템플릿은 Unicode, 줄바꿈, 참조 모양 텍스트와 셸 기호를 변경 없이 보존합니다.
#[test]
fn literal_bodies_and_stable_names_are_preserved() {
    let body = "  한글 👩‍💻\n\t$skill @file $(command) {{variable}}\r\n";
    let prompts = PromptTemplates::new(BTreeMap::from([
        ("z-last".to_owned(), "last".to_owned()),
        ("review".to_owned(), body.to_owned()),
    ]))
    .unwrap();
    assert_eq!(prompts.get("review"), Some(body));
    assert_eq!(prompts.get("Review"), None);
    assert_eq!(prompts.names().collect::<Vec<_>>(), ["review", "z-last"]);
}

// 단일 본문과 전체 본문은 제한값까지 허용하고 첫 초과 바이트부터 거절합니다.
#[test]
fn body_and_total_limits_reject_the_first_excess_byte() {
    let make = |body| BTreeMap::from([("review".to_owned(), body)]);
    assert!(PromptTemplates::new(make("a".repeat(65536))).is_ok());
    assert_eq!(
        PromptTemplates::new(make("a".repeat(65537))),
        Err(PromptTemplateError::InvalidBody)
    );
    let mut entries: BTreeMap<_, _> = (0..16)
        .map(|index| (format!("p{index}"), "a".repeat(65536)))
        .collect();
    assert!(PromptTemplates::new(entries.clone()).is_ok());
    entries.insert("excess".to_owned(), "x".to_owned());
    assert_eq!(
        PromptTemplates::new(entries),
        Err(PromptTemplateError::TotalTooLarge)
    );
}

// 이름과 개수 경계 및 터미널 제어문자를 검증해 사용자 설정을 그대로 실행하지 않습니다.
#[test]
fn names_counts_and_control_bytes_are_validated() {
    let mut entries: BTreeMap<_, _> = (0..128)
        .map(|index| (format!("p{index}"), "body".to_owned()))
        .collect();
    assert!(PromptTemplates::new(entries.clone()).is_ok());
    entries.insert("excess".to_owned(), "x".to_owned());
    assert_eq!(
        PromptTemplates::new(entries),
        Err(PromptTemplateError::TooManyEntries)
    );
    assert!(PromptTemplates::new(BTreeMap::from([("n".repeat(64), "x".to_owned())])).is_ok());
    for name in [
        "".to_owned(),
        "n".repeat(65),
        "with space".to_owned(),
        "/exit".to_owned(),
    ] {
        assert_eq!(
            PromptTemplates::new(BTreeMap::from([(name, "x".to_owned())])),
            Err(PromptTemplateError::InvalidName)
        );
    }
    for body in ["", "\0", "\x1b[31m", "\u{85}"] {
        assert_eq!(
            PromptTemplates::new(BTreeMap::from([("p".to_owned(), body.to_owned())])),
            Err(PromptTemplateError::InvalidBody)
        );
    }
}
