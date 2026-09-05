use std::num::NonZeroU16;

use super::{VerticalLayoutError, VerticalTrack, solve_vertical};
use crate::surface::{Point, Rect, Size};

mod failures;

fn area(height: u16) -> Rect {
    Rect::new(Point::new(3, 5), Size::new(80, height))
}

fn rows(rows: u16) -> NonZeroU16 {
    NonZeroU16::new(rows).unwrap()
}

fn heights(layout: &super::VerticalLayout) -> Vec<u16> {
    layout.areas().iter().map(|area| area.size.height).collect()
}

// 일반 화면에서는 Preferred가 희망 높이를 받고 Flexible이 나머지를 채운다.
#[test]
fn allocates_exact_preferred_and_flexible_tracks() {
    let tracks = [
        VerticalTrack::flexible(),
        VerticalTrack::preferred(rows(3)),
        VerticalTrack::exact(1),
    ];

    let layout = solve_vertical(area(10), &tracks).unwrap();

    assert_eq!(heights(&layout), [6, 3, 1]);
    assert_eq!(
        layout.areas(),
        [
            Rect::new(Point::new(3, 5), Size::new(80, 6)),
            Rect::new(Point::new(3, 11), Size::new(80, 3)),
            Rect::new(Point::new(3, 14), Size::new(80, 1)),
        ]
    );
}

// 공간이 부족하면 Flexible을 먼저 0으로 만들고 Preferred를 최소 한 행까지 줄인다.
#[test]
fn shrinks_flexible_before_preferred() {
    let tracks = [
        VerticalTrack::flexible(),
        VerticalTrack::preferred(rows(3)),
        VerticalTrack::exact(1),
    ];

    let layout = solve_vertical(area(2), &tracks).unwrap();

    assert_eq!(heights(&layout), [0, 1, 1]);
    assert_eq!(layout.areas()[1].origin, Point::new(3, 5));
    assert_eq!(layout.areas()[2].origin, Point::new(3, 6));
}

// resize는 이전 결과를 기억하지 않고 같은 현재 입력에 항상 같은 배치를 만든다.
#[test]
fn resize_history_does_not_change_the_result() {
    let tracks = [
        VerticalTrack::flexible(),
        VerticalTrack::preferred(rows(3)),
        VerticalTrack::exact(1),
    ];

    let first = solve_vertical(area(10), &tracks).unwrap();
    let smaller = solve_vertical(area(6), &tracks).unwrap();
    let restored = solve_vertical(area(10), &tracks).unwrap();

    assert_eq!(heights(&smaller), [2, 3, 1]);
    assert_eq!(restored, first);
}

// Flexible이 없으면 요청한 영역만 연속 배치하고 남은 아래 공간은 소유하지 않는다.
#[test]
fn leaves_trailing_space_unallocated_without_flexible_track() {
    let tracks = [VerticalTrack::preferred(rows(2)), VerticalTrack::exact(1)];

    let layout = solve_vertical(area(10), &tracks).unwrap();

    assert_eq!(heights(&layout), [2, 1]);
    assert_eq!(layout.areas()[1].origin, Point::new(3, 7));
}

// 높이 0인 Exact와 Flexible은 빈 영역으로 표현할 수 있다.
#[test]
fn zero_height_tracks_have_stable_empty_regions() {
    let tracks = [VerticalTrack::exact(0), VerticalTrack::flexible()];

    let layout = solve_vertical(area(0), &tracks).unwrap();

    assert_eq!(heights(&layout), [0, 0]);
    assert!(
        layout
            .areas()
            .iter()
            .all(|area| area.origin == Point::new(3, 5))
    );
}
