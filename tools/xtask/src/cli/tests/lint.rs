use std::{collections::BTreeMap, fs, path::Path};

fn lint_section(path: &Path, section: &str) -> BTreeMap<String, String> {
    let source = fs::read_to_string(path).expect("lint manifest must be readable");
    let mut selected = false;
    let mut entries = BTreeMap::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            selected = line == section;
            continue;
        }
        if !selected || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        entries.insert(
            key.trim().to_owned(),
            value.split('#').next().unwrap().trim().to_owned(),
        );
    }
    entries
}

// yo-cli는 Unix syscall 경계 때문에 unsafe_code만 별도 완화하므로, 나머지 lint가
// workspace와 계속 일치하는지 검사해 root 설정을 추가할 때 수동 복제 누락을 막습니다.
#[test]
fn yo_cli_lints_mirror_workspace_defaults_except_unsafe_code() {
    let workspace_manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let yo_cli_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/yo-cli/Cargo.toml");

    for (workspace_section, package_section) in [
        ("[workspace.lints.rust]", "[lints.rust]"),
        ("[workspace.lints.clippy]", "[lints.clippy]"),
    ] {
        let mut expected = lint_section(&workspace_manifest, workspace_section);
        let mut actual = lint_section(&yo_cli_manifest, package_section);
        expected.remove("unsafe_code");
        actual.remove("unsafe_code");
        assert_eq!(actual, expected, "yo-cli lint drift in {package_section}");
    }

    assert_eq!(
        lint_section(&workspace_manifest, "[workspace.lints.rust]").get("unsafe_code"),
        Some(&"\"forbid\"".to_owned())
    );
    assert_eq!(
        lint_section(&yo_cli_manifest, "[lints.rust]").get("unsafe_code"),
        Some(&"\"deny\"".to_owned())
    );
}
