use std::io::{self, Write};

use super::super::{InlineRecovery, InlineRenderError, InlineRenderer};
use crate::{
    surface::{Grapheme, Point, Rect, Size, Style, Surface},
    terminal::mode::inline::InlineViewport,
};

const TERMINAL_SIZE: Size = Size::new(12, 24);

// 두 persistent 행과 compact live frame을 한 plan으로 끝내면 receipt는 publication 전체를
// 확정하고 각 행은 live viewport를 확보하기 전에 정확히 한 번 terminal stream에 들어간다.
#[test]
fn complete_transaction_publishes_each_row_once_before_the_live_frame() {
    let publication = publication_surface(&["first", "second"]);
    let live = Surface::new(Size::new(12, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport
        .begin_frame_at(live.size(), Point::new(1, 1))
        .unwrap();
    let mut renderer = InlineRenderer::new(FaultWriter::default());

    let receipt = renderer
        .render(pending, None, &live, Some(&publication), TERMINAL_SIZE)
        .unwrap();
    let output = renderer.into_inner().bytes;

    assert!(receipt.publication_complete);
    assert_eq!(receipt.recovery, None);
    assert_eq!(occurrences(&output, b"first"), 1);
    assert_eq!(occurrences(&output, b"second"), 1);
    assert!(position(&output, b"second") < position(&output, b"\r\n\n\x1b[2A"));
}

// effect ledger의 cursor 범위는 준비 때 관찰한 complete terminal geometry에 묶여야 한다.
// live Surface가 그 geometry 밖이면 byte를 쓰기 전에 typed frame 오류로 거부한다.
#[test]
fn publication_rejects_a_live_frame_outside_observed_terminal_geometry() {
    let publication = publication_surface(&["only"]);
    let live = Surface::new(Size::new(12, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::default());

    let error = renderer
        .render(pending, None, &live, Some(&publication), Size::new(12, 1))
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Frame(_)));
    assert!(renderer.into_inner().bytes.is_empty());
}

// 첫 persistent operation 전에 0-byte write 실패가 나면 이미 정리한 owned footprint를
// 유지한 채 complete publication plan을 첫 persistent operation부터 한 번 재시작하고,
// semantic 행 자체는 중복하지 않는다.
#[test]
fn zero_byte_failure_before_publication_uses_one_reversible_restart() {
    let publication = publication_surface(&["only"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([1]));

    let receipt = renderer
        .render(pending, None, &live, Some(&publication), TERMINAL_SIZE)
        .unwrap();
    let output = renderer.into_inner().bytes;

    assert_eq!(receipt.recovery, Some(InlineRecovery::ReversibleRestart));
    assert_eq!(occurrences(&output, b"only"), 1);
}

// 기존 prompt caret에서 owned rows를 모두 지워 clean top에 도달한 뒤 첫 persistent
// operation이 0-byte로 실패해도, cleanup cursor sequence를 잘못 재생하지 않고 그 clean
// footprint에서 publication만 재시작한다.
#[test]
fn reversible_restart_does_not_repeat_completed_owned_row_cleanup() {
    let publication = publication_surface(&["clean"]);
    let size = Size::new(12, 3);
    let previous = Surface::new(size).unwrap();
    let live = Surface::new(size).unwrap();
    let mut viewport = InlineViewport::default();
    viewport
        .begin_frame_at(size, Point::new(2, 1))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame_at(size, Point::new(2, 1)).unwrap();
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([2]));

    let receipt = renderer
        .render(
            pending,
            Some(&previous),
            &live,
            Some(&publication),
            TERMINAL_SIZE,
        )
        .unwrap();
    let output = renderer.into_inner().bytes;

    assert_eq!(receipt.recovery, Some(InlineRecovery::ReversibleRestart));
    assert_eq!(occurrences(&output, b"\x1b[2K"), 3);
    assert_eq!(occurrences(&output, b"clean"), 1);
}

// terminal 높이와 remembered anchor가 세 persistent 행 뒤의 실제 scroll을 확정한
// operation 경계에서 다음 write가 0-byte로 실패하면, native history prefix를 지우거나
// 재생하지 않고 남은 operation suffix만 이어 쓴다.
#[test]
fn operation_boundary_failure_preserves_prefix_and_resumes_suffix() {
    let publication = publication_surface(&["first", "second", "third", "fourth"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let previous = live.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(live.size()).commit();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([5]));

    let receipt = renderer
        .render(
            pending,
            Some(&previous),
            &live,
            Some(&publication),
            Size::new(12, 3),
        )
        .unwrap();
    let output = renderer.into_inner().bytes;

    assert_eq!(receipt.recovery, Some(InlineRecovery::IrreversibleResume));
    assert_eq!(occurrences(&output, b"first"), 1);
    assert_eq!(occurrences(&output, b"second"), 1);
    assert_eq!(occurrences(&output, b"third"), 1);
    assert_eq!(occurrences(&output, b"fourth"), 1);
}

// remembered live footprint 안에서 끝난 첫 persistent 행은 아직 addressable하므로 다음
// operation의 0-byte 실패 때 그 행을 지우고 persistent plan 전체를 clean top에서 다시
// 시작한다. raw stream의 첫 행은 두 번 쓰였지만 앞선 사본은 restart 전에 지워진다.
#[test]
fn addressable_prefix_is_cleared_before_reversible_publication_restart() {
    let publication = publication_surface(&["first", "second"]);
    let live = Surface::new(Size::new(12, 3)).unwrap();
    let previous = live.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(live.size()).commit();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([3]));

    let receipt = renderer
        .render(
            pending,
            Some(&previous),
            &live,
            Some(&publication),
            TERMINAL_SIZE,
        )
        .unwrap();
    let output = renderer.into_inner().bytes;

    assert_eq!(receipt.recovery, Some(InlineRecovery::ReversibleRestart));
    assert_eq!(occurrences(&output, b"first"), 2);
    assert_eq!(occurrences(&output, b"second"), 1);
    assert_eq!(occurrences(&output, b"\x1b[2K"), 4);
}

// cursor range가 terminal bottom을 포함하지만 아직 모든 가능한 anchor에서 scroll했다고
// 확정할 수 없는 prefix는 addressable/scrollback 중 하나로 추측하지 않는다. 다음 0-byte
// 실패는 재생 없이 fatal이 되어 semantic publication ownership을 확정하지 않는다.
#[test]
fn possible_scroll_without_exact_anchor_refuses_recovery() {
    let publication = publication_surface(&["first", "second", "third"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let previous = live.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(live.size()).commit();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([4]));

    let error = renderer
        .render(
            pending,
            Some(&previous),
            &live,
            Some(&publication),
            Size::new(12, 3),
        )
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(!error.to_string().contains("reconciliation also failed"));
    assert_eq!(occurrences(&renderer.into_inner().bytes, b"\x1b[?25h"), 0);
    assert!(matches!(
        viewport.begin_frame(live.size()).plan(),
        crate::terminal::mode::inline::InlineFramePlan::Reanchor { .. }
    ));
}

// self-delimiting operation 한가운데 일부 byte가 admission된 뒤 실패하면 parser/effect
// 경계를 추측하지 않고 bounded recovery를 중단하며 viewport 소유권도 확정하지 않는다.
#[test]
fn partial_operation_failure_is_fatal_and_leaves_live_ownership_untrusted() {
    let publication = publication_surface(&["partial"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::partial_write_at(1, 4));

    let error = renderer
        .render(pending, None, &live, Some(&publication), TERMINAL_SIZE)
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert_eq!(occurrences(&renderer.into_inner().bytes, b"\x1b[?25h"), 0);
    assert!(matches!(
        viewport.begin_frame(live.size()).plan(),
        crate::terminal::mode::inline::InlineFramePlan::Reanchor { .. }
    ));
}

// unbuffered transport의 flush는 숨은 byte를 옮기지 않으므로 한 번 실패해도 byte suffix를
// 재생하지 않고 flush 자체만 한 번 재시도해 exact publication을 확정한다.
#[test]
fn flush_failure_retries_only_flush_without_replaying_bytes() {
    let publication = publication_surface(&["flushed"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter {
        flush_failures: 1,
        ..FaultWriter::default()
    });

    let receipt = renderer
        .render(pending, None, &live, Some(&publication), TERMINAL_SIZE)
        .unwrap();
    let output = renderer.into_inner();

    assert_eq!(receipt.recovery, Some(InlineRecovery::FlushRetry));
    assert_eq!(output.flushes, 2);
    assert_eq!(occurrences(&output.bytes, b"flushed"), 1);
}

// suffix-only recovery도 다시 실패하면 첫 output 오류를 primary로 유지하고 reconciliation
// 실패를 추가 진단으로 붙인 채 더 이상 세 번째 시도를 하지 않는다.
#[test]
fn second_failure_stops_recovery_and_reports_both_attempts() {
    let publication = publication_surface(&["first", "second", "third", "fourth"]);
    let live = Surface::new(Size::new(12, 1)).unwrap();
    let previous = live.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(live.size()).commit();
    let pending = viewport.begin_frame(live.size());
    let mut renderer = InlineRenderer::new(FaultWriter::failing_writes([5, 6]));

    let error = renderer
        .render(
            pending,
            Some(&previous),
            &live,
            Some(&publication),
            Size::new(12, 3),
        )
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(error.to_string().contains("reconciliation also failed"));
    assert_eq!(occurrences(&renderer.into_inner().bytes, b"\x1b[?25h"), 0);
}

fn publication_surface(rows: &[&str]) -> Surface {
    let size = Size::new(12, u16::try_from(rows.len()).unwrap());
    let mut surface = Surface::new(size).unwrap();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    for (row, text) in rows.iter().enumerate() {
        for (column, character) in text.chars().enumerate() {
            view.write(
                Point::new(u16::try_from(column).unwrap(), u16::try_from(row).unwrap()),
                Grapheme::try_from(character.to_string().as_str()).unwrap(),
                Style::default(),
            );
        }
    }
    surface
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("the expected operation bytes are present")
}

#[derive(Default)]
struct FaultWriter {
    bytes: Vec<u8>,
    write_calls: usize,
    failing_writes: Vec<usize>,
    partial: Option<PartialWrite>,
    fail_next: bool,
    flushes: usize,
    flush_failures: usize,
}

struct PartialWrite {
    call: usize,
    admitted: usize,
}

impl FaultWriter {
    fn failing_writes(calls: impl IntoIterator<Item = usize>) -> Self {
        Self {
            failing_writes: calls.into_iter().collect(),
            ..Self::default()
        }
    }

    fn partial_write_at(call: usize, admitted: usize) -> Self {
        Self {
            partial: Some(PartialWrite { call, admitted }),
            ..Self::default()
        }
    }
}

impl Write for FaultWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let call = self.write_calls;
        self.write_calls += 1;
        if self.fail_next || self.failing_writes.contains(&call) {
            self.fail_next = false;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("write {call}"),
            ));
        }
        if let Some(partial) = self.partial.as_ref()
            && partial.call == call
        {
            let written = partial.admitted.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..written]);
            self.fail_next = true;
            return Ok(written);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.flush_failures > 0 {
            self.flush_failures -= 1;
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush"))
        } else {
            Ok(())
        }
    }
}
