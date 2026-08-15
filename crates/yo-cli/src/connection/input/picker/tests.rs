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
            input_limit: 100_000 + index as u64,
            output_limit: 8_000,
            reasoning: index.is_multiple_of(2),
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
            input_limit: 1,
            output_limit: 1,
            reasoning: false,
        },
        PickerChoice {
            display_name: "ＢＥＴＡ".to_owned(),
            model_id: "vendor/two".to_owned(),
            input_limit: 1,
            output_limit: 1,
            reasoning: false,
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
