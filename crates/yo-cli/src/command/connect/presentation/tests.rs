use std::num::NonZeroU16;

use yo_core::{CompleteModelBinding, CredentialMutationAction};
use yo_tui::surface::cell_width;

use super::*;

fn width(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn fixture_binding() -> BindingDetails {
    fixture_binding_for_model("alpha")
}

fn fixture_binding_for_model(model: &str) -> BindingDetails {
    let mut durable = serde_json::from_str::<serde_json::Value>(
        r#"{"provider":"vendor","account":"team","model":"alpha","connector":"openai-responses","base_url":"https://long-provider.example.test/compatible-mode/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":4096,"max_output_tokens":128,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap();
    durable["model"] = serde_json::Value::String(model.to_owned());
    BindingDetails::from(&CompleteModelBinding::from_durable_json(&durable.to_string()).unwrap())
}

// 기본 confirmation은 적용 판단에 필요한 change set과 요약만 보여 주며, exact profile은
// -v를 선택한 경우에만 노출해 반복 실행의 기본 화면을 짧게 유지합니다.
#[test]
fn compact_connect_preview_hides_exact_profile_until_verbose() {
    let preview = Confirmation::Connect(Box::new(ConnectPreview::new(
        "vendor:team:alpha".to_owned(),
        "vendor:team".to_owned(),
        "unset  →  vendor:team:alpha".to_owned(),
        StoredConnectionChange::Create,
        CredentialMutationAction::Add,
        true,
        vec![fixture_binding()],
    )));

    let output = preview.render(width(80)).unwrap();

    assert!(output.contains("Yo will make these changes:\n+ Stored connection"));
    assert!(output.contains("+ API key\n  Save vendor:team · register 1 model"));
    assert!(output.contains("  Models          alpha"));
    assert!(output.contains("~ Default model\n  unset  →  vendor:team:alpha"));
    assert!(!output.contains("Connection profile"));
    assert!(!output.contains("long-provider.example.test"));
    assert!(output.ends_with("Plan: 2 to add, 1 to change."));
}

// 등록 성공 요약은 model profile 검증을 주장하지 않고 게시된 모델 하나를 정확히 셉니다.
#[test]
fn connect_success_reports_one_registered_model() {
    assert_eq!(
        connect_success("vendor:team:alpha", 1, "vendor:team:alpha").unwrap(),
        "✓ Connected\n\n  Model       vendor:team:alpha\n  Registered  1 model profile\n  Default     vendor:team:alpha\n"
    );
}

// 80열 connect 확인 화면은 먼저 사람이 결정할 대상·API key·default를 보여 주고,
// exact profile 세부정보를 이름 있는 보조 영역에 배치합니다.
#[test]
fn connect_preview_prioritizes_the_decision_before_exact_details() {
    let preview = Confirmation::Connect(Box::new(
        ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            StoredConnectionChange::Create,
            CredentialMutationAction::Add,
            true,
            vec![fixture_binding()],
        )
        .with_verbose(true),
    ));

    let output = preview.render(width(80)).unwrap();

    assert!(output.starts_with("CONNECT  vendor:team:alpha"));
    assert!(output.contains("Yo will make these changes:\n+ Stored connection"));
    assert!(output.contains("+ API key\n  Save vendor:team"));
    assert!(output.contains("Connection profile"));
    assert!(output.contains("  Models (1)      alpha"));
    assert!(output.contains("  Endpoint"));
    assert!(output.contains("  Request options"));
    assert!(output.find("~ Default model").unwrap() < output.find("Connection profile").unwrap());
    assert!(output.ends_with("Plan: 2 to add, 1 to change."));
}

// 쉼표나 내부 공백이 허용된 Model ID는 일반 ID와 섞여도 따옴표로 경계를 보존하며,
// compact와 verbose 목록이 같은 두 모델을 서로 다른 항목으로 표시합니다.
#[test]
fn model_lists_quote_delimiter_bearing_identifiers_in_both_views() {
    let render = |verbose| {
        Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:a".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:a".to_owned(),
                StoredConnectionChange::Create,
                CredentialMutationAction::Add,
                true,
                vec![
                    fixture_binding_for_model("a"),
                    fixture_binding_for_model("b, c"),
                ],
            )
            .with_verbose(verbose),
        ))
        .render(width(80))
        .unwrap()
    };

    let compact = render(false);
    let verbose = render(true);

    assert!(compact.contains("Models          a, \"b, c\""));
    assert_eq!(verbose.matches("Models (2)      a, \"b, c\"").count(), 1);
}

