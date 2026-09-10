use std::{path::PathBuf, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use yo_core::{
    PreparedImageAttachment, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind,
};

use super::*;
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    runner::AgentAction,
};

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn prepared(source_bytes: u64) -> PreparedImageAttachment {
    let png = STANDARD.decode(PNG).unwrap();
    let snapshot = InputImageSnapshot::new(1, 1, png.clone()).unwrap();
    PreparedImageAttachment::new(
        InputImage::new(0..7, source_bytes, snapshot).unwrap(),
        png,
        1,
        1,
    )
    .unwrap()
}
fn key(code: KeyCode) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}
fn start(state: &mut TuiState, text: &str) -> ImagePreparationRequest {
    state.enable_image_preparation();
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::PrepareImage(request) =
        state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap()
    else {
        panic!("explicit attachment starts preparation only");
    };
    request
}
fn finish(state: &mut TuiState, request: ImagePreparationRequest, source_bytes: u64) {
    assert!(
        state
            .observe_image_preparation(ImagePreparationUpdate {
                id: request.id,
                revision: request.revision,
                result: Ok(prepared(source_bytes))
            })
            .unwrap()
    );
}

// 명시적인 마지막 attach 행만 이미지로 바꾸며 제출과 거절은 원본 바이트 근거를 유지한다.
#[test]
fn explicit_attachment_keeps_text_and_rejected_submission_exact() {
    let mut state = TuiState::new();
    let request = start(&mut state, "describe this\n/attach local image.jpg");
    assert_eq!(
        state.editor.text(),
        "describe this\n/attach local image.jpg"
    );
    assert_eq!(
        request.source,
        ImagePreparationSource::File(PathBuf::from("local image.jpg"))
    );
    finish(&mut state, request, 321);
    assert_eq!(state.editor.text(), "describe this\n[image]");
    let StateEffect::Dispatch(AgentAction::Submit(submission)) =
        state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap()
    else {
        panic!("ready image submits structured input");
    };
    assert_eq!(submission.input().images()[0].span(), &(14..21));
    assert_eq!(submission.input().images()[0].source_byte_length(), 321);
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::Incompatible,
                "unsupported model",
            ),
        })
        .unwrap();
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        *submission.input()
    );
}

// 편집했다가 같은 텍스트로 되돌려도 이전 revision의 완료 결과는 게시하지 않는다.
#[test]
fn changed_and_restored_text_still_invalidates_late_preparation() {
    let mut state = TuiState::new();
    let request = start(&mut state, "/attach local.png");
    state
        .handle(InputEvent::Paste("x".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Backspace), Duration::ZERO)
        .unwrap();
    assert_eq!(state.editor.text(), "/attach local.png");
    assert!(!state.image_preparation_is_current());
    assert!(
        !state
            .observe_image_preparation(ImagePreparationUpdate {
                id: request.id,
                revision: request.revision,
                result: Ok(prepared(512))
            })
            .unwrap()
    );
    assert!(state.prompt_assist.image_occurrences().is_empty());
}

// 준비 실패와 원본 총량 초과 모두 명령 행과 앞서 준비한 이미지들을 보존한다.
#[test]
fn preparation_failure_and_source_aggregate_excess_preserve_draft() {
    let mut state = TuiState::new();
    let request = start(&mut state, "/attach missing.png");
    state
        .observe_image_preparation(ImagePreparationUpdate {
            id: request.id,
            revision: request.revision,
            result: Err(SubmissionRejection::new(
                SubmissionRejectionKind::InvalidReference,
                "missing",
            )),
        })
        .unwrap();
    assert_eq!(state.editor.text(), "/attach missing.png");
    state.clear_editor();
    for index in 0..2 {
        let request = start(
            &mut state,
            if index == 0 {
                "/attach one.jpg"
            } else {
                "\n/attach two.jpg"
            },
        );
        finish(&mut state, request, InputImage::MAX_SOURCE_BYTES);
    }
    let request = start(&mut state, "\n/attach third.jpg");
    let original = state.prompt_assist.input(state.editor.text()).unwrap();
    finish(&mut state, request, 1);
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        original
    );
    assert_eq!(state.prompt_assist.image_occurrences().len(), 2);
}

