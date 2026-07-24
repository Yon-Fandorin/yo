use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn local_failures_are_aggregated_and_block_global_validation() {
    let report = methexis::check_repository(&fixture("local-invalid"));

    assert!(!report.ok);
    assert!(report.snapshot_revision.is_none());
    assert!(report.units.is_empty());
    assert!(report.diagnostics.len() >= 3);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.phase == methexis::DiagnosticPhase::Local)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_yaml")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_knowledge_id")
    );
}

#[test]
fn global_failures_include_missing_targets_and_cycles() {
    let report = methexis::check_repository(&fixture("global-invalid"));

    assert!(!report.ok);
    assert!(report.snapshot_revision.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.phase == methexis::DiagnosticPhase::Global)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_relation_target")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "required_relation_cycle")
    );
}

#[test]
fn duplicate_knowledge_ids_are_reported_for_each_path() {
    let report = methexis::check_repository(&fixture("duplicate-id"));
    let duplicate_diagnostics = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "duplicate_knowledge_id")
        .collect::<Vec<_>>();

    assert!(!report.ok);
    assert_eq!(duplicate_diagnostics.len(), 2);
    assert!(
        duplicate_diagnostics
            .iter()
            .all(|diagnostic| diagnostic.affected_ids == ["tui.duplicate"])
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "required_relation_cycle"),
        "an unrelated unambiguous cycle must still be reported"
    );
}

#[test]
fn repeated_checks_and_physical_relocation_preserve_identity() {
    let first = methexis::check_repository(&fixture("relocation-a"));
    let repeated = methexis::check_repository(&fixture("relocation-a"));
    let relocated = methexis::check_repository(&fixture("relocation-b"));

    assert!(first.ok);
    assert_eq!(first, repeated);
    assert_eq!(first.snapshot_revision, relocated.snapshot_revision);
    assert_eq!(first.units[0].revision, relocated.units[0].revision);
    assert_ne!(first.units[0].path, relocated.units[0].path);
}

#[cfg(unix)]
#[test]
fn authority_root_symlinks_are_rejected_without_following_them() {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "methexis-root-symlink-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&repository).expect("create temporary repository");
    symlink(
        fixture("relocation-a").join("methexis"),
        repository.join("methexis"),
    )
    .expect("create authority-root symlink");

    let report = methexis::check_repository(&repository);

    fs::remove_dir_all(&repository).expect("remove temporary repository");
    assert!(!report.ok);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "symlink_forbidden");
}
