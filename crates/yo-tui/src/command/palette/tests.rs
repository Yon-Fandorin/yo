use super::command_query;

// draft 전체의 첫 slash token만 ASCII case를 정규화한 query가 되고, 앞쪽 공백은
// command 소유권을 바꾸지 않는다.
#[test]
fn first_slash_token_produces_a_normalized_query() {
    assert_eq!(command_query("/MO", 3).as_deref(), Some("mo"));
    assert_eq!(command_query("  /he", 5).as_deref(), Some("he"));
}

// 일반 문장에 포함된 slash, argument가 붙은 command, cursor 뒤 text가 남은 draft는
// palette query가 아니므로 command controller가 입력을 가로채지 않는다.
#[test]
fn embedded_or_completed_slash_tokens_are_not_commands() {
    assert_eq!(command_query("explain /", 9), None);
    assert_eq!(command_query("/model other", 12), None);
    assert_eq!(command_query("/h keep", 2), None);
}

// terminal control text를 query로 받아도 panel row에 투영하지 않고 안전한 disabled
// no-match snapshot을 만들 수 있어야 한다.
#[test]
fn unsafe_query_text_is_not_projected_into_the_panel() {
    super::panel_snapshot("\u{1b}");
}