// 대기열 이동과 다시 편집하기는 동일한 이미지·원본 크기·미리보기를 유지한다.
#[test]
fn queue_and_recall_preserve_full_occurrence_and_thumbnail() {
    let mut state = TuiState::new();
    let request = start(&mut state, "/attach file.png");
    finish(&mut state, request, 777);
    let original = state.prompt_assist.input(state.editor.text()).unwrap();
    state.queue_follow_up().unwrap();
    assert_eq!(state.follow_ups.front(), Some(&original));
    assert!(state.editor.text().is_empty());
    state.recall_follow_up().unwrap();
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        original
    );
    assert!(state.prompt_assist.image_thumbnail().is_some());
}

// 64 MiB 소유 한도는 작업 전에 최악의 9 MiB를 예약하고 첫 초과 바이트를 거부한다.
#[test]
fn aggregate_ownership_reservation_rejects_first_excess_and_overflow() {
    let last = MAX_OWNED_PNG_BYTES - InputImageSnapshot::MAX_BYTES;
    assert!(can_reserve_image(last));
    assert!(!can_reserve_image(last + 1));
    assert!(!can_reserve_image(usize::MAX));
}

// 리터럴 마커와 Markdown 경로는 입력 이미지를 만들지 않는다.
#[test]
fn image_looking_text_is_not_an_attachment() {
    let mut state = TuiState::new();
    state
        .handle(
            InputEvent::Paste("[image] ![photo](local.png)".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) =
        state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap()
    else {
        panic!("literal text remains ordinary input");
    };
    assert!(submission.input().images().is_empty());
}

// 프롬프트 프레임은 준비된 썸네일 PNG만 배치하고 전체 원본을 다시 디코딩하지 않는다.
#[test]
fn ready_thumbnail_is_placed_directly_in_prompt_frame() {
    let mut state = TuiState::new();
    let request = start(&mut state, "/attach file.png");
    finish(&mut state, request, 777);
    let frame = state
        .prepare_frame(
            crate::surface::Size::new(40, 24),
            &crate::appearance::AppearanceState::default().pin(),
        )
        .unwrap();
    assert_eq!(frame.surface.rasters.len(), 1);
    assert_eq!(
        frame.surface.rasters[0].png.as_ref(),
        STANDARD.decode(PNG).unwrap()
    );
}

// 마커 앞 UTF-8 편집은 범위만 이동하고 마커 내부 편집은 첨부 주석을 제거한다.
#[test]
fn edits_shift_image_spans_and_detach_only_when_marker_is_changed() {
    let mut state = TuiState::new();
    let request = start(&mut state, "/attach file.png");
    finish(&mut state, request, 777);
    for _ in 0..7 {
        state.handle(key(KeyCode::Left), Duration::ZERO).unwrap();
    }
    state
        .handle(InputEvent::Paste("한 ".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(state.prompt_assist.image_occurrences()[0].span(), &(4..11));
    assert_eq!(
        state.prompt_assist.image_occurrences()[0].source_byte_length(),
        777
    );
    state.handle(key(KeyCode::Right), Duration::ZERO).unwrap();
    state.handle(key(KeyCode::Delete), Duration::ZERO).unwrap();
    assert!(state.prompt_assist.image_occurrences().is_empty());
}

// 실제 PNG 주석 뒤에 붙인 터미널 줄바꿈의 attach 행은 제출하지 않고 실패 시 원본을 보존한다.
#[test]
fn trailing_attachment_after_ready_image_never_submits_the_draft() {
    for (separator, trailing) in ["\n", "\r", "\r\n"].into_iter().flat_map(|separator| {
        ["", "\n", "\r", "\r\n", "\r\n\r\n"].map(|trailing| (separator, trailing))
    }) {
        let mut state = TuiState::new();
        let first = start(&mut state, "/attach first.png");
        finish(&mut state, first, 777);
        let first_image = state.prompt_assist.image_occurrences()[0].clone();
        state
            .handle(
                InputEvent::Paste(format!("{separator}/attach /tmp/not-there.png{trailing}")),
                Duration::ZERO,
            )
            .unwrap();
        let draft = state.prompt_assist.input(state.editor.text()).unwrap();
        let StateEffect::PrepareImage(request) =
            state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap()
        else {
            panic!("a final attachment command must not submit an existing image");
        };
        assert_eq!(
            request.source,
            ImagePreparationSource::File(PathBuf::from("/tmp/not-there.png"))
        );
        assert!(state.pending_submissions.is_empty());
        assert!(state.starting_submission.is_none());
        assert!(
            state
                .observe_image_preparation(ImagePreparationUpdate {
                    id: request.id,
                    revision: request.revision,
                    result: Err(SubmissionRejection::new(
                        SubmissionRejectionKind::InvalidReference,
                        "synthetic source is absent",
                    )),
                })
                .unwrap()
        );
        assert_eq!(
            state.prompt_assist.input(state.editor.text()).unwrap(),
            draft
        );
        assert_eq!(state.prompt_assist.image_occurrences(), &[first_image]);
        assert!(state.pending_submissions.is_empty());
        assert!(state.starting_submission.is_none());
        assert!(state.pending_image.is_none());
    }
}

// 일반 문장 뒤 CR·LF·CRLF attach 행도 마지막 행만 준비 명령으로 처리한다.
#[test]
fn trailing_attachment_after_prose_preserves_each_terminal_line_break() {
    for (separator, trailing) in ["\n", "\r", "\r\n"].into_iter().flat_map(|separator| {
        ["", "\n", "\r", "\r\n", "\r\n\r\n"].map(|trailing| (separator, trailing))
    }) {
        let mut state = TuiState::new();
        let text = format!("describe this{separator}/attach local.png{trailing}");
        let request = start(&mut state, &text);
        assert_eq!(state.editor.text(), text);
        assert!(state.pending_submissions.is_empty());
        finish(&mut state, request, 777);
        assert_eq!(
            state.editor.text(),
            format!("describe this{separator}[image]{trailing}")
        );
        assert_eq!(
            state.prompt_assist.image_occurrences()[0].source_byte_length(),
            777
        );
    }
}

// 표시가 승인된 질문의 답안에서도 attach 명령은 준비나 질문 응답으로 전송되지 않는다.
#[test]
fn trailing_attachment_is_blocked_in_a_presented_question_prompt() {
    use std::num::NonZeroU64;

    use yo_core::{
        ActivityId, ActivityKind, ActivityQuestion, ActivityRef, ActivityUpdate, AgentEvent,
        RequestId, TurnId, TurnRef,
    };

    let mut state = TuiState::new();
    state.enable_image_preparation();
    let one = NonZeroU64::new(1).unwrap();
    let turn = TurnRef::new(
        "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
        TurnId::new(one),
    );
    let activity = ActivityRef::new(turn, ActivityId::new(one));
    state
        .observe(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputRequest {
                request_id: RequestId::new(one),
            },
        })
        .unwrap();
    let question = ActivityQuestion {
        allow_notes: false,
        previous_question: false,
        draft: None,
        draft_choice: None,
        plain_text: "Describe the result".to_owned(),
        choices: Vec::new(),
    };
    state
        .observe(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(question.to_snapshot().unwrap()),
        })
        .unwrap();
    for trailing in ["", "\n", "\r", "\r\n", "\r\n\r\n"] {
        state.clear_editor();
        let draft = format!("answer draft\r/attach local.png{trailing}");
        state
            .handle(InputEvent::Paste(draft.clone()), Duration::ZERO)
            .unwrap();
        let frame = state
            .prepare_frame(
                crate::surface::Size::new(80, 24),
                &crate::appearance::AppearanceState::default().pin(),
            )
            .unwrap();
        state.commit_frame(&frame);
        assert_eq!(
            state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap(),
            StateEffect::Redraw
        );
        assert_eq!(state.editor.text(), draft);
        assert!(state.has_pending_request());
        assert!(state.pending_image.is_none());
        assert!(state.pending_submissions.is_empty());
        assert_eq!(
            state
                .handle(clipboard_key(KeyAction::Press), Duration::ZERO)
                .unwrap(),
            StateEffect::Redraw
        );
        assert!(state.pending_image.is_none());
        assert_eq!(state.editor.text(), draft);
        assert_eq!(
            state
                .handle(clipboard_key(KeyAction::Press), Duration::ZERO)
                .unwrap(),
            StateEffect::Redraw
        );
        assert!(state.pending_image.is_none());
        assert_eq!(state.editor.text(), draft);
    }
}

fn clipboard_key(action: KeyAction) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code: KeyCode::Character('v'),
        modifiers: KeyModifiers::CONTROL,
        action,
        state: KeyState::NONE,
    })
}

