use super::{RustixTermiosDriver, TermiosDriver, TtyStateAdapter};

#[derive(Clone, Debug, Eq, PartialEq)]
struct State {
    value: u8,
    raw: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Call {
    Capture,
    MakeRaw(State),
    Apply(State),
}

struct RecordingDriver {
    captured: State,
    calls: Vec<Call>,
}

impl TermiosDriver for RecordingDriver {
    type State = State;
    type Error = &'static str;

    fn capture(&mut self) -> Result<Self::State, Self::Error> {
        self.calls.push(Call::Capture);
        Ok(self.captured.clone())
    }

    fn make_raw(&mut self, state: &mut Self::State) {
        self.calls.push(Call::MakeRaw(state.clone()));
        state.raw = true;
    }

    fn apply(&mut self, state: &Self::State) -> Result<(), Self::Error> {
        self.calls.push(Call::Apply(state.clone()));
        Ok(())
    }
}

fn adapter() -> TtyStateAdapter<RecordingDriver> {
    TtyStateAdapter::new(RecordingDriver {
        captured: State {
            value: 7,
            raw: false,
        },
        calls: Vec::new(),
    })
}

// raw input 상태는 저장된 원본의 복제본에서 만들며 복구에 쓸 원본 자체는 바꾸지 않는다.
#[test]
fn raw_input_is_derived_without_mutating_the_captured_state() {
    let mut adapter = adapter();
    let original = adapter.capture().unwrap();

    adapter.enable_raw(&original).unwrap();

    assert_eq!(
        original,
        State {
            value: 7,
            raw: false
        }
    );
    assert_eq!(
        adapter.driver.calls,
        [
            Call::Capture,
            Call::MakeRaw(State {
                value: 7,
                raw: false,
            }),
            Call::Apply(State {
                value: 7,
                raw: true,
            }),
        ]
    );
}

// 복구는 raw 상태를 재계산하지 않고 최초에 저장한 TTY 상태를 그대로 적용한다.
#[test]
fn restoration_applies_the_exact_captured_state() {
    let mut adapter = adapter();
    let original = adapter.capture().unwrap();
    adapter.enable_raw(&original).unwrap();

    adapter.restore(&original).unwrap();

    assert_eq!(
        adapter.driver.calls.last(),
        Some(&Call::Apply(State {
            value: 7,
            raw: false,
        }))
    );
}

// 실제 Unix 구현은 안전한 Rustix stdio 경계에서 stdin을 빌릴 수 있어야 한다.
#[test]
fn rustix_driver_can_bind_process_stdin_without_a_syscall() {
    let _driver = RustixTermiosDriver::stdin();
}
