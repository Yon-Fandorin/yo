use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::json;
use yo_core::{
    ToolExecution, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult, ToolId,
};

use super::{super::CommandExecution, PreparedCommandTools, encoding};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "yo-command-manifest-{}",
            yo_core::SessionId::new().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("script.sh"), b"cat\n").unwrap();
        fs::write(path.join("native"), b"fixture executable bytes\n").unwrap();
        fs::set_permissions(path.join("native"), fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn config(
        &self,
        executable: &str,
        script: Option<&str>,
        argv: &[&str],
    ) -> crate::state::config::Config {
        let mut command = json!({"id":"configured","name":"configured","description":"Run the configured fixture.","executable": executable,"parameters":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false},"argv":argv});
        if let Some(script) = script {
            command["script"] = json!(script);
        }
        let path = self.0.join("config.json");
        fs::write(&path, json!({"tools":{"commands":[command]}}).to_string()).unwrap();
        crate::state::config::load_from(&path).unwrap()
    }

    fn prepare(&self, config: &crate::state::config::Config) -> PreparedCommandTools {
        PreparedCommandTools::prepare(
            config.command_tools(),
            &self.0,
            &self.0.join("credentials.yaml"),
            &mut || false,
        )
        .unwrap()
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait(execution: &mut CommandExecution) -> ToolExecutionResult {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match execution.poll().unwrap() {
            ToolExecutionPoll::Ready => {
                let result = execution.take_result().unwrap();
                execution.shutdown().unwrap();
                return result;
            },
            ToolExecutionPoll::Pending => {
                assert!(
                    Instant::now() < deadline,
                    "configured command did not finish"
                );
                std::thread::sleep(Duration::from_millis(5));
            },
        }
    }
}

fn start(fixture: &Fixture, prepared: &PreparedCommandTools, text: &str) -> CommandExecution {
    let call = prepared
        .registry()
        .validate_call(
            "call",
            "configured",
            &json!({"text":text}).to_string(),
            101 * 1024 * 1024,
        )
        .unwrap();
    CommandExecution::spawn_prepared(
        fixture.0.clone(),
        prepared
            .command(&ToolId::new("configured").unwrap())
            .unwrap(),
        call.normalized_arguments().to_owned(),
        1024 * 1024,
        Some(Duration::from_secs(5)),
        None,
    )
    .unwrap()
}

