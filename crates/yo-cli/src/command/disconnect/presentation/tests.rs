use std::num::NonZeroU16;

use yo_core::CompleteModelBinding;
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

// disconnect 화면은 실제 변화와 Session 영향이 제거 상세보다 먼저 나오고, 남는 모델은
// 식별에 필요한 reference만 보여 제거 profile 전체를 반복하지 않습니다.
#[test]
fn disconnect_preview_prioritizes_effects_and_compacts_remaining_models() {
    let removed = fixture_binding();
    let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
        "vendor:team:alpha".to_owned(),
        "Stored model removed".to_owned(),
        removed,
        DisconnectImpact::new(
            DisconnectEffect::change("Clear vendor:team:alpha".to_owned()),
            DisconnectEffect::keep(
                "Keep it because another configured model still uses vendor:team".to_owned(),
            ),
            DisconnectEffect::attention(
                "Need another available model because the default will be cleared".to_owned(),
            ),
            DisconnectEffect::ready(
                "Can resume after this exact stored model is restored; history is kept".to_owned(),
            ),
        ),
        vec![RemainingBinding {
            model: "alpha".to_owned(),
        }],
        true,
    )));

    let output = preview.render(width(80)).unwrap();

    assert!(output.starts_with("DISCONNECT  vendor:team:alpha"));
    assert!(output.find("= API key").unwrap() < output.find("Connection being removed").unwrap());
    assert_eq!(
        output.matches("https://long-provider.example.test").count(),
        1
    );
    assert!(output.contains("Still available for this account (1)\n  • alpha"));
}

// Verbose remaining-model bullet도 compact credential 영향 목록과 같은 quoting을 써서
// 쉼표·공백·따옴표·backslash가 든 ModelId 하나를 여러 항목처럼 보이지 않게 합니다.
#[test]
fn verbose_disconnect_remaining_models_use_deterministic_quoting() {
    for model in ["alpha, beta", "alpha beta", "alpha\\beta", "alpha\"beta"] {
        let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
            "vendor:team:removed".to_owned(),
            "Stored model removed".to_owned(),
            fixture_binding(),
            DisconnectImpact::new(
                DisconnectEffect::keep("Keep default".to_owned()),
                DisconnectEffect::keep("Keep API key".to_owned()),
                DisconnectEffect::ready("Ready".to_owned()),
                DisconnectEffect::ready("Can resume".to_owned()),
            ),
            vec![RemainingBinding {
                model: model.to_owned(),
            }],
            true,
        )));

        let output = preview.render(width(24)).unwrap();
        let displayed = display_model_item(model);

        assert!(output.contains(&displayed), "rendered preview:\n{output}");
    }
}

// disconnect의 문장형 risk와 exact endpoint도 좁은 폭에서 셸의 임의 개행에 의존하지
// 않고 모든 줄이 폭 안에 남습니다.
#[test]
fn narrow_disconnect_preview_keeps_every_line_within_width() {
    let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
        "vendor:team:alpha".to_owned(),
        "Stored model removed".to_owned(),
        fixture_binding(),
        DisconnectImpact::new(
            DisconnectEffect::change("Clear vendor:team:alpha".to_owned()),
            DisconnectEffect::remove(
                "Remove it because no configured model still uses vendor:team".to_owned(),
            ),
            DisconnectEffect::attention(
                "Need another available model because the default will be cleared".to_owned(),
            ),
            DisconnectEffect::attention(
                "May not resume until this exact model is restored; history is kept".to_owned(),
            ),
        ),
        Vec::new(),
        true,
    )));

    let output = preview.render(width(36)).unwrap();

    for line in output.lines() {
        assert!(
            cell_width(line).unwrap() <= 36,
            "overwide disconnect-preview line: {line:?}"
        );
    }
    assert!(output.contains("Still available for this account (0)"));
}

// 두 셀 atomic grapheme는 2~80열에서 분할·대체 없이 보존되지만 1열에서는 보존과
// 폭 준수를 동시에 만족할 수 없어 기존 typed GraphemeExceedsWidth로 명시적으로 실패합니다.
#[test]
fn success_presentation_rejects_a_two_cell_grapheme_only_at_width_one() {
    for columns in 2..=80 {
        let output = disconnect_success_with(
            SuccessPresentation::plain(width(columns)),
            "vendor:team:한",
            "Kept",
            "vendor:team:한",
        )
        .unwrap();
        let compact = output
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(compact.matches("vendor:team:한").count() >= 2);
        assert!(
            output
                .lines()
                .all(|line| cell_width(line).unwrap() <= usize::from(columns))
        );
    }

    assert!(matches!(
        disconnect_success_with(
            SuccessPresentation::plain(width(1)),
            "vendor:team:한",
            "Kept",
            "unset",
        ),
        Err(PresentationError::GraphemeExceedsWidth {
            grapheme_width: 2,
            width: 1
        })
    ));
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
        let outputs = [(
            disconnect_success_with(plain_presentation, &target, "Kept", &target).unwrap(),
            disconnect_success_with(ansi_presentation, &target, "Kept", &target).unwrap(),
        )];

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