// 클립보드 이미지는 UTF-8 커서 위치에 준비 후 삽입하며 Enter 전에는 제출하지 않는다.
#[test]
fn clipboard_image_inserts_at_cursor_without_submitting() {
    let mut state = TuiState::new();
    state.enable_image_preparation();
    state
        .handle(InputEvent::Paste("앞뒤".to_owned()), Duration::ZERO)
        .unwrap();
    state.handle(key(KeyCode::Left), Duration::ZERO).unwrap();
    let StateEffect::PrepareImage(request) = state
        .handle(clipboard_key(KeyAction::Press), Duration::ZERO)
        .unwrap()
    else {
        panic!("clipboard gesture prepares an image");
    };
    assert_eq!(request.source, ImagePreparationSource::Clipboard);
    assert_eq!(state.editor.text(), "앞뒤");
    assert!(state.pending_submissions.is_empty());
    assert_eq!(
        state
            .handle(clipboard_key(KeyAction::Repeat), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(
        state
            .handle(clipboard_key(KeyAction::Release), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    finish(&mut state, request, 777);
    assert_eq!(state.editor.text(), "앞[image]뒤");
    assert_eq!(state.prompt_assist.image_occurrences()[0].span(), &(3..10));
    assert!(state.pending_submissions.is_empty());
    let StateEffect::Dispatch(AgentAction::Submit(input)) =
        state.handle(key(KeyCode::Enter), Duration::ZERO).unwrap()
    else {
        panic!("only Enter submits the prepared attachment");
    };
    assert_eq!(input.input().images().len(), 1);
}

// 이미지 마커 내부에서 붙여넣어도 기존 첨부를 깨뜨리지 않고 다음 위치에 추가한다.
#[test]
fn clipboard_image_preserves_an_existing_image_marker() {
    let mut state = TuiState::new();
    let first = start(&mut state, "/attach first.png");
    finish(&mut state, first, 123);
    state.handle(key(KeyCode::Left), Duration::ZERO).unwrap();
    let StateEffect::PrepareImage(request) = state
        .handle(clipboard_key(KeyAction::Press), Duration::ZERO)
        .unwrap()
    else {
        panic!("prepare");
    };
    finish(&mut state, request, 456);
    assert_eq!(state.editor.text(), "[image][image]");
    let images = state.prompt_assist.image_occurrences();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].source_byte_length(), 123);
    assert_eq!(images[1].source_byte_length(), 456);
}

// 클립보드 실패와 일반 텍스트 붙여넣기는 원본 초안·첨부를 유지하며 전송하지 않는다.
#[test]
fn clipboard_failure_keeps_draft_and_text_paste_stays_text() {
    let mut state = TuiState::new();
    let first = start(&mut state, "/attach first.png");
    finish(&mut state, first, 123);
    state
        .handle(InputEvent::Paste(" 설명".to_owned()), Duration::ZERO)
        .unwrap();
    let original = state.prompt_assist.input(state.editor.text()).unwrap();
    let StateEffect::PrepareImage(request) = state
        .handle(clipboard_key(KeyAction::Press), Duration::ZERO)
        .unwrap()
    else {
        panic!("prepare");
    };
    state
        .observe_image_preparation(ImagePreparationUpdate {
            id: request.id,
            revision: request.revision,
            result: Err(SubmissionRejection::new(
                SubmissionRejectionKind::InvalidReference,
                "No clipboard image",
            )),
        })
        .unwrap();
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        original
    );
    assert!(state.pending_submissions.is_empty());
}
