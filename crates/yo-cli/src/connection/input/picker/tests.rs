use std::{
    io::Read as _,
    panic::AssertUnwindSafe,
    thread,
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    pty::openpty,
    sys::termios::tcgetattr,
};

use super::*;

fn choices(count: usize) -> Vec<PickerChoice> {
    (0..count)
        .map(|index| PickerChoice {
            display_name: format!("Model {index:02}"),
            model_id: format!("vendor/model-{index:02}"),
            input_limit: Some(100_000 + index as u64),
            output_limit: Some(8_000),
            tool_policy: Some("local-tools/v1".to_owned()),
            reasoning: Some(index.is_multiple_of(2)),
            reasoning_label: None,
            badges: Vec::new(),
            enabled: true,
            disabled_reason: None,
        })
        .collect()
}

fn identity() -> PickerIdentity {
    PickerIdentity {
        provider: "openrouter".to_owned(),
        account: "team".to_owned(),
    }
}

// 여덟 행보다 큰 catalog에서도 Down이 모든 결과에 도달하고 viewport가 선택을 따라가며
// 끝에서 wrap하지 않는지 순수 상태로 판별합니다.
#[test]
fn viewport_reaches_every_result_and_clamps_without_wrapping() {
    let choices = choices(12);
    let mut state = PickerState::new(&choices);
    for _ in 0..20 {
        state.move_down();
    }
    assert_eq!(state.selected_model_index(), Some(11));
    assert_eq!(state.viewport_start, 4);
    state.move_down();
    assert_eq!(state.selected_model_index(), Some(11));
    for _ in 0..20 {
        state.move_up();
    }
    assert_eq!(state.selected_model_index(), Some(0));
    assert_eq!(state.viewport_start, 0);
}

// 검색 edit는 name과 ID를 Unicode-normalized case-insensitive로 다시 계산하고 첫 결과로
// 돌아가며, zero match의 Enter 대상은 None인 채 picker를 닫지 않는 상태를 보존합니다.
#[test]
fn query_edits_reset_selection_and_allow_a_recoverable_empty_result() {
    let choices = vec![
        PickerChoice {
            display_name: "Alpha".to_owned(),
            model_id: "vendor/one".to_owned(),
            input_limit: Some(1),
            output_limit: Some(1),
            tool_policy: Some("no-tools/v1".to_owned()),
            reasoning: Some(false),
            reasoning_label: None,
            badges: Vec::new(),
            enabled: true,
            disabled_reason: None,
        },
        PickerChoice {
            display_name: "ＢＥＴＡ".to_owned(),
            model_id: "vendor/two".to_owned(),
            input_limit: Some(1),
            output_limit: Some(1),
            tool_policy: Some("no-tools/v1".to_owned()),
            reasoning: Some(false),
            reasoning_label: None,
            badges: Vec::new(),
            enabled: true,
            disabled_reason: None,
        },
    ];
    let mut state = PickerState::new(&choices);
    state.query = "beta".to_owned();
    state.recompute(&choices);
    assert_eq!(state.selected_model_index(), Some(1));
    state.query = "missing".to_owned();
    state.recompute(&choices);
    assert_eq!(state.selected_model_index(), None);
    state.pop_query(&choices);
    assert_eq!(state.selected_model_index(), None);
}

// disabled 행의 Enter는 picker를 닫거나 선택 index를 반환하지 않고 정확한 이유를
// panel에 노출하며, 다음 enabled 행으로 이동하면 정상 선택할 수 있습니다.
#[test]
fn disabled_enter_keeps_the_picker_active_and_exposes_the_reason() {
    let mut choices = choices(2);
    choices[0].enabled = false;
    choices[0].disabled_reason = Some("text output unsupported".to_owned());
    choices[0].output_limit = None;
    choices[0].tool_policy = None;
    choices[0].reasoning = None;
    let mut state = PickerState::new(&choices);

    assert_eq!(state.accept_selected(&choices), None);
    let rendered =
        render_lines(&identity(), &state, &choices, 120, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("Unavailable  text output unsupported"));
    assert!(rendered.contains("? out"));

    state.move_down();
    assert_eq!(state.accept_selected(&choices), Some(1));
}

// remote UTF-8, quote, backslash, newline, ESC를 모두 printable ASCII byte escape로 바꿔
// 한 행과 terminal control 경계를 깨지 않고 원래 byte identity를 식별할 수 있게 합니다.
#[test]
fn remote_text_is_reversibly_escaped_before_rendering() {
    assert_eq!(
        escape_remote_text("a\"\\\n\u{1b}한"),
        "a\\x22\\x5C\\x0A\\x1B\\xED\\x95\\x9C"
    );
    let mut state = PickerState::new(&choices(1));
    state.query = "\u{1b}]0;owned".to_owned();
    let rendered = render_lines(
        &identity(),
        &state,
        &choices(1),
        80,
        PresentationStyle::Plain,
    )
    .join("\n");
    assert!(!rendered.contains("\u{1b}]0;owned"));
    assert!(rendered.contains("\\x1B]0;owned"));
}

