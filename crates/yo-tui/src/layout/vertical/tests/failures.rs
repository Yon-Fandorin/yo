use super::{VerticalLayoutError, VerticalTrack, area, rows, solve_vertical};
use crate::surface::{Point, Rect, Size};

// Exact와 Preferred의 최소 높이가 들어가지 않으면 필요한 높이를 구조적으로 보고한다.
#[test]
fn rejects_height_below_required_minimum() {
    let tracks = [
        VerticalTrack::flexible(),
        VerticalTrack::preferred(rows(3)),
        VerticalTrack::exact(1),
    ];

    let error = solve_vertical(area(1), &tracks).unwrap_err();

    assert_eq!(
        error,
        VerticalLayoutError::InsufficientHeight {
            required: 2,
            available: 1,
        }
    );
}

// 여러 Preferred의 축소 우선순위는 정하지 않았으므로 부분 배치 없이 거절한다.
#[test]
fn rejects_multiple_preferred_tracks() {
    let tracks = [
        VerticalTrack::preferred(rows(2)),
        VerticalTrack::preferred(rows(3)),
    ];

    let error = solve_vertical(area(10), &tracks).unwrap_err();

    assert_eq!(error, VerticalLayoutError::MultiplePreferred);
}

// 여러 Flexible의 분배 정책은 정하지 않았으므로 부분 배치 없이 거절한다.
#[test]
fn rejects_multiple_flexible_tracks() {
    let tracks = [VerticalTrack::flexible(), VerticalTrack::flexible()];

    let error = solve_vertical(area(10), &tracks).unwrap_err();

    assert_eq!(error, VerticalLayoutError::MultipleFlexible);
}

// Exact 높이 합이 u16 범위를 넘으면 wrapping하지 않고 명시적으로 실패한다.
#[test]
fn rejects_exact_height_overflow() {
    let tracks = [VerticalTrack::exact(u16::MAX), VerticalTrack::exact(1)];

    let error = solve_vertical(area(u16::MAX), &tracks).unwrap_err();

    assert_eq!(error, VerticalLayoutError::Overflow);
}

// 입력 영역 자체의 끝 좌표가 범위를 넘으면 어떤 하위 영역도 반환하지 않는다.
#[test]
fn rejects_input_area_overflow() {
    let area = Rect::new(Point::new(0, u16::MAX), Size::new(1, 1));

    let error = solve_vertical(area, &[VerticalTrack::flexible()]).unwrap_err();

    assert_eq!(error, VerticalLayoutError::Overflow);
}

// 수직 solver도 반환할 Rect의 가로 끝이 범위를 넘는 입력 영역은 그대로 거절한다.
#[test]
fn rejects_input_area_width_overflow() {
    let area = Rect::new(Point::new(u16::MAX, 0), Size::new(1, 1));

    let error = solve_vertical(area, &[VerticalTrack::flexible()]).unwrap_err();

    assert_eq!(error, VerticalLayoutError::Overflow);
}
