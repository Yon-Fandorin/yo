use super::{PhysicalEffect, PublicationTransaction, encode};
use crate::{
    surface::{FrameDiff, Point, Size, Surface},
    terminal::{TerminalOp, TerminalOps, mode::inline::InlineFramePlan},
};

// publication transaction의 모든 expected byte는 보존된 typed TerminalOp 목록을
// 공용 ANSI encoder에 다시 넣은 결과와 같고, row 경계도 typed CR/LF로 남는다.
// 따라서 direct ANSI 조립으로 operation/effect ledger를 우회하면 이 검사가 실패한다.
#[test]
fn publication_bytes_are_derived_from_retained_terminal_operations() {
    let publication = Surface::new(Size::new(8, 1)).unwrap();
    let live = Surface::new(Size::new(8, 2)).unwrap();
    let live_operations = TerminalOps::from_diff(&FrameDiff::complete(live.size(), &live));
    let transaction = PublicationTransaction::compile(
        InlineFramePlan::Update {
            current: live.size(),
            previous_cursor: Point::new(0, 1),
            cursor: Point::new(0, 1),
        },
        Size::new(8, 24),
        &publication,
        &live_operations,
    );

    assert!(transaction.operations.iter().all(|operation| {
        !operation.terminal_ops.is_empty() && encode(&operation.terminal_ops) == operation.bytes
    }));
    let row = transaction
        .operations
        .iter()
        .find(|operation| matches!(operation.effect, PhysicalEffect::PublicationRow { row: 0 }))
        .unwrap();
    assert!(matches!(
        row.terminal_ops.as_slice(),
        [TerminalOp::CarriageReturn, .., TerminalOp::LineFeed]
    ));
    assert!(transaction.operations.iter().any(|operation| matches!(
        operation.terminal_ops.as_slice(),
        [TerminalOp::SetCursorVisible(false)]
    )));
}
