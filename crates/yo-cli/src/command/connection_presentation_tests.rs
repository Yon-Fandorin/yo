#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use yo_core::{CompleteModelBinding, CredentialMutationAction};
    use yo_tui::surface::{GraphemeError, cell_width};

    use crate::{
        command::{
            connect::presentation::{
                Confirmation as ConnectConfirmation, ConnectPreview, StoredConnectionChange,
                connect_success, connect_success_with, import_success_with,
            },
            disconnect::presentation::{
                Confirmation as DisconnectConfirmation, DisconnectEffect, DisconnectImpact,
                DisconnectPreview, RemainingBinding, disconnect_success_with,
            },
        },
        connection::presentation::*,
        presentation::PresentationStyle,
    };

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
        BindingDetails::from(
            &CompleteModelBinding::from_durable_json(&durable.to_string()).unwrap(),
        )
    }

    // 기본 confirmation은 적용 판단에 필요한 change set과 요약만 보여 주며, exact profile은
    // -v를 선택한 경우에만 노출해 반복 실행의 기본 화면을 짧게 유지합니다.
    #[test]
    fn compact_connect_preview_hides_exact_profile_until_verbose() {
        let preview = ConnectConfirmation::Connect(Box::new(ConnectPreview::new(
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
        let preview = ConnectConfirmation::Connect(Box::new(
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
        assert!(
            output.find("~ Default model").unwrap() < output.find("Connection profile").unwrap()
        );
        assert!(output.ends_with("Plan: 2 to add, 1 to change."));
    }

    // 여러 모델 목록은 쉼표 경계에서만 다음 줄로 넘겨 충분한 폭이 있는데도 긴 모델 ID를
    // 중간에서 자르지 않으며, 각 모델을 정확히 한 번씩 그대로 보여 줍니다.
    #[test]
    fn model_list_wraps_between_models_without_splitting_identifiers() {
        let models = [
            "anthropic/claude-opus-4",
            "deepseek/deepseek-r1",
            "mistralai/mistral-large",
        ];

        let lines = wrap_list(&models, 30).unwrap();

        assert_eq!(
            lines,
            [
                "anthropic/claude-opus-4,",
                "deepseek/deepseek-r1,",
                "mistralai/mistral-large",
            ]
        );
    }

    // 쉼표나 내부 공백이 허용된 Model ID는 일반 ID와 섞여도 따옴표로 경계를 보존하며,
    // compact와 verbose 목록이 같은 두 모델을 서로 다른 항목으로 표시합니다.
    #[test]
    fn model_lists_quote_delimiter_bearing_identifiers_in_both_views() {
        let render = |verbose| {
            ConnectConfirmation::Connect(Box::new(
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

    // Model ID 자체는 inline 폭에 맞지만 뒤 구분자까지 맞지 않으면 쉼표를 별도 줄로
    // 떼거나 ID를 자르지 않고 명확한 bullet 항목으로 전환합니다.
    #[test]
    fn model_list_uses_bullets_when_an_inline_separator_would_not_fit() {
        let mut output = String::new();

        push_model_list_field(
            &mut output,
            "Models",
            &["abcde", "f"],
            23,
            PresentationStyle::Plain,
        )
        .unwrap();

        assert_eq!(output, "  Models\n  • abcde\n  • f\n");
    }

    // 좁은 terminal에서도 long endpoint와 versioned profile을 Yo가 직접 grapheme 단위로
    // 감싸 모든 물리 줄을 폭 안에 두며, 정보 손실 없이 다시 이어 읽을 수 있습니다.
    #[test]
    fn narrow_preview_wraps_every_line_without_losing_exact_values() {
        let binding = fixture_binding();
        let preview = ConnectConfirmation::Connect(Box::new(
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

    // disconnect 화면은 실제 변화와 Session 영향이 제거 상세보다 먼저 나오고, 남는 모델은
    // 식별에 필요한 reference만 보여 제거 profile 전체를 반복하지 않습니다.
    #[test]
    fn disconnect_preview_prioritizes_effects_and_compacts_remaining_models() {
        let removed = fixture_binding();
        let preview = DisconnectConfirmation::Disconnect(Box::new(DisconnectPreview::new(
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
                    "Can resume after this exact stored model is restored; history is kept"
                        .to_owned(),
                ),
            ),
            vec![RemainingBinding {
                model: "alpha".to_owned(),
            }],
            true,
        )));

        let output = preview.render(width(80)).unwrap();

        assert!(output.starts_with("DISCONNECT  vendor:team:alpha"));
        assert!(
            output.find("= API key").unwrap() < output.find("Connection being removed").unwrap()
        );
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
            let preview = DisconnectConfirmation::Disconnect(Box::new(DisconnectPreview::new(
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
        let preview = DisconnectConfirmation::Disconnect(Box::new(DisconnectPreview::new(
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

    // structured parameter의 연속 공백도 의미 있는 문자열 bytes일 수 있으므로 wrapping 전후
    // 조각을 이어 붙이면 원문과 같고 split_whitespace식 정규화가 일어나지 않습니다.
    #[test]
    fn wrapping_preserves_exact_whitespace_in_profile_values() {
        let original = r#"{"note":"a  b"}"#;
        let wrapped = wrap(original, 7).unwrap();

        assert_eq!(wrapped.concat(), original);
        assert!(wrapped.iter().all(|line| cell_width(line).unwrap() <= 7));
    }

    // ANSI 장식은 TTY 전용 styled 경로에만 들어가고 같은 preview의 평문 경로는 로그와
    // snapshot 비교에 안전한 의미 marker를 그대로 유지합니다.
    #[test]
    fn ansi_style_decorates_semantics_without_changing_plain_output() {
        let preview = ConnectConfirmation::Connect(Box::new(ConnectPreview::new(
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
        let preview = ConnectConfirmation::Connect(Box::new(
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

    // 독립 default-ignorable grapheme은 폭 0이라 보이지 않는 내용을 confirmation에
    // 몰래 섞을 수 있으므로 일반 문자처럼 허용하지 않고 terminal-safe 경계에서 거절합니다.
    #[test]
    fn wrapping_rejects_an_isolated_zero_width_grapheme() {
        assert!(matches!(
            wrap("\u{200b}", 80),
            Err(PresentationError::UnsafeText(GraphemeError::ZeroWidth))
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
            let ansi_presentation = SuccessPresentation {
                width,
                style: PresentationStyle::Ansi,
            };
            let outputs = [
                (
                    connect_success_with(plain_presentation, &target, 2, &target).unwrap(),
                    connect_success_with(ansi_presentation, &target, 2, &target).unwrap(),
                ),
                (
                    import_success_with(plain_presentation, &target, 2, &target).unwrap(),
                    import_success_with(ansi_presentation, &target, 2, &target).unwrap(),
                ),
                (
                    disconnect_success_with(plain_presentation, &target, "Kept", &target).unwrap(),
                    disconnect_success_with(ansi_presentation, &target, "Kept", &target).unwrap(),
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

    // terminal stdout snapshot은 nonzero winsize를 사용하고 zero 또는 non-terminal이면 80열로
    // 돌아가며, ANSI 선택은 같은 snapshot의 terminal 여부와 NO_COLOR 입력에만 따릅니다.
    #[test]
    fn success_snapshot_uses_terminal_width_with_zero_and_redirected_fallbacks() {
        use nix::pty::openpty;
        use rustix::termios::{Winsize, tcsetwinsize};

        let pty = openpty(None, None).unwrap();
        tcsetwinsize(
            &pty.slave,
            Winsize {
                ws_row: 24,
                ws_col: 37,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        assert_eq!(
            SuccessPresentation::for_output(&pty.slave, true, false),
            SuccessPresentation {
                width: width(37),
                style: PresentationStyle::Ansi,
            }
        );
        assert_eq!(
            SuccessPresentation::for_output(&pty.slave, true, true).style,
            PresentationStyle::Plain
        );

        tcsetwinsize(
            &pty.slave,
            Winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        assert_eq!(
            SuccessPresentation::for_output(&pty.slave, true, false).width,
            default_width()
        );

        let redirected = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(
            SuccessPresentation::for_output(&redirected, false, false),
            SuccessPresentation {
                width: default_width(),
                style: PresentationStyle::Plain,
            }
        );
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
}