// 같은 model 목록이라도 어느 Provider·Account의 discovery인지 정확히 한 번 식별하고,
// 좁은 terminal에서도 둘을 자르지 않으며 각 결과 행에는 반복하지 않는지 판별합니다.
#[test]
fn panel_identifies_the_provider_and_account_once_without_narrow_width_clipping() {
    let choices = choices(2);
    let lines = render_lines(
        &identity(),
        &PickerState::new(&choices),
        &choices,
        12,
        PresentationStyle::Plain,
    );
    let unwrapped = lines.join("");
    assert_eq!(unwrapped.matches("Provider  openrouter").count(), 1);
    assert_eq!(unwrapped.matches("Account  team").count(), 1);
    assert!(unwrapped.contains("Provider  openrouterAccount  team"));
    assert_eq!(unwrapped.matches("openrouter").count(), 1);
    assert_eq!(unwrapped.matches("team").count(), 1);
}

// Kimi inventory의 실제 typed 행을 picker 입력으로 넘겨 K3/K2.7 badge와 reasoning 상태,
// 미검토 일반 K2.7의 비활성 사유가 최종 panel까지 손실 없이 전달되는지 판별합니다.
#[test]
fn kimi_inventory_fields_reach_the_rendered_picker_panel() {
    let seed = yo_core::KimiCatalogSeed::resolve(
        yo_core::VersionedProfileId::new("kimi-platform-ai/v1").unwrap(),
        yo_core::ProviderId::new("kimi").unwrap(),
        yo_core::AccountId::new("team").unwrap(),
        None,
        None,
    )
    .unwrap();
    let snapshot = br#"{"object":"list","data":[
        {"object":"model","id":"kimi-k3","context_length":1048576},
        {"object":"model","id":"kimi-k2.7-code-highspeed","context_length":262144},
        {"object":"model","id":"kimi-k2.7","context_length":262144}
    ]}"#;
    let items = yo_core::parse_kimi_catalog_snapshot(&seed, snapshot)
        .unwrap()
        .iter()
        .map(ModelPickerItem::from_kimi)
        .collect::<Vec<_>>();
    let identity = PickerIdentity::from_models(&items).unwrap();
    let choices = items.iter().map(PickerChoice::from).collect::<Vec<_>>();
    let mut state = PickerState::new(&choices);
    state.query = "kimi-k3".to_owned();
    state.recompute(&choices);
    let rendered =
        render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("reasoning required/max · recommended · ready"));

    state.query = "highspeed".to_owned();
    state.recompute(&choices);
    let rendered =
        render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("reasoning required · high-speed · ready"));

    state.query = "kimi-k2.7".to_owned();
    state.recompute(&choices);
    assert_eq!(state.accept_selected(&choices), None);
    let rendered =
        render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("Unavailable  profile unavailable"));
}

// Code membership picker는 k3-256k를 기본 권장 행으로, Code high-speed 변형을 별도
// badge로 표시하고 Platform 이름을 섞지 않은 채 exact ModelId를 선택합니다.
#[test]
fn kimi_code_inventory_renders_recommendation_and_high_speed_badges() {
    let seed = yo_core::KimiCatalogSeed::resolve(
        yo_core::VersionedProfileId::new("kimi-code-membership/v1").unwrap(),
        yo_core::ProviderId::new("kimi").unwrap(),
        yo_core::AccountId::new("coding").unwrap(),
        None,
        None,
    )
    .unwrap();
    let snapshot = br#"{"object":"list","data":[
        {"object":"model","id":"k3","context_length":262144},
        {"object":"model","id":"k3-256k","context_length":262144},
        {"object":"model","id":"kimi-for-coding-highspeed","context_length":262144}
    ]}"#;
    let items = yo_core::parse_kimi_catalog_snapshot(&seed, snapshot)
        .unwrap()
        .iter()
        .map(ModelPickerItem::from_kimi)
        .collect::<Vec<_>>();
    let identity = PickerIdentity::from_models(&items).unwrap();
    let choices = items.iter().map(PickerChoice::from).collect::<Vec<_>>();
    let mut state = PickerState::new(&choices);

    state.query = "k3-256k".to_owned();
    state.recompute(&choices);
    let rendered =
        render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("reasoning required/high · recommended · ready"));

    state.query = "highspeed".to_owned();
    state.recompute(&choices);
    let rendered =
        render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");
    assert!(rendered.contains("reasoning required · high-speed · ready"));
}

