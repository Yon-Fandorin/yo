use std::num::NonZeroU16;

use crate::{
    surface::{CellContent, Color, Point, Rect, Size, Style, Surface},
    transcript::{
        TranscriptItemId, TranscriptLayoutConfig, TranscriptRenderError, TranscriptRenderFrame,
        TranscriptScrollCommand, TranscriptState, TranscriptStyles, TranscriptViewMode,
        TranscriptViewState, render,
    },
};

mod failures;
mod scrolling;

fn id(value: u64) -> TranscriptItemId {
    TranscriptItemId::new(value)
}

fn styles() -> TranscriptStyles {
    TranscriptStyles {
        background: style(0),
        user_marker: style(1),
        user_body: style(2),
        assistant_marker: style(3),
        assistant_body: style(4),
    }
}

fn style(index: u8) -> Style {
    Style {
        foreground: Color::Indexed(index),
        ..Style::default()
    }
}

fn render_into(
    transcript: &TranscriptState,
    size: Size,
    config: &TranscriptLayoutConfig,
    state: &mut TranscriptViewState,
    command: Option<TranscriptScrollCommand>,
) -> (Surface, TranscriptRenderFrame) {
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(transcript, &mut view, config, styles(), state, command).unwrap()
    };
    (surface, frame)
}

fn rendered_row(surface: &Surface, y: u16) -> String {
    (0..surface.size().width)
        .map(
            |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

// 기본 설정은 rib의 마커와 2열 본문 시작점을 쓰되 본문 폭을 임의로 제한하지 않는다.
#[test]
fn defaults_to_rib_markers_without_a_body_width_cap() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "x".repeat(101))
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "done")
        .expect("streaming assistant");
    let size = Size::new(103, 5);
    let mut state = TranscriptViewState::default();

    let (surface, frame) = render_into(
        &transcript,
        size,
        &TranscriptLayoutConfig::default(),
        &mut state,
        None,
    );

    assert_eq!(frame.content_height, 3);
    assert_eq!(rendered_row(&surface, 0), format!("❯ {}", "x".repeat(101)));
    assert_eq!(rendered_row(&surface, 1), "");
    assert_eq!(rendered_row(&surface, 2), "⏺ done");
    assert_eq!(
        surface.cell(Point::new(0, 0)).unwrap().style(),
        styles().user_marker
    );
    assert_eq!(
        surface.cell(Point::new(2, 0)).unwrap().style(),
        styles().user_body
    );
    assert_eq!(
        surface.cell(Point::new(0, 2)).unwrap().style(),
        styles().assistant_marker
    );
}

// 마커, 본문 시작 열과 최대 본문 폭을 바꾸면 같은 transcript가 해당 정책으로 다시 배치된다.
#[test]
fn applies_custom_markers_indent_and_optional_body_width() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "abcdef".into())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "xy")
        .expect("streaming assistant");
    let config = TranscriptLayoutConfig::default()
        .with_max_body_width(NonZeroU16::new(3))
        .with_body_indent(4)
        .with_user_marker("U>")
        .with_assistant_marker("A>");
    let mut state = TranscriptViewState::default();

    let (surface, frame) = render_into(&transcript, Size::new(10, 6), &config, &mut state, None);

    assert_eq!(frame.content_height, 4);
    assert_eq!(rendered_row(&surface, 0), "U>  abc");
    assert_eq!(rendered_row(&surface, 1), "    def");
    assert_eq!(rendered_row(&surface, 2), "");
    assert_eq!(rendered_row(&surface, 3), "A>  xy");
}

// 새 사용자 턴은 두 빈 행으로 분리하고 빈 streaming 항목은 stray separator를 만들지 않는다.
#[test]
fn separates_turns_and_skips_empty_streaming_items() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "first".into())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .push_user(id(3), "second".into())
        .expect("unique user item");
    let mut state = TranscriptViewState::default();

    let (surface, frame) = render_into(
        &transcript,
        Size::new(12, 5),
        &TranscriptLayoutConfig::default(),
        &mut state,
        None,
    );

    assert_eq!(frame.content_height, 4);
    assert_eq!(rendered_row(&surface, 0), "❯ first");
    assert_eq!(rendered_row(&surface, 1), "");
    assert_eq!(rendered_row(&surface, 2), "");
    assert_eq!(rendered_row(&surface, 3), "❯ second");
}

// 빈 확정 메시지는 사라지지 않고 역할 marker 한 행으로 남아 transcript 순서를 보존한다.
#[test]
fn finalized_empty_messages_keep_their_role_markers() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), String::new())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript.finalize(id(2)).expect("streaming assistant");
    let mut state = TranscriptViewState::default();

    let (surface, frame) = render_into(
        &transcript,
        Size::new(5, 3),
        &TranscriptLayoutConfig::default(),
        &mut state,
        None,
    );

    assert_eq!(frame.content_height, 3);
    assert_eq!(rendered_row(&surface, 0), "❯");
    assert_eq!(rendered_row(&surface, 1), "");
    assert_eq!(rendered_row(&surface, 2), "⏺");
}
