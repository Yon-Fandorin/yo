use std::time::Duration;

use super::{ControlEffect, ControlKeyPolicy};
use crate::input::{
    buffer::TextBuffer,
    event::{KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
};

const NOW: Duration = Duration::from_secs(10);

fn key(character: char, action: KeyAction) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Character(character),
        modifiers: KeyModifiers::CONTROL,
        action,
        state: KeyState::NONE,
    }
}

fn plain_key(character: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Character(character),
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Press,
        state: KeyState::NONE,
    }
}

fn escape(action: KeyAction) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Escape,
        modifiers: KeyModifiers::NONE,
        action,
        state: KeyState::NONE,
    }
}

// 실행 중인 작업에서 Esc press는 Ctrl+C와 같은 중단 요청이며 입력 내용은 보존한다.
#[test]
fn escape_interrupts_an_active_task_without_editing_input() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();
    buffer.insert("keep");

    let effect = policy.handle(escape(KeyAction::Press), true, &mut buffer, NOW);

    assert_eq!(effect, ControlEffect::InterruptTask);
    assert_eq!(buffer.as_str(), "keep");
}

// 유휴 상태의 Esc는 종료나 편집 명령으로 해석하지 않고 상위 overlay 계층이 사용할 수 있게 돌려준다.
#[test]
fn idle_escape_remains_unhandled() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    assert_eq!(
        policy.handle(escape(KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::Unhandled
    );
    assert_eq!(
        policy.handle(escape(KeyAction::Repeat), false, &mut buffer, NOW),
        ControlEffect::NoChange
    );
}

// 실행 중인 작업이 있으면 Ctrl+C는 입력을 지우거나 종료하지 않고 작업 중단을 요청한다.
#[test]
fn ctrl_c_interrupts_an_active_task_first() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();
    buffer.insert("keep");

    let effect = policy.handle(key('c', KeyAction::Press), true, &mut buffer, NOW);

    assert_eq!(effect, ControlEffect::InterruptTask);
    assert_eq!(buffer.as_str(), "keep");
}

// 작업이 없고 입력이 있으면 첫 Ctrl+C는 입력 전체만 지운다.
#[test]
fn ctrl_c_clears_nonempty_input() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();
    buffer.insert("discard");

    let effect = policy.handle(key('C', KeyAction::Press), false, &mut buffer, NOW);

    assert_eq!(effect, ControlEffect::BufferChanged);
    assert!(buffer.is_empty());
    assert_eq!(buffer.cursor_byte_index(), 0);
}

// 빈 입력에서 1초 안에 Ctrl+C를 두 번 누르면 정상 종료를 요청한다.
#[test]
fn two_empty_ctrl_c_presses_within_the_window_exit() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    assert_eq!(
        policy.handle(key('c', KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::ExitArmed
    );
    assert_eq!(
        policy.handle(
            key('c', KeyAction::Press),
            false,
            &mut buffer,
            NOW + Duration::from_millis(1_000)
        ),
        ControlEffect::Exit
    );
}

// 제한 시간이 지나거나 시간이 역행하면 다음 Ctrl+C를 새로운 첫 입력으로 취급한다.
#[test]
fn expired_or_non_monotonic_ctrl_c_rearms_exit() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    policy.handle(key('c', KeyAction::Press), false, &mut buffer, NOW);
    assert_eq!(
        policy.handle(
            key('c', KeyAction::Press),
            false,
            &mut buffer,
            NOW + Duration::from_millis(1_001)
        ),
        ControlEffect::ExitArmed
    );
    assert_eq!(
        policy.handle(
            key('c', KeyAction::Press),
            false,
            &mut buffer,
            NOW - Duration::from_secs(1)
        ),
        ControlEffect::ExitArmed
    );
}

// 키 release와 repeat만으로는 Ctrl+C 두 번 누르기나 우발 종료가 성립하지 않는다.
#[test]
fn release_and_repeat_do_not_count_as_another_ctrl_c_press() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    policy.handle(key('c', KeyAction::Press), false, &mut buffer, NOW);
    assert_eq!(
        policy.handle(key('c', KeyAction::Release), false, &mut buffer, NOW),
        ControlEffect::Unhandled
    );
    assert_eq!(
        policy.handle(key('c', KeyAction::Repeat), false, &mut buffer, NOW),
        ControlEffect::NoChange
    );
}

// 다른 키를 누르면 연속 Ctrl+C 조건이 끊기고 다음 Ctrl+C가 다시 종료를 준비한다.
#[test]
fn another_key_breaks_the_ctrl_c_sequence() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    policy.handle(key('c', KeyAction::Press), false, &mut buffer, NOW);
    assert_eq!(
        policy.handle(plain_key('x'), false, &mut buffer, NOW),
        ControlEffect::Unhandled
    );
    assert_eq!(
        policy.handle(key('c', KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::ExitArmed
    );
}

// 빈 입력의 Ctrl+D press는 두 번째 입력을 기다리지 않고 바로 정상 종료를 요청한다.
#[test]
fn ctrl_d_on_empty_input_exits_immediately() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();

    assert_eq!(
        policy.handle(key('d', KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::Exit
    );
}

// 입력 중 Ctrl+D는 커서 뒤 grapheme 하나를 지우며 끝에서는 종료하지 않는다.
#[test]
fn ctrl_d_deletes_forward_but_does_not_exit_at_nonempty_end() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();
    buffer.insert("A가");
    buffer.move_left();

    assert_eq!(
        policy.handle(key('d', KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::BufferChanged
    );
    assert_eq!(buffer.as_str(), "A");
    assert_eq!(
        policy.handle(key('d', KeyAction::Press), false, &mut buffer, NOW),
        ControlEffect::NoChange
    );
    assert_eq!(buffer.as_str(), "A");
}

// Ctrl 이외의 modifier가 함께 있으면 Ctrl+C/D 계약으로 가로채지 않는다.
#[test]
fn additional_modifiers_are_not_treated_as_control_commands() {
    let mut policy = ControlKeyPolicy::new();
    let mut buffer = TextBuffer::new();
    let modified = KeyEvent {
        code: KeyCode::Character('c'),
        modifiers: KeyModifiers::CONTROL.union(KeyModifiers::ALT),
        action: KeyAction::Press,
        state: KeyState::NONE,
    };

    assert_eq!(
        policy.handle(modified, false, &mut buffer, NOW),
        ControlEffect::Unhandled
    );
}