// 양쪽 제품 inventory에 다른 제품의 K3 ModelId가 나타나도 disabled 행에 그 제품의
// reviewed reasoning 문구를 붙이지 않아 ModelId만으로 제품 profile을 추론하지 않습니다.
#[test]
fn kimi_cross_product_rows_do_not_borrow_reasoning_presentation() {
    for (profile, foreign_model, forbidden_label) in [
        ("kimi-platform-ai/v1", "k3", "reasoning required/high"),
        (
            "kimi-code-membership/v1",
            "kimi-k3",
            "reasoning required/max",
        ),
    ] {
        let seed = yo_core::KimiCatalogSeed::resolve(
            yo_core::VersionedProfileId::new(profile).unwrap(),
            yo_core::ProviderId::new("kimi").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            None,
            None,
        )
        .unwrap();
        let snapshot = format!(
            r#"{{"object":"list","data":[{{"object":"model","id":"{foreign_model}","context_length":1048576}}]}}"#
        );
        let item = yo_core::parse_kimi_catalog_snapshot(&seed, snapshot.as_bytes())
            .unwrap()
            .into_iter()
            .map(|model| ModelPickerItem::from_kimi(&model))
            .next()
            .unwrap();
        let identity = PickerIdentity::from_models(std::slice::from_ref(&item)).unwrap();
        let choices = [PickerChoice::from(&item)];
        let mut state = PickerState::new(&choices);
        assert_eq!(state.accept_selected(&choices), None);
        let rendered =
            render_lines(&identity, &state, &choices, 160, PresentationStyle::Plain).join("\n");

        assert!(
            rendered.contains("Unavailable  profile unavailable"),
            "rendered panel:\n{rendered}"
        );
        assert!(
            !rendered.contains(forbidden_label),
            "rendered panel:\n{rendered}"
        );
    }
}

// K2.6은 실행 시 thinking off가 고정되므로 remote capability가 없거나 malformed여도
// generic reasoning-unknown으로 보이지 않고 정확히 unknown/off를 표시합니다.
#[test]
fn kimi_k26_missing_or_malformed_reasoning_is_explicitly_unknown_and_off() {
    let seed = yo_core::KimiCatalogSeed::resolve(
        yo_core::VersionedProfileId::new("kimi-platform-ai/v1").unwrap(),
        yo_core::ProviderId::new("kimi").unwrap(),
        yo_core::AccountId::new("team").unwrap(),
        None,
        None,
    )
    .unwrap();
    for row in [
        r#"{"object":"model","id":"kimi-k2.6","context_length":262144}"#,
        r#"{"object":"model","id":"kimi-k2.6","context_length":262144,"supports_reasoning":"unknown"}"#,
    ] {
        let snapshot = format!(r#"{{"object":"list","data":[{row}]}}"#);
        let model = yo_core::parse_kimi_catalog_snapshot(&seed, snapshot.as_bytes())
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let choice = PickerChoice::from(&ModelPickerItem::from_kimi(&model));
        assert_eq!(
            choice.reasoning_label.as_deref(),
            Some("reasoning unknown/off")
        );
        let rendered = render_lines(
            &PickerIdentity {
                provider: "kimi".to_owned(),
                account: "team".to_owned(),
            },
            &PickerState::new(std::slice::from_ref(&choice)),
            std::slice::from_ref(&choice),
            160,
            PresentationStyle::Plain,
        )
        .join("\n");
        assert!(rendered.contains("reasoning unknown/off"), "{rendered}");
    }
}

// 실제 PTY에서 picker가 raw mode와 숨긴 cursor를 소유한 채 panic해도 Drop 경계가
// exact termios를 복구하고 dynamic panel을 지운 뒤 cursor를 다시 보이는지 확인합니다.
#[test]
fn panic_unwind_restores_terminal_mode_and_cleans_the_panel() {
    let pty = openpty(None, None).unwrap();
    let observed = pty.slave.try_clone().unwrap();
    let original = tcgetattr(&observed).unwrap();
    let terminal = File::from(pty.slave);
    let mut master = File::from(pty.master);
    fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
    let reader = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    output.extend_from_slice(&buffer[..count]);
                    if output.ends_with(b"\x1b[?25h") {
                        break;
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "picker cleanup did not reach the PTY peer"
                    );
                    thread::sleep(Duration::from_millis(1));
                },
                Err(error) => panic!("reading picker PTY failed: {error}"),
            }
        }
        output
    });
    let choices = choices(2);
    let state = PickerState::new(&choices);

    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut scope = PickerTerminalScope::enter(&terminal).unwrap();
        scope
            .render(&identity(), &state, &choices, PresentationStyle::Ansi)
            .unwrap();
        panic!("injected picker panic");
    }));
    assert!(panic.is_err());
    assert_eq!(tcgetattr(&observed).unwrap(), original);

    let output = reader.join().unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\x1b[?25l"));
    assert!(output.contains("\x1b[J"));
    assert!(output.ends_with("\x1b[?25h"));
}
