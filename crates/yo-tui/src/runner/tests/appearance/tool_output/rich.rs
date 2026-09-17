use super::*;

// 호스트 도구 본문은 실제 코드·표·이미지 프레임으로 표시되며 실패 상태·원문·표시 설정을 보존한다.
#[test]
fn tool_renderer_uses_rich_layout_without_replacing_status_or_source() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityOutcome, Failure};

    use crate::{OutputPreferences, Theme, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(32, 16)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let markdown = format!(
        "```rust\nlet value = 42;\n```\n\n| Key | Value |\n| --- | --- |\n| 상태 | ready |\n\n![Tool image](data:image/png;base64,{})",
        STANDARD.encode(&png)
    );
    for show_images in [true, false] {
        let renderer = ToolRenderer::new({
            let markdown = markdown.clone();
            move |input| {
                assert_eq!(input.kind, ActivityKind::ToolCall);
                assert_eq!(input.source, "docs.search\noriginal payload");
                assert_eq!(input.columns.get(), 30);
                Some(markdown.clone())
            }
        });
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(renderer))
            .with_output_preferences(
                OutputPreferences::default()
                    .with_max_body_width(NonZeroU16::new(30))
                    .with_tool_head_rows(u16::MAX)
                    .with_images(show_images)
                    .with_image_max_width(NonZeroU16::new(4).unwrap()),
            )
            .with_theme(Theme::Light);
        let pin = session.appearance_pin();
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("docs.search\noriginal payload".to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("VISIBLE FAILURE")),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(60, 50), &pin).unwrap();
        let text = visible_rows(&frame.surface);
        for expected in [
            "Tool failed",
            "VISIBLE FAILURE",
            "let value = 42;",
            "ready",
            "Tool image",
        ] {
            assert!(text.contains(expected), "{expected}: {text}");
        }
        assert!(!text.contains("original payload"));
        assert_eq!(text.contains("Image display disabled"), !show_images);
        if show_images {
            assert_eq!(frame.surface.rasters.len(), 1);
            assert_eq!(frame.surface.rasters[0].area.size.width, 4);
            assert_eq!(&*frame.surface.rasters[0].png, png.as_slice());
        } else {
            assert!(frame.surface.rasters.is_empty());
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("original payload"));
        assert!(plain.contains("VISIBLE FAILURE"));
        assert!(!plain.contains("let value = 42;"));
        session = session.with_tool_renderer(None);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 50), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("original payload"));
        assert!(frame.surface.rasters.is_empty());
    }
}
// 적용 거절·높이 초과는 원문으로 돌아가며 새 렌더러 설치는 이미 캐시된 레이아웃을 갱신한다.
#[test]
fn tool_renderer_fallback_and_replacement_invalidate_layout() {
    use crate::ToolRenderer;

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("original payload".to_owned()),
        })
        .unwrap();
    for (renderer, expected) in [
        (
            ToolRenderer::new(|_| Some("replacement one".to_owned())),
            "replacement one",
        ),
        (
            ToolRenderer::new(|_| Some("replacement two".to_owned())),
            "replacement two",
        ),
        (ToolRenderer::new(|_| None), "original payload"),
        (
            ToolRenderer::new(|_| Some("\u{301}".into())),
            "original payload",
        ),
    ] {
        session = session.with_tool_renderer(Some(renderer));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("Tool result"), "{text}");
    }
}
// 도구 확장 콜백은 일반 답변·추론·파일 diff를 가로채지 않는다.
#[test]
fn tool_renderer_does_not_intercept_other_activity_kinds() {
    use crate::ToolRenderer;

    for kind in [
        ActivityKind::AgentMessage,
        ActivityKind::ModelWork,
        ActivityKind::FileChange,
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(|_| {
                panic!("non-tool must not reach renderer")
            })));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("literal payload".to_owned()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("literal payload"));
    }
}
