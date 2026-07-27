use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

use super::{TERMINATION_SIGNALS, TerminationSignal, TerminationSignals};

// production receiver는 지원 신호를 등록하고 사용하지 않은 상태에서 안전하게 해제한다.
#[test]
fn registers_the_production_signal_receiver() {
    let _signals = TerminationSignals::register().expect("termination signal registration");
}

// SSH 연결 종료를 포함한 네 가지 종료 신호만 typed 종료 이벤트로 해석한다.
#[test]
fn maps_supported_termination_signals_without_aliasing() {
    assert_eq!(TERMINATION_SIGNALS, [SIGHUP, SIGINT, SIGQUIT, SIGTERM]);
    assert_eq!(
        TerminationSignal::from_raw(SIGHUP),
        Some(TerminationSignal::Hangup)
    );
    assert_eq!(
        TerminationSignal::from_raw(SIGINT),
        Some(TerminationSignal::Interrupt)
    );
    assert_eq!(
        TerminationSignal::from_raw(SIGQUIT),
        Some(TerminationSignal::Quit)
    );
    assert_eq!(
        TerminationSignal::from_raw(SIGTERM),
        Some(TerminationSignal::Terminate)
    );
    assert_eq!(TerminationSignal::from_raw(0), None);
}

// 복구 뒤 기본 disposition을 재현할 때 원래 신호 번호를 그대로 사용한다.
#[test]
fn preserves_raw_signal_identity_for_default_disposition() {
    for (signal, raw) in [
        (TerminationSignal::Hangup, SIGHUP),
        (TerminationSignal::Interrupt, SIGINT),
        (TerminationSignal::Quit, SIGQUIT),
        (TerminationSignal::Terminate, SIGTERM),
    ] {
        assert_eq!(signal.as_raw(), raw);
    }
}
