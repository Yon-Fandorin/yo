mod bridges;
mod metadata;
mod session_tree;

use yo_core::NoticeLevel;

use super::{
    super::FrameRateLimit,
    TuiSession,
    metadata::{TuiSessionInfo, TuiStatusError, TuiStatusLine},
};
use crate::{
    appearance::{ColorCapability, MotionPreference},
    overlay::{PanelSnapshot, SelectionEntry, SlotError},
};

fn panel(label: &str) -> PanelSnapshot {
    PanelSnapshot::new(
        "Commands",
        vec![SelectionEntry::enabled("entry", label, None)],
    )
    .unwrap()
}

// 시작 시 선택한 frame 제한은 terminal ownership generation이 parts를 다시 빌려도 유지됩니다.
#[test]
fn frame_rate_limit_is_retained_across_generation_borrows() {
    let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard)
        .with_frame_rate_limit(FrameRateLimit::Fps60);

    assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
    assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
}
