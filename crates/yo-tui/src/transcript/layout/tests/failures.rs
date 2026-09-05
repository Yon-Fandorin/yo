use super::{
    TranscriptLayoutConfig, TranscriptRenderError, TranscriptScrollCommand, TranscriptState,
    TranscriptViewState, id, render, styles,
};
use crate::surface::{Point, Rect, Size, Surface};

mod config;
mod rendering;

pub(super) fn assert_failure_preserves_surface_and_state(
    transcript: &TranscriptState,
    size: Size,
    config: &TranscriptLayoutConfig,
    expected: TranscriptRenderError,
) {
    let mut surface = Surface::new(size).unwrap();
    let mut state = TranscriptViewState::default();
    let mut seed = TranscriptState::new();
    seed.push_user(id(99), "0\n1\n2\n3".into())
        .expect("unique seed item");
    {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            &seed,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            None,
        )
        .unwrap();
    }
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            &seed,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            Some(TranscriptScrollCommand::LineUp),
        )
        .unwrap()
    };
    assert!(
        frame.first_visible_row > 0,
        "seed must create a nonzero offset"
    );
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(transcript, &mut view, config, styles(), &mut state, None).unwrap_err()
    };

    assert_eq!(error, expected);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}
