use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "cli/bounded_check.rs"]
mod bounded_check;

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis check [--only <class>[,<class>...]]... [--summary] [--unit <id>]
    methexis check --staged-activation
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis approve <request.json>
    methexis create-checkpoint <request.json>
    methexis propose-activation <request.json>
    methexis refresh-context-manifests <activation-request.json>
    methexis resolve-context <request.json>

COMMANDS:
    check             Validate current SOT integrity or one exact staged activation
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    approve           Record a human-authorized approval proposal
    create-checkpoint Create an immutable trusted-revision Checkpoint proposal
    propose-activation Propose the active Checkpoint with compare-and-swap
    refresh-context-manifests Refresh registered manifests for an activation proposal
    resolve-context    Build or reuse deterministic token-bounded agent context

Run commands from the repository root. Mutations remain Draft proposals until
trusted integration. Check derives approval and active/degraded eligibility
from local develop, then uses current Source observations only to demote it.
",
);

pub(crate) fn methexis() -> Command {
    Command::new(env!("CARGO_BIN_EXE_methexis"))
}

pub(crate) struct CorpusRepository {
    pub(crate) path: PathBuf,
}

impl CorpusRepository {
    pub(crate) fn without_active_checkpoint() -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crate is under <repository>/tools/methexis");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "methexis-cli-corpus-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        copy_directory(&source.join("methexis"), &path.join("methexis"));
        fs::remove_file(path.join("methexis/active-checkpoint.yaml")).unwrap();
        fs::remove_dir_all(path.join("methexis/checkpoints")).unwrap();
        let repository = Self { path };
        repository.git(&[
            "init",
            "--initial-branch=develop",
            "--object-format=sha1",
            "--template=",
        ]);
        repository.git(&["config", "user.email", "fixture@example.invalid"]);
        repository.git(&["config", "user.name", "Methexis Fixture"]);
        repository.git(&["add", "methexis"]);
        repository.git(&["commit", "-m", "repository corpus without activation"]);
        repository
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("/usr/bin/git")
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_GRAFT_FILE", "/dev/null")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("LC_ALL", "C")
            .arg("--no-replace-objects")
            .current_dir(&self.path)
            .env("GIT_AUTHOR_DATE", "2026-07-31T12:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-07-31T12:00:00Z")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for CorpusRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn copy_directory(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}

// --help가 성공 상태로 Source-aware check를 포함한 전체 명령 표면을 정확히 출력하는지 확인한다.
#[test]
fn help_describes_the_source_aware_check_surface() {
    let output = methexis().arg("--help").output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("help is UTF-8"),
        HELP,
    );
}

// 인자 없이 실행해도 --help와 동일한 bootstrap help를 성공 상태로 출력하는지 확인한다.
#[test]
fn no_arguments_uses_the_same_bootstrap_help() {
    let output = methexis().output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("help is UTF-8"),
        HELP,
    );
}

// --version이 패키지 버전을 그대로 포함한 한 줄만 출력하고 stderr를 비우는지 확인한다.
#[test]
fn version_uses_the_package_version() {
    let output = methexis().arg("--version").output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("methexis {}\n", env!("CARGO_PKG_VERSION")),
    );
}

