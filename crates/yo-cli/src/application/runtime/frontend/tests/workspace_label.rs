use std::path::Path;

use super::super::compact_workspace_label_with_home;

// 홈 아래 작업공간은 사용자가 익숙한 `~/...` 표기로 줄이되 경로의 나머지는 보존한다.
#[test]
fn home_workspace_uses_tilde_without_losing_the_relative_path() {
    assert_eq!(
        compact_workspace_label_with_home(
            Path::new("/home/yon/projects/yo"),
            Some(Path::new("/home/yon")),
        ),
        "~/projects/yo"
    );
    assert_eq!(
        compact_workspace_label_with_home(Path::new("/home/yon"), Some(Path::new("/home/yon"))),
        "~"
    );
}

// 홈 밖 경로이거나 홈 정보를 모르는 경우에는 의미가 달라지지 않도록 절대 경로를 유지한다.
#[test]
fn external_workspace_remains_an_absolute_path() {
    assert_eq!(
        compact_workspace_label_with_home(Path::new("/srv/work/yo"), Some(Path::new("/home/yon")),),
        "/srv/work/yo"
    );
    assert_eq!(
        compact_workspace_label_with_home(Path::new("/srv/work/yo"), None),
        "/srv/work/yo"
    );
}
