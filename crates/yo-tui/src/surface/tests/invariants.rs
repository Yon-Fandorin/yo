use super::super::{CellContent, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome};

// 여러 overwrite 뒤에도 모든 leader와 continuation이 서로 완전하게 대응하는지 검사한다.
#[test]
fn mutations_preserve_complete_grapheme_footprints() {
    let size = Size::new(8, 2);
    let mut surface = Surface::new(size).unwrap();
    {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();

        // 넓은 문자, continuation 시작 overwrite, 다시 넓은 문자를 순서대로 섞는다.
        for (point, text) in [
            (Point::new(0, 0), "가"),
            (Point::new(3, 0), "A"),
            (Point::new(1, 0), "B"),
            (Point::new(5, 1), "👩‍💻"),
        ] {
            assert_eq!(
                view.write(point, Grapheme::try_from(text).unwrap(), Style::default()),
                WriteOutcome::Written
            );
        }
    }

    for y in 0..size.height {
        for x in 0..size.width {
            let cell = surface.cell(Point::new(x, y)).unwrap();
            match cell.content() {
                CellContent::Blank => {},
                CellContent::Grapheme { width, .. } => {
                    assert!(
                        x.checked_add(width.get())
                            .is_some_and(|end| end <= size.width)
                    );
                    for back in 1..width.get() {
                        assert!(matches!(
                            surface.cell(Point::new(x + back, y)).unwrap().content(),
                            CellContent::Continuation { back: actual } if actual.get() == back
                        ));
                    }
                },
                CellContent::Continuation { back } => {
                    let leader_x = x
                        .checked_sub(back.get())
                        .expect("continuation has an owner");
                    assert!(matches!(
                        surface
                            .cell(Point::new(leader_x, y))
                            .unwrap()
                            .content(),
                        CellContent::Grapheme { width, .. } if width.get() > back.get()
                    ));
                },
            }
        }
    }
}
