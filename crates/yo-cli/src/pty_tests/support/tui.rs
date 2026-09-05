use std::error::Error;

use yo_tui::{PresentationMode, TerminationSource};

use super::{ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN, agent::PendingAgent};

pub(in crate::pty_tests) fn assert_fullscreen_pair(output: &[u8]) {
    let enter = output
        .windows(ENTER_ALTERNATE_SCREEN.len())
        .position(|candidate| candidate == ENTER_ALTERNATE_SCREEN)
        .expect("fullscreen output must enter the alternate screen");
    let leave = output
        .windows(LEAVE_ALTERNATE_SCREEN.len())
        .position(|candidate| candidate == LEAVE_ALTERNATE_SCREEN)
        .expect("fullscreen output must leave the alternate screen");

    assert!(enter < leave, "alternate-screen cleanup must follow entry");
    assert_eq!(
        output
            .windows(ENTER_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == ENTER_ALTERNATE_SCREEN)
            .count(),
        1
    );
    assert_eq!(
        output
            .windows(LEAVE_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == LEAVE_ALTERNATE_SCREEN)
            .count(),
        1
    );
}

pub(in crate::pty_tests) fn run_fullscreen(
    termination: &mut impl TerminationSource,
) -> Result<(), Box<dyn Error>> {
    let mut agent = PendingAgent;
    yo_tui::run_with_mode(
        termination,
        &mut agent,
        PresentationMode::Fullscreen,
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    )?;
    Ok(())
}
