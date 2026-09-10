//! Image payload and cell placement, independent of terminal protocols.
use std::sync::Arc;

use super::Rect;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RasterImage {
    pub(crate) area: Rect,
    pub(crate) png: Arc<[u8]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{Grapheme, Point, Size, Style, Surface};

    // 이미지 위에 overlay가 쓰면 원본 placement를 제거하되 바깥쪽 입력창 변경은 보존한다.
    #[test]
    fn overlapping_cell_writes_invalidate_only_the_covered_image() {
        let size = Size::new(20, 10);
        let mut surface = Surface::new(size).unwrap();
        surface
            .view(Rect::new(Point::new(0, 0), size))
            .unwrap()
            .place_raster(RasterImage {
                area: Rect::new(Point::new(2, 2), Size::new(8, 4)),
                png: vec![1, 2, 3].into(),
            });
        surface
            .view(Rect::new(Point::new(0, 0), size))
            .unwrap()
            .write(
                Point::new(0, 9),
                Grapheme::try_from("x").unwrap(),
                Style::default(),
            );
        assert_eq!(surface.rasters.len(), 1);
        surface
            .view(Rect::new(Point::new(2, 2), Size::new(2, 1)))
            .unwrap()
            .clear(Style::default());
        assert!(surface.rasters.is_empty());
    }
}
