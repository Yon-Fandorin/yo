use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use super::{JobControl, JobControlOs};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Call {
    Block,
    Unblock,
    InstallDefault,
    RestoreMask,
    Suspend,
    RestoreAction,
}

#[derive(Clone)]
struct RecordingOs {
    calls: Rc<RefCell<Vec<Call>>>,
    failures: BTreeSet<Call>,
}

impl RecordingOs {
    fn new(failures: impl IntoIterator<Item = Call>) -> (Self, Rc<RefCell<Vec<Call>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                calls: Rc::clone(&calls),
                failures: failures.into_iter().collect(),
            },
            calls,
        )
    }

    fn record(&self, call: Call) -> Result<(), String> {
        self.calls.borrow_mut().push(call.clone());
        if self.failures.contains(&call) {
            Err(format!("{call:?} failed"))
        } else {
            Ok(())
        }
    }
}

impl JobControlOs for RecordingOs {
    type Action = &'static str;
    type Mask = &'static str;

    fn block_suspend(&mut self) -> Result<Self::Mask, String> {
        self.record(Call::Block)?;
        Ok("original mask")
    }

    fn install_default(&mut self) -> Result<Self::Action, String> {
        self.record(Call::InstallDefault)?;
        Ok("prior action")
    }

    fn unblock_suspend(&mut self) -> Result<(), String> {
        self.record(Call::Unblock)
    }

    fn restore_mask(&mut self, _mask: &Self::Mask) -> Result<(), String> {
        self.record(Call::RestoreMask)
    }

    fn suspend_self(&mut self) -> Result<(), String> {
        self.record(Call::Suspend)
    }

    fn restore_action(&mut self, _action: &Self::Action) -> Result<(), String> {
        self.record(Call::RestoreAction)
    }
}

// 실제 정지 전에는 SIGTSTP 기본 동작을 설치하고 원래 mask를 복구하며, SIGCONT로 돌아온
// 뒤에는 외부에서 물려받은 handler와 mask를 원래대로 되돌린다.
#[test]
fn suspension_is_transactional_around_the_stopped_interval() {
    let (os, calls) = RecordingOs::new([]);
    let mut control = JobControl { os };

    control.suspend().unwrap();

    assert_eq!(
        *calls.borrow(),
        [
            Call::Block,
            Call::InstallDefault,
            Call::Unblock,
            Call::Suspend,
            Call::Block,
            Call::RestoreAction,
            Call::RestoreMask,
        ]
    );
}

// SIGTSTP 전달이 실패해도 임시 default action과 signal mask를 그대로 남기지 않고 가능한
// 복구를 모두 수행한 뒤 오류를 반환한다.
#[test]
fn failed_suspend_still_restores_inherited_process_state() {
    let (os, calls) = RecordingOs::new([Call::Suspend]);
    let mut control = JobControl { os };

    let error = control.suspend().unwrap_err().to_string();

    assert!(error.contains("sending SIGTSTP"));
    assert_eq!(
        *calls.borrow(),
        [
            Call::Block,
            Call::InstallDefault,
            Call::Unblock,
            Call::Suspend,
            Call::Block,
            Call::RestoreAction,
            Call::RestoreMask,
        ]
    );
}

// 호출자가 원래 SIGTSTP를 차단했더라도 정지 구간에만 해당 signal을 명시적으로
// unblock하므로 raise가 pending 성공으로 끝나지 않고 기본 정지 동작에 도달한다.
#[test]
fn suspension_unblocks_tstp_instead_of_reusing_the_inherited_mask() {
    let (os, calls) = RecordingOs::new([]);
    let mut control = JobControl { os };

    control.suspend().unwrap();

    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| matches!(call, Call::Unblock))
            .count(),
        1
    );
    assert!(
        calls
            .borrow()
            .iter()
            .position(|call| *call == Call::Unblock)
            < calls
                .borrow()
                .iter()
                .position(|call| *call == Call::Suspend)
    );
}

// default action 설치 전에 실패하면 프로세스를 멈추지 않고 처음 차단한 mask만 복원한다.
#[test]
fn failed_default_install_rolls_back_without_suspending() {
    let (os, calls) = RecordingOs::new([Call::InstallDefault]);
    let mut control = JobControl { os };

    assert!(control.suspend().is_err());
    assert_eq!(
        *calls.borrow(),
        [Call::Block, Call::InstallDefault, Call::RestoreMask]
    );
}
