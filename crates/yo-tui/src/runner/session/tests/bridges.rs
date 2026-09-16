use super::*;

// TuiSession facade는 provider가 발급받은 token을 state slot에 그대로 전달하고,
// close 뒤 같은 token의 refresh를 stale로 거절한다.
#[test]
fn session_facade_preserves_overlay_token_scope() {
    let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    let token = session.open_prompt_overlay(panel("First")).unwrap();

    session
        .refresh_prompt_overlay(token, panel("Updated"))
        .unwrap();
    session.close_prompt_overlay(token).unwrap();
    assert_eq!(session.take_prompt_overlay_acceptance(), None);

    assert_eq!(
        session.refresh_prompt_overlay(token, panel("Late")),
        Err(SlotError::StaleToken)
    );
}
