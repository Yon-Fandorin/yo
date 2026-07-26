use super::super::{GeometryError, Point, Rect, Size, Surface};

// 유효한 사각형만 SurfaceView 경계로 만들 수 있는지 확인한다.
#[test]
fn view_rejects_overflow_and_out_of_bounds_geometry() {
    let mut surface = Surface::new(Size::new(10, 4)).unwrap();

    assert!(
        surface
            .view(Rect::new(Point::new(2, 1), Size::new(8, 3)))
            .is_ok()
    );
    assert!(matches!(
        surface.view(Rect::new(Point::new(3, 1), Size::new(8, 3))),
        Err(GeometryError::OutOfBounds)
    ));
    assert!(matches!(
        surface.view(Rect::new(Point::new(u16::MAX, 0), Size::new(2, 1))),
        Err(GeometryError::Overflow)
    ));
}

// Surface 크기와 셀 조회가 width·height의 물리 경계를 그대로 따르는지 확인한다.
#[test]
fn surface_reports_size_and_bounds_cell_reads() {
    let surface = Surface::new(Size::new(3, 2)).unwrap();

    assert_eq!(surface.size(), Size::new(3, 2));
    assert!(surface.cell(Point::new(2, 1)).is_some());
    assert!(surface.cell(Point::new(3, 1)).is_none());
    assert!(surface.cell(Point::new(2, 2)).is_none());
}