// 별도 framing 계산 golden은 key 순서와 float -0을 정규화하고 scalar/array 순서는 구별한다.
#[test]
fn manifest_framing_has_a_stable_golden_and_distinguishes_order_and_number_kind() {
    let a: serde_json::Value =
        serde_json::from_str(r#"{"b":[true,null,1,1.0,-0.0,"é"],"a":"x"}"#).unwrap();
    let b: serde_json::Value =
        serde_json::from_str(r#"{"a":"x","b":[true,null,1,1.0,0.0,"é"]}"#).unwrap();
    assert_eq!(
        encoding::digest(&a).unwrap(),
        "sha256:be2bddafd242e535e97fd5e42a23f9c94f26d5a6fd76429ccc12d0c986b6bf4a"
    );
    assert_eq!(encoding::digest(&a).unwrap(), encoding::digest(&b).unwrap());
    assert_ne!(
        encoding::digest(&json!([1])).unwrap(),
        encoding::digest(&json!([1.0])).unwrap()
    );
    assert_ne!(
        encoding::digest(&json!(["a", "b"])).unwrap(),
        encoding::digest(&json!(["b", "a"])).unwrap()
    );
    assert!(encoding::digest(&json!("x".repeat(2 * 1024 * 1024))).is_err());
}

// 빈 command 설정은 없는 workspace/credential도 열지 않으며 구성된 목록은 built-in 뒤에 붙는다.
#[test]
fn empty_commands_skip_artifacts_and_preparation_freezes_the_exact_registry() {
    let fixture = Fixture::new();
    assert!(
        PreparedCommandTools::prepare(
            &[],
            &fixture.0.join("missing"),
            &fixture.0.join("missing"),
            &mut || true
        )
        .unwrap()
        .is_none()
    );
    let config = fixture.config(
        fixture.0.join("native").to_str().unwrap(),
        Some("script.sh"),
        &["one", "two"],
    );
    let prepared = fixture.prepare(&config);
    assert_eq!(
        prepared
            .registry()
            .definitions()
            .iter()
            .map(|d| d.wire_name())
            .collect::<Vec<_>>(),
        [
            "list_files",
            "read_files",
            "edit_file",
            "write_file",
            "run_command",
            "configured"
        ]
    );
    assert!(prepared.host_identity().contains(prepared.digest()));
    assert!(
        prepared
            .registry()
            .validate_call(
                "call",
                "configured",
                &json!({"text":"x".repeat(4 * 1024 * 1024)}).to_string(),
                101 * 1024 * 1024
            )
            .is_err()
    );
    let changed = fixture.config(
        fixture.0.join("native").to_str().unwrap(),
        Some("script.sh"),
        &["two", "one"],
    );
    assert_ne!(prepared.digest(), fixture.prepare(&changed).digest());
}

// 저장된 model projection 비교는 artifact가 사라져도 파일을 열지 않고 정의 순서와 내용을 판정한다.
#[test]
fn replay_projection_is_checked_without_opening_configured_artifacts() {
    let fixture = Fixture::new();
    let config = fixture.config(fixture.0.join("native").to_str().unwrap(), None, &[]);
    let prepared = fixture.prepare(&config);
    fs::remove_file(fixture.0.join("native")).unwrap();
    let projection = prepared.registry().replay_tools();
    let contract = yo_core::ModelReplayContract::new("system", projection.clone());
    assert!(
        PreparedCommandTools::validate_replay_contract(config.command_tools(), Some(&contract))
            .is_ok()
    );
    let mut changed = projection;
    changed.swap(0, 1);
    let changed = yo_core::ModelReplayContract::new("system", changed);
    assert!(
        PreparedCommandTools::validate_replay_contract(config.command_tools(), Some(&changed))
            .is_err()
    );
    assert!(PreparedCommandTools::validate_replay_contract(config.command_tools(), None).is_err());
    assert!(PreparedCommandTools::validate_replay_contract(&[], Some(&contract)).is_err());
}

// 명시 command 순서도 실행 manifest identity이며 같은 artifact의 재사용은 순서를 지우지 않는다.
#[test]
fn configured_tool_order_changes_the_frozen_manifest_digest() {
    let fixture = Fixture::new();
    fixture.config(fixture.0.join("native").to_str().unwrap(), None, &[]);
    let path = fixture.0.join("config.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut second = value["tools"]["commands"][0].clone();
    second["id"] = json!("second");
    second["name"] = json!("second");
    value["tools"]["commands"]
        .as_array_mut()
        .unwrap()
        .push(second);
    fs::write(&path, value.to_string()).unwrap();
    let first = fixture.prepare(&crate::state::config::load_from(&path).unwrap());
    value["tools"]["commands"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    fs::write(&path, value.to_string()).unwrap();
    let second = fixture.prepare(&crate::state::config::load_from(&path).unwrap());
    assert_ne!(first.digest(), second.digest());
}

// inode 교체만으로 같은 설치를 거절하지 않되 bytes·configured mapping 변경은 digest와 최종검증을
// 바꾼다.
#[test]
fn artifact_bytes_and_symlink_mapping_are_frozen_but_inode_numbers_are_not_identity() {
    let fixture = Fixture::new();
    let executable = fixture.0.join("native");
    let link = fixture.0.join("installed");
    symlink(&executable, &link).unwrap();
    let config = fixture.config(link.to_str().unwrap(), Some("script.sh"), &[]);
    let prepared = fixture.prepare(&config);
    let command = prepared
        .command(&ToolId::new("configured").unwrap())
        .unwrap();
    fs::write(fixture.0.join("new"), fs::read(&executable).unwrap()).unwrap();
    fs::set_permissions(fixture.0.join("new"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(fixture.0.join("new"), &executable).unwrap();
    assert_eq!(prepared.digest(), fixture.prepare(&config).digest());
    assert!(command.verify_for_launch(&mut || false, None).is_ok());
    fs::write(&executable, b"changed bytes\n").unwrap();
    assert_ne!(prepared.digest(), fixture.prepare(&config).digest());
    assert!(command.verify_for_launch(&mut || false, None).is_err());
    fs::copy(&executable, fixture.0.join("other")).unwrap();
    fs::remove_file(&link).unwrap();
    symlink(fixture.0.join("other"), &link).unwrap();
    assert!(command.verify_for_launch(&mut || false, None).is_err());
}

// 저장된 workspace의 중간 디렉터리가 symlink로 바뀌면 같은 bytes라도 script 최종검증이 거절한다.
#[test]
fn final_script_verification_rejects_a_replaced_workspace_ancestor_symlink() {
    let fixture = Fixture::new();
    let ancestor = fixture.0.join("ancestor");
    let workspace = ancestor.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("script.sh"), b"cat\n").unwrap();
    let config = fixture.config("/bin/sh", Some("script.sh"), &[]);
    let prepared = PreparedCommandTools::prepare(
        config.command_tools(),
        &workspace,
        &fixture.0.join("credentials.yaml"),
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let command = prepared
        .command(&ToolId::new("configured").unwrap())
        .unwrap();
    assert!(command.verify_for_launch(&mut || false, None).is_ok());
    fs::rename(&ancestor, fixture.0.join("moved")).unwrap();
    symlink(fixture.0.join("moved"), &ancestor).unwrap();
    assert!(command.verify_for_launch(&mut || false, None).is_err());
}

// script symlink, credential hardlink, FIFO와 41번째 executable symlink는 launch 없이 거절한다.
#[test]
fn artifact_admission_rejects_credential_aliases_scripts_links_and_nonregular_files() {
    let fixture = Fixture::new();
    let executable = fixture.0.join("native");
    fs::write(fixture.0.join("credentials.yaml"), b"credential fixture").unwrap();
    fs::hard_link(fixture.0.join("credentials.yaml"), fixture.0.join("alias")).unwrap();
    symlink("script.sh", fixture.0.join("linked-script")).unwrap();
    nix::unistd::mkfifo(
        &fixture.0.join("fifo"),
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    for script in ["alias", "linked-script", "fifo"] {
        let config = fixture.config(executable.to_str().unwrap(), Some(script), &[]);
        assert!(
            PreparedCommandTools::prepare(
                config.command_tools(),
                &fixture.0,
                &fixture.0.join("credentials.yaml"),
                &mut || false
            )
            .is_err()
        );
    }
    let mut target = executable;
    for number in 0..41 {
        let link = fixture.0.join(format!("link-{number}"));
        symlink(&target, &link).unwrap();
        target = link;
        if number >= 39 {
            let config = fixture.config(target.to_str().unwrap(), None, &[]);
            assert_eq!(
                PreparedCommandTools::prepare(
                    config.command_tools(),
                    &fixture.0,
                    &fixture.0.join("credentials.yaml"),
                    &mut || false
                )
                .is_ok(),
                number == 39
            );
        }
    }
}

// interpreter는 고정 argv를 literal로 받고 stdin의 정규화 JSON+LF 뒤 EOF를 정확히 한 번 본다.
#[test]
fn configured_shell_receives_literal_argv_and_normalized_json_stdin() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("script.sh"),
        b"printf 'arg:%s\\n' \"$@\"\nprintf 'path:%s\\n' \"$PATH\"\ncat\n",
    )
    .unwrap();
    let config = fixture.config(
        "/bin/sh",
        Some("script.sh"),
        &["$(touch must-not-exist)", "two words"],
    );
    let prepared = fixture.prepare(&config);
    let result = wait(&mut start(&fixture, &prepared, "value"));
    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(
        result
            .output()
            .contains("arg:$(touch must-not-exist)\narg:two words\n")
    );
    assert!(
        result
            .output()
            .contains("path:/usr/local/bin:/usr/bin:/bin\n")
    );
    assert!(result.output().contains("{\"text\":\"value\"}\n"));
    assert!(!fixture.0.join("must-not-exist").exists());
}

// script 없는 native 실행도 stdin의 정규화 JSON+LF를 한 번만 받고 EOF로 완료한다.
#[test]
fn configured_native_executable_receives_json_and_eof_without_a_script() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(&fixture.config("/usr/bin/cat", None, &[]));
    let result = wait(&mut start(&fixture, &prepared, "native"));
    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert_eq!(
        result.output(),
        "status: 0\nstdout:\n{\"text\":\"native\"}\n\nstderr:\n"
    );
}

// 설치된 interpreter의 원래 script 경로·argv와 JSON stdin을 실제 Python/Node 프로세스로 검증한다.
#[test]
#[ignore = "requires installed /usr/bin/python3 and /usr/bin/node; run explicitly in the host integration gate"]
fn configured_python_and_node_keep_original_script_paths_and_literal_arguments() {
    let fixture = Fixture::new();
    for (executable, script, source) in [
        (
            "/usr/bin/python3",
            "script.py",
            "import json, os, sys\nprint(sys.argv[0])\nprint(sys.argv[1])\nprint(os.getcwd())\nprint(os.environ['PATH'])\nsys.stdout.write(sys.stdin.read())\n",
        ),
        (
            "/usr/bin/node",
            "script.js",
            "const fs = require('fs');\nconsole.log(process.argv[1]);\nconsole.log(process.argv[2]);\nconsole.log(process.cwd());\nconsole.log(process.env.PATH);\nprocess.stdout.write(fs.readFileSync(0));\n",
        ),
    ] {
        fs::write(fixture.0.join(script), source).unwrap();
        let prepared = fixture.prepare(&fixture.config(
            executable,
            Some(script),
            &["$(touch must-not-exist)"],
        ));
        let result = wait(&mut start(&fixture, &prepared, "interpreter"));
        assert_eq!(
            result.outcome(),
            ToolExecutionOutcome::Completed,
            "{executable}: {}",
            result.output()
        );
        let expected = format!(
            "{}\n$(touch must-not-exist)\n{}\n/usr/local/bin:/usr/bin:/bin\n{{\"text\":\"interpreter\"}}\n",
            fixture.0.join(script).display(),
            fixture.0.display()
        );
        assert!(
            result.output().contains(&expected),
            "{executable}: {}",
            result.output()
        );
        assert!(!fixture.0.join("must-not-exist").exists());
    }
}

// 출력이 stdin보다 먼저 커도 교착하지 않으며 stdin을 일찍 닫는 exit0은 실패한다.
#[test]
fn configured_stdin_is_concurrent_and_early_close_is_a_failed_attempt() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("script.sh"),
        b"dd if=/dev/zero bs=65536 count=2 2>/dev/null\ncat >/dev/null\nprintf done\n",
    )
    .unwrap();
    let config = fixture.config("/bin/sh", Some("script.sh"), &[]);
    let prepared = fixture.prepare(&config);
    let result = wait(&mut start(&fixture, &prepared, &"x".repeat(1024 * 1024)));
    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(result.output().contains("done"));
    fs::write(fixture.0.join("script.sh"), b"exec 0<&-\nexit 0\n").unwrap();
    let prepared = fixture.prepare(&config);
    let result = wait(&mut start(&fixture, &prepared, &"x".repeat(1024 * 1024)));
    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert!(result.output().contains("stdin delivery failed"));
}

// 승인 뒤 artifact 변경은 worker에서 spawn을 막고 stdin을 읽지 않는 자식도 취소로 정리된다.
#[test]
fn final_artifact_verification_prevents_spawn_and_blocked_stdin_is_cancellable() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("script.sh"), b"touch executed\n").unwrap();
    let config = fixture.config("/bin/sh", Some("script.sh"), &[]);
    let prepared = fixture.prepare(&config);
    fs::write(
        fixture.0.join("script.sh"),
        b"touch executed\nprintf changed\n",
    )
    .unwrap();
    let result = wait(&mut start(&fixture, &prepared, "value"));
    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert!(!fixture.0.join("executed").exists());
    fs::write(fixture.0.join("script.sh"), b"sleep 30\n").unwrap();
    let prepared = fixture.prepare(&config);
    let mut execution = start(&fixture, &prepared, &"x".repeat(1024 * 1024));
    std::thread::sleep(Duration::from_millis(100));
    execution.cancel();
    let result = wait(&mut execution);
    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
}
