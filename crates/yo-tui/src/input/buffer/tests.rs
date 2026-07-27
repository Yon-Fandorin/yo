use super::TextBuffer;

// ASCII와 한글을 넣으면 입력 순서와 커서 위치를 그대로 보존한다.
#[test]
fn inserts_text_at_the_cursor() {
    let mut buffer = TextBuffer::new();

    assert!(buffer.insert("A가"));

    assert_eq!(buffer.as_str(), "A가");
    assert_eq!(buffer.cursor_byte_index(), "A가".len());
    assert!(!buffer.is_empty());
}

// 한글처럼 화면에서 넓은 문자도 한 글자 단위로 이동한다.
#[test]
fn moves_across_grapheme_clusters() {
    let mut buffer = TextBuffer::new();
    buffer.insert("A가B");

    assert!(buffer.move_left());
    assert_eq!(buffer.cursor_byte_index(), "A가".len());
    assert!(buffer.move_left());
    assert_eq!(buffer.cursor_byte_index(), "A".len());
    assert!(buffer.move_right());
    assert_eq!(buffer.cursor_byte_index(), "A가".len());
}

// 결합 문자와 이모지 시퀀스를 구성하는 여러 코드 포인트는 한 글자로 이동한다.
#[test]
fn treats_combining_and_emoji_sequences_as_single_units() {
    let mut buffer = TextBuffer::new();
    buffer.insert("e\u{301}👨‍👩‍👧");

    assert!(buffer.move_left());
    assert_eq!(buffer.cursor_byte_index(), "e\u{301}".len());
    assert!(buffer.move_left());
    assert_eq!(buffer.cursor_byte_index(), 0);
}

// 넓은 한글을 지운 뒤 ASCII를 넣어도 숨은 공백 없이 문자열이 이어진다.
#[test]
fn replaces_a_wide_grapheme_without_leaving_padding() {
    let mut buffer = TextBuffer::new();
    buffer.insert("가B");

    assert!(buffer.move_left());
    assert!(buffer.delete_backward());
    assert!(buffer.insert("A"));

    assert_eq!(buffer.as_str(), "AB");
    assert_eq!(buffer.cursor_byte_index(), 1);
}

// Delete는 커서 뒤의 결합 문자 전체만 제거한다.
#[test]
fn deletes_the_next_grapheme_cluster() {
    let mut buffer = TextBuffer::new();
    buffer.insert("Ae\u{301}B");
    buffer.move_left();
    buffer.move_left();

    assert!(buffer.delete_forward());

    assert_eq!(buffer.as_str(), "AB");
    assert_eq!(buffer.cursor_byte_index(), 1);
}

// 문자열 경계의 이동과 삭제 및 빈 입력은 상태를 바꾸지 않는다.
#[test]
fn boundary_operations_are_no_ops() {
    let mut buffer = TextBuffer::new();

    assert!(!buffer.insert(""));
    assert!(!buffer.move_left());
    assert!(!buffer.move_right());
    assert!(!buffer.delete_backward());
    assert!(!buffer.delete_forward());
    assert!(buffer.is_empty());
}

// 새 입력이 이웃 문자와 하나의 grapheme으로 합쳐지면 커서는 합쳐진 글자 뒤로 간다.
#[test]
fn insertion_normalizes_the_cursor_to_a_grapheme_boundary() {
    let mut buffer = TextBuffer::new();
    buffer.insert("a");

    assert!(buffer.insert("\u{301}"));

    assert_eq!(buffer.as_str(), "a\u{301}");
    assert_eq!(buffer.cursor_byte_index(), "a\u{301}".len());
    assert!(buffer.move_left());
    assert_eq!(buffer.cursor_byte_index(), 0);
}

// Backspace로 경계 문자를 지워 양옆이 합쳐져도 커서는 새 grapheme 안에 남지 않는다.
#[test]
fn backward_deletion_normalizes_a_newly_merged_grapheme() {
    let mut buffer = TextBuffer::new();
    buffer.insert("a\0\u{301}");
    buffer.move_left();

    assert!(buffer.delete_backward());

    assert_eq!(buffer.as_str(), "a\u{301}");
    assert_eq!(buffer.cursor_byte_index(), "a\u{301}".len());
    assert!(buffer.delete_backward());
    assert!(buffer.is_empty());
}

// Delete로 경계 문자를 지워 양옆이 합쳐져도 커서는 새 grapheme 안에 남지 않는다.
#[test]
fn forward_deletion_normalizes_a_newly_merged_grapheme() {
    let mut buffer = TextBuffer::new();
    buffer.insert("a\0\u{301}");
    buffer.move_left();
    buffer.move_left();

    assert!(buffer.delete_forward());

    assert_eq!(buffer.as_str(), "a\u{301}");
    assert_eq!(buffer.cursor_byte_index(), "a\u{301}".len());
    assert!(buffer.delete_backward());
    assert!(buffer.is_empty());
}

// 제출할 문자열은 복사하지 않고 꺼내며 버퍼와 커서는 초기 상태로 돌아간다.
#[test]
fn takes_owned_text_and_resets_the_buffer() {
    let mut buffer = TextBuffer::new();
    buffer.insert("질문");

    assert_eq!(buffer.take().as_deref(), Some("질문"));
    assert!(buffer.is_empty());
    assert_eq!(buffer.cursor_byte_index(), 0);
    assert_eq!(buffer.take(), None);
}