// 지원하지 않는 명령은 종료 코드 2와 구조화된 unsupported_command 오류 JSON을 stderr로 반환한다.
#[test]
fn unsupported_input_is_a_structured_failure() {
    let output = methexis()
        .arg("compile")
        .output()
        .expect("run unsupported methexis command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("error is UTF-8"),
        concat!(
            "{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,",
            "\"error\":{\"code\":\"unsupported_command\",",
            "\"affected_ids\":[],",
            "\"next_actions\":[\"methexis --help\"]}}\n",
        ),
    );
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "injected"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// stdout writer가 BrokenPipe를 반환하면 library가 성공처럼 삼키지 않고 같은 오류 종류를
// main 호출자에게 돌려준다. 그러면 main이 정해진 fallback 처리를 실행할 수 있다.
#[test]
fn stream_failures_are_returned_to_the_binary_boundary() {
    let error = methexis::run(Vec::<OsString>::new(), FailingWriter, Vec::<u8>::new())
        .expect_err("injected writer must fail");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

// active record만 제거한 독립 trusted corpus에서 `methexis check`의 stdout 순서와
// exact approval을 검증한다. 실제 저장소의 activation 전환 상태가 CLI 단위 테스트 답안이 되지 않게
// 한다. 기존 seed와 검수 가능한 14개 Surface Draft도 빠짐없이 포함해야 한다.
#[test]
fn check_reports_the_repository_corpus_on_stdout() {
    let repository = CorpusRepository::without_active_checkpoint();
    let output = methexis()
        .current_dir(&repository.path)
        .arg("check")
        .output()
        .expect("run methexis check");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("check output is JSON");
    assert_eq!(report["schema"], "methexis.check/v1alpha1");
    assert_eq!(report["ok"], true);
    assert_eq!(report["authority"], "draft");
    assert_eq!(
        report["requested_checks"],
        serde_json::json!(["records", "relations", "authority", "artifacts"])
    );
    assert_eq!(report["requested_checks"], report["executed_checks"]);
    let units = report["units"].as_array().expect("units are an array");
    let ids = units
        .iter()
        .map(|unit| unit["id"].as_str().expect("unit has an ID"))
        .collect::<Vec<_>>();

    // agent가 같은 입력을 항상 같은 순서로 읽을 수 있도록 wire 출력 순서를 고정한다.
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    // 새 Surface 계약을 추가해도 이미 활성화된 최초 seed가 빠지면 안 된다.
    for seed in [
        "tui.architecture.evidence-based-split",
        "tui.architecture.module-boundaries",
        "tui.crate.ui-only-boundary",
        "tui.dependencies.selection-gate",
        "tui.runtime.typed-flow",
    ] {
        assert!(ids.contains(&seed), "repository corpus lost seed `{seed}`");
    }

    // Surface corpus는 계속 늘어날 수 있지만, develop에 통합된 14개 단위는
    // exact revision에 맞는 trusted approval을 가진 상태로 모두 노출되어야 한다.
    for (id, revision) in [
        // grapheme 쓰기는 원자적으로 수행하고, 주변 셀의 재배치는 상위 layout에 맡긴다.
        (
            "tui.surface.atomic-grapheme-write",
            "sha256:c1cdcafa4c92e4e590431b36b8afa3cdeace0e2a9b3355bc544bb76580eaac02",
        ),
        // 비어 있는 셀도 resolved Style을 가진 명시적인 상태로 취급한다.
        (
            "tui.surface.blank-cell",
            "sha256:ec8988e176bf90cbe93c8c0d19c547dbf20fe006e08d79fc89ceb7d052d7ba85",
        ),
        // component는 자신에게 할당된 Rect 안에서만 Surface를 읽고 변경한다.
        (
            "tui.surface.bounded-view",
            "sha256:87ae8d0afee3a38ac35fe33cc9d7edfcbc96809236d6931e1fa22f7bd5fb9634",
        ),
        // 완성된 이전·현재 frame의 차이는 항상 같은 row span 순서로 나온다.
        (
            "tui.surface.deterministic-diff",
            "sha256:269a7815cb3c6b213295b70da7c26ddc2dded7a776bfbe12353d5b2ebff41e4c",
        ),
        // viewport 좌표는 u16과 checked arithmetic으로 안전하게 계산한다.
        (
            "tui.surface.geometry",
            "sha256:41f9fe004f1a95d1d95b6810cd05408e5e356e17858ee3f7f0002164d2abff8f",
        ),
        // wide grapheme은 leader와 역참조 가능한 continuation 셀로 표현한다.
        (
            "tui.surface.grapheme-cells",
            "sha256:f0c1a62e8e1121618003f8b5c264fc77945afb7bec087037813ce2bbba6d72ff",
        ),
        // HTML은 ANSI를 흉내 내지 않고 완성된 Surface를 직접 투영한다.
        (
            "tui.surface.html-projection",
            "sha256:8779702ef532b1b0761c59cb10ba935dac5fe84537f6cb9f10231c27e133cd21",
        ),
        // 기존 wide grapheme과 겹치면 기존 footprint 전체를 원자적으로 정리한다.
        (
            "tui.surface.intersecting-overwrite",
            "sha256:f3a763bf9e406f42fc674a22fbe37e1585074bffb66436854553f48408d2aa0f",
        ),
        // Surface는 완성된 2차원 셀 상태만 소유하고 terminal lifecycle은 소유하지 않는다.
        (
            "tui.surface.model-ownership",
            "sha256:d1529670a39e3d9ca4cda0fcaf822c2afee833043334b1894931f25e821bcd24",
        ),
        // 셀에는 theme 역할이 아니라 최종 계산된 Style을 inline으로 저장한다.
        (
            "tui.surface.resolved-style",
            "sha256:7210c4f0cdb9a5f7382c0e7edb7ea24d40b42181259854d2e4ae558b284ac33e",
        ),
        // terminal 출력은 FrameDiff에서 typed operation을 거쳐 ANSI로 변환한다.
        (
            "tui.surface.terminal-ops",
            "sha256:ad84fe74ecf5998e0f5f20c92ac793d02550bc114b0836d580d916b47d63c1b1",
        ),
        // 문자열은 Unicode 17.0 extended grapheme cluster 단위로 나눈다.
        (
            "tui.surface.text-segmentation",
            "sha256:a4911404c56747266cada0136602123dc0c06115f330f8bd6e7fbd355bdd46f3",
        ),
        // 실제 PTY를 출력 권위로 두고 tmux·SSH 환경의 미검증 상태도 추적한다.
        (
            "tui.surface.validation-matrix",
            "sha256:b50c6d872a02e25a95c7397bf8c46a1f32bbcdeb22d429c49832cffbb9e1bd1d",
        ),
        // terminal과 HTML은 동일한 Unicode 17.0 기반 셀 너비 규칙을 사용한다.
        (
            "tui.surface.width-profile",
            "sha256:6f83deba02e9cce6473c947191acca41e160fe9a78a3a2b9e1646ecd5aac0883",
        ),
    ] {
        let unit = units
            .iter()
            .find(|unit| unit["id"] == id)
            .unwrap_or_else(|| panic!("repository corpus lost Surface unit `{id}`"));
        assert_eq!(unit["revision"], revision);
        // 이 테스트는 corpus의 exact revision과 trusted approval만 소유한다.
        // 활성 여부는 현재 Checkpoint에 따라 달라지므로 checkpoint_flow에서 검증한다.
        assert_eq!(unit["effective_approval"], "approved");
        assert_eq!(unit["approval_evidence"], "trusted_approval");
    }
    assert_eq!(report["diagnostics"].as_array().map(Vec::len), Some(0));
}

// 쉼표 목록과 반복된 --only가 같은 정규화된 요청과 선행 검사 계획으로 실행되는지 확인한다.
#[test]
fn check_only_accepts_comma_lists_and_repeated_flags_equivalently() {
    let repository = CorpusRepository::without_active_checkpoint();
    let comma = methexis()
        .current_dir(&repository.path)
        .args(["check", "--only", " authority, artifacts "])
        .output()
        .expect("run comma-separated selection");
    let repeated = methexis()
        .current_dir(&repository.path)
        .args([
            "check",
            "--only=authority",
            "--only",
            "artifacts",
            "--only",
            "authority",
        ])
        .output()
        .expect("run repeated selection");

    assert!(comma.status.success());
    assert!(repeated.status.success());
    let comma: serde_json::Value =
        serde_json::from_slice(&comma.stdout).expect("comma output is JSON");
    let repeated: serde_json::Value =
        serde_json::from_slice(&repeated.stdout).expect("repeated output is JSON");
    assert_eq!(
        comma["requested_checks"],
        serde_json::json!(["authority", "artifacts"])
    );
    assert_eq!(
        comma["executed_checks"],
        serde_json::json!(["records", "relations", "authority", "artifacts"])
    );
    assert_eq!(comma, repeated);
}

// 알 수 없는 이름이나 빈 쉼표 항목은 검사를 일부 실행하지 않고 구조화된 사용 오류로 거부한다.
#[test]
fn check_only_rejects_unknown_and_empty_selectors() {
    for selectors in [
        &["check", "--only", "records,unknown"][..],
        &["check", "--only", "Authority"][..],
        &["check", "--only", "authority,"][..],
        &["check", "--only="][..],
    ] {
        let output = methexis()
            .args(selectors)
            .output()
            .expect("run invalid selection");

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: serde_json::Value =
            serde_json::from_slice(&output.stderr).expect("error is JSON");
        assert_eq!(error["error"]["code"], "invalid_check_selector");
    }
}

// 잘못된 fixture 저장소에서 check는 종료 코드 2와 실패 보고 JSON을 stdout이 아닌 stderr로 보낸다.
#[test]
fn check_reports_validation_failures_on_stderr() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("local-invalid");
    let output = methexis()
        .current_dir(fixture)
        .arg("check")
        .output()
        .expect("run failing methexis check");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("failure output is JSON");
    assert_eq!(report["schema"], "methexis.check/v1alpha1");
    assert_eq!(report["ok"], false);
    assert_eq!(report["authority"], "draft");
    assert_eq!(report["snapshot_revision"], serde_json::Value::Null);
}