// 좁은 terminal에서도 long endpoint와 versioned profile을 Yo가 직접 grapheme 단위로
// 감싸 모든 물리 줄을 폭 안에 두며, 정보 손실 없이 다시 이어 읽을 수 있습니다.
#[test]
fn narrow_preview_wraps_every_line_without_losing_exact_values() {
    let binding = fixture_binding();
    let preview = Confirmation::Connect(Box::new(
        ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            StoredConnectionChange::Create,
            CredentialMutationAction::Add,
            true,
            vec![binding],
        )
        .with_verbose(true),
    ));

    let output = preview.render(width(36)).unwrap();

    for line in output.lines() {
        assert!(
            cell_width(line).unwrap() <= 36,
            "overwide connection-preview line: {line:?}"
        );
    }
    let compact = output.split_whitespace().collect::<String>();
    assert!(compact.contains("https://long-provider.example.test/compatible-mode/v1"));
    assert!(compact.contains("utf8-bytes/v1"));
    assert!(compact.contains("semantic-only/v1"));
}

// ANSI 장식은 TTY 전용 styled 경로에만 들어가고 같은 preview의 평문 경로는 로그와
// snapshot 비교에 안전한 의미 marker를 그대로 유지합니다.
#[test]
fn ansi_style_decorates_semantics_without_changing_plain_output() {
    let preview = Confirmation::Connect(Box::new(ConnectPreview::new(
        "vendor:team:alpha".to_owned(),
        "vendor:team".to_owned(),
        "unset  →  vendor:team:alpha".to_owned(),
        StoredConnectionChange::Create,
        CredentialMutationAction::Replace,
        true,
        vec![fixture_binding()],
    )));

    let plain = preview.render(width(80)).unwrap();
    let styled = preview
        .render_styled(width(80), PresentationStyle::Ansi)
        .unwrap();

    assert!(!plain.contains('\u{1b}'));
    assert!(styled.contains("\u{1b}[1;36mCONNECT\u{1b}[0m"));
    assert!(styled.contains("\u{1b}[33m~\u{1b}[0m"));
    assert_eq!(strip_ansi(&styled), plain);
}

// 제목보다도 좁은 1~15열 terminal에서 제목과 모든 본문을 자체 줄바꿈해 ANSI를
// 제외한 각 물리 줄이 관찰한 폭을 넘지 않습니다.
#[test]
fn every_heading_fits_terminals_narrower_than_the_title() {
    let preview = Confirmation::Connect(Box::new(
        ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            StoredConnectionChange::Create,
            CredentialMutationAction::Add,
            true,
            vec![fixture_binding()],
        )
        .with_verbose(true),
    ));

    for columns in 1..=15 {
        let output = preview.render(width(columns)).unwrap();
        for line in output.lines() {
            assert!(
                cell_width(line).unwrap() <= usize::from(columns),
                "{columns}-cell terminal received overwide line {line:?}"
            );
        }
    }
}

// stdout success presenter는 1~80열의 모든 ASCII identity를 손실 없이 자체 줄바꿈하고,
// ANSI는 장식만 더하므로 제거하면 같은 plain 의미와 정확한 값을 복원할 수 있습니다.
#[test]
fn success_presentations_fit_every_width_and_preserve_ascii_values() {
    let target = format!(
        "{}:{}:{}",
        "p".repeat(256),
        "a".repeat(256),
        "m".repeat(256)
    );
    for columns in 1..=80 {
        let width = width(columns);
        let plain_presentation = SuccessPresentation::plain(width);
        let ansi_presentation = SuccessPresentation::ansi(width);
        let outputs = [
            (
                connect_success_with(plain_presentation, &target, 2, &target).unwrap(),
                connect_success_with(ansi_presentation, &target, 2, &target).unwrap(),
            ),
            (
                import_success_with(plain_presentation, &target, 2, &target).unwrap(),
                import_success_with(ansi_presentation, &target, 2, &target).unwrap(),
            ),
        ];

        for (plain, styled) in outputs {
            let stripped = strip_ansi(&styled);
            assert_eq!(stripped, plain);
            let compact = plain
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            assert!(compact.matches(&target).count() >= 2);
            for line in stripped.lines() {
                assert!(
                    cell_width(line).unwrap() <= usize::from(columns),
                    "{columns}-cell success output received overwide line {line:?}"
                );
            }
        }
    }
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::new();
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            characters.next();
            for next in characters.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}
