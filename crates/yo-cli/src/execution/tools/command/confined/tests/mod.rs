use std::{
    fs,
    path::Path,
    slice,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use yo_core::{
    ToolExecution, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionPreparation,
    ToolExecutionRequest,
};

use super::super::{CommandConfinement, ConfinedPlan};
use crate::{
    execution::tools::{
        command::CommandExecution,
        registry::{LocalToolRegistryRevision, registry},
        tests::{TestDirectory, request},
    },
    state::config,
};

mod macos;

fn call(command: &str) -> ToolExecutionRequest {
    let mut request = request(
        &registry(LocalToolRegistryRevision::BasicFilesV2).unwrap(),
        "run_command",
        &json!({"command":command}).to_string(),
    );
    request.absolute_execution_timeout = Some(Duration::from_secs(45));
    request.maximum_output_bytes = 16384;
    request
}

fn plan(
    policy: &CommandConfinement,
    request: &ToolExecutionRequest,
    approved: bool,
) -> ConfinedPlan {
    let plan = match policy.prepare(request, None) {
        ToolExecutionPreparation::Automatic(plan) if !approved => plan,
        ToolExecutionPreparation::ApprovalRequired(plan) if approved => plan,
        ToolExecutionPreparation::Unavailable(reason) => {
            panic!("qualification unavailable: {}", reason.code())
        },
        _ => panic!("unexpected approval disposition"),
    };
    plan.into_payload().unwrap()
}

fn execute(plan: ConfinedPlan, request: &ToolExecutionRequest) -> yo_core::ToolExecutionResult {
    let mut execution = CommandExecution::spawn_confined(plan, request).unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(50) {
        if execution.poll().unwrap() == ToolExecutionPoll::Ready {
            let result = execution.take_result().unwrap();
            execution.shutdown().unwrap();
            return result;
        }
        thread::sleep(Duration::from_millis(10));
    }
    execution.cancel();
    let _ = execution.shutdown();
    panic!("qualification exceeded bounded deadline")
}

fn successful(policy: &CommandConfinement, command: &str, approved: bool) {
    let request = call(command);
    let result = execute(plan(policy, &request, approved), &request);
    assert_eq!(
        result.outcome(),
        ToolExecutionOutcome::Completed,
        "{}",
        result.output()
    );
}

fn python(script: &str) -> String {
    format!("/usr/bin/python3 -c '{}'", script.replace('\'', "'\\''"))
}

// 비지원 플랫폼도 Planned를 무제한 legacy 호출로 바꾸지 않고 안정된 준비 실패를 돌려준다.
#[test]
fn requested_network_has_no_shared_host_network_fallback() {
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    assert!(matches!(
        policy.prepare(&call("curl https://example.invalid"), None),
        ToolExecutionPreparation::Unavailable(_)
    ));
}

#[cfg(target_os = "linux")]
// 실제 Linux 자식에서 보호된 이름과 추상 host IPC를 거부하면서 private stream/datagram/socketpair를
// 유지한다.
#[test]
#[ignore = "Linux bwrap와 Python을 사용하는 실제 호스트 격리 qualification"]
fn linux_qualification_preserves_private_ipc_and_denies_protected_names() {
    use std::{
        os::{
            linux::net::SocketAddrExt,
            unix::net::{SocketAddr, UnixDatagram, UnixListener},
        },
        process,
    };

    use nix::unistd::dup;
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let state = workspace.join("private-state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("token"), "fixture-secret").unwrap();
    let agent = workspace.join("agent.sock");
    let _agent = UnixListener::bind(&agent).unwrap();
    let datagram = workspace.join("agent-dgram.sock");
    let _datagram = UnixDatagram::bind(&datagram).unwrap();
    let abstract_name = format!("yo-qualification-{}", process::id());
    let _abstract_agent =
        UnixListener::bind_addr(&SocketAddr::from_abstract_name(abstract_name.as_bytes()).unwrap())
            .unwrap();
    let outside = root.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("read"), "fixture-read").unwrap();
    let _inheritable_secret = dup(fs::File::open(state.join("token")).unwrap()).unwrap();
    let _inheritable_agent = dup(&_agent).unwrap();
    let policy = CommandConfinement::new(
        &workspace,
        vec![state.clone()],
        vec![agent.clone(), datagram.clone()],
        vec![(outside.clone(), outside.clone())],
    )
    .unwrap();
    let script = format!(
        r#"
import os,socket,pathlib,subprocess
state={state:?};agent={agent:?};datagram={datagram:?};outside={outside:?}
abstract_name={abstract_name:?}
assert not pathlib.Path(state+'/token').exists()
for kind,path in [(socket.SOCK_STREAM,agent),(socket.SOCK_DGRAM,datagram)]:
 s=socket.socket(socket.AF_UNIX,kind);s.settimeout(1)
 try:
  if kind==socket.SOCK_DGRAM:s.sendto(b'forbidden',path)
  else:s.connect(path)
 except OSError:pass
 else:raise AssertionError('protected host endpoint reachable')
 s.close()
s=socket.socket(socket.AF_UNIX);s.settimeout(1)
try:s.connect(chr(0)+abstract_name)
except OSError:pass
else:raise AssertionError('abstract host endpoint reachable')
s.close()
assert pathlib.Path(outside+'/read').read_text()=='fixture-read'
try:pathlib.Path(outside+'/write').write_text('denied')
except OSError:pass
else:raise AssertionError('readonly support root writable')
assert os.environ['HOME']=='/yo-private/home'
assert os.environ['TMPDIR']=='/yo-private/tmp'
assert 'SSH_AUTH_SOCK' not in os.environ
assert not pathlib.Path('/proc/{host_pid}').exists()
for entry in pathlib.Path('/proc/self/fd').iterdir():
 if int(entry.name)>2:
  try:os.fstat(int(entry.name))
  except OSError:pass
  else:raise AssertionError('non-stdio inherited descriptor')
for operation in [['umount',state],['mount','-o','remount,rw',outside]]:
 attempt=subprocess.run(['unshare','--user','--map-root-user','--mount','--']+operation,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=2)
 assert attempt.returncode != 0
 assert not pathlib.Path(state+'/token').exists()
try:pathlib.Path(outside+'/after-nested').write_text('denied')
except OSError:pass
else:raise AssertionError('nested namespace changed readonly support')
for name in ['HOME','TMPDIR']:pathlib.Path(os.environ[name], 'private-cache').write_text('ok')
for base in [os.getcwd(),os.environ['TMPDIR']]:
 for kind in [socket.SOCK_STREAM,socket.SOCK_DGRAM]:
  path=base+'/child-'+str(kind)+'.sock'
  server=socket.socket(socket.AF_UNIX,kind);server.settimeout(1);server.bind(path)
  client=socket.socket(socket.AF_UNIX,kind);client.settimeout(1)
  if kind==socket.SOCK_STREAM:
   server.listen(1);client.connect(path);accepted,_=server.accept();client.sendall(b'ok');assert accepted.recv(2)==b'ok';accepted.close()
  else:client.sendto(b'ok',path);assert server.recv(2)==b'ok'
  client.close();server.close();os.unlink(path)
a,b=socket.socketpair();a.sendall(b'ok');assert b.recv(2)==b'ok';a.close();b.close()
pathlib.Path('ordinary-write').write_text('ok')
"#,
        host_pid = process::id(),
        state = state.to_str().unwrap(),
        agent = agent.to_str().unwrap(),
        datagram = datagram.to_str().unwrap(),
        outside = outside.to_str().unwrap()
    );
    successful(&policy, &python(&script), false);
    assert_eq!(
        fs::read_to_string(workspace.join("ordinary-write")).unwrap(),
        "ok"
    );
    assert_eq!(
        fs::read_to_string(state.join("token")).unwrap(),
        "fixture-secret"
    );
}

#[cfg(target_os = "linux")]
// 계획 이후 보호 이름에 소켓을 만들어도 mask가 적용되며, 보호 ancestor 교체는 실행 전에 실패한다.
#[test]
#[ignore = "Linux bwrap와 Python을 사용하는 실제 호스트 격리 qualification"]
fn linux_qualification_masks_late_endpoints_and_rejects_ancestor_replacement() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let root = TestDirectory::new();
    let parent = root.0.join("control");
    fs::create_dir(&parent).unwrap();
    let endpoint = parent.join("late.sock");
    let policy = CommandConfinement::new(&root.0, vec![], vec![endpoint.clone()], vec![]).unwrap();
    let script = format!(
        "import socket;s=socket.socket(socket.AF_UNIX);s.settimeout(1)\ntry:s.connect({:?})\nexcept OSError:pass\nelse:raise AssertionError('late endpoint reachable')",
        endpoint.to_str().unwrap()
    );
    let request = call(&python(&script));
    let prepared = plan(&policy, &request, false);
    let _server = UnixListener::bind(&endpoint).unwrap();
    assert_eq!(
        execute(prepared, &request).outcome(),
        ToolExecutionOutcome::Completed
    );
    let parent_name = parent.to_str().unwrap();
    successful(
        &policy,
        &python(&format!(
            r#"
import os,pathlib
try:os.rename({parent_name:?}, {parent_name:?}+'-moved')
except OSError:pass
else:raise AssertionError('protected endpoint ancestor moved after spawn')
assert pathlib.Path({parent_name:?}).is_dir()
"#
        )),
        false,
    );
    let request = call("printf bad > spawned");
    let prepared = plan(&policy, &request, false);
    fs::rename(&parent, root.0.join("old-control")).unwrap();
    fs::create_dir(&parent).unwrap();
    assert_eq!(
        execute(prepared, &request).outcome(),
        ToolExecutionOutcome::Failed
    );
    assert!(!root.0.join("spawned").exists());
    fs::remove_dir(&parent).unwrap();
    symlink(root.0.join("old-control"), &parent).unwrap();
    assert!(matches!(
        policy.prepare(&request, None),
        ToolExecutionPreparation::Unavailable(_)
    ));
}

#[cfg(target_os = "linux")]
// 인접 캐시를 쓰기 grant 없이 노출하지 않고 private 캐시·workspace target으로 실제 cargo test를
// 실행한다.
#[test]
#[ignore = "설치된 Rust toolchain과 readonly Cargo 의존성 캐시를 사용하는 실제 호스트 qualification"]
fn linux_qualification_runs_ordinary_cargo_and_git_automatically() {
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("Cargo.toml"),"[package]\nname = \"yo-confinement-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[dependencies]\nserde = \"=1.0.229\"\n").unwrap();
    fs::write(workspace.join("src/lib.rs"),"#[test] fn fixture() { fn serializable<T: serde::Serialize>(_: T) {} serializable(4_u32); }\n").unwrap();
    let mut policy =
        CommandConfinement::from_environment(&workspace, vec![root.0.join("state")]).unwrap();
    let identity_home = root.0.join("identity-home");
    fs::create_dir(&identity_home).unwrap();
    fs::write(identity_home.join(".gitconfig"), "[user]\nname = Fixture\nemail = fixture@example.invalid\n[credential]\nhelper = !printf forbidden-helper\n").unwrap();
    policy.git_identity = super::read_git_identity(&workspace, &identity_home, None);
    assert_eq!(policy.git_identity.len(), 4);
    successful(
        &policy,
        "CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test --offline && git init -q && git add Cargo.toml src && git commit -qm fixture && test \"$(git log -1 --format=%ae)\" = fixture@example.invalid",
        false,
    );
    assert!(workspace.join("target").is_dir());
    assert!(workspace.join(".git").is_dir());
}

#[cfg(target_os = "linux")]
// linked worktree의 외부 metadata는 자동 읽기만 허용하고 쓰기는 정확한 resolved root 승인을
// 요구한다.
#[test]
#[ignore = "Linux bwrap와 Git을 사용하는 실제 호스트 격리 qualification"]
fn linux_qualification_external_git_metadata_requires_exact_scope() {
    use std::process::Command;
    let root = TestDirectory::new();
    let repository = root.0.join("repository");
    fs::create_dir(&repository).unwrap();
    let workspace = root.0.join("worktree");
    let git = |args: &[&str]| {
        let output = Command::new("/usr/bin/git")
            .current_dir(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "--allow-empty",
        "-qm",
        "fixture",
    ]);
    git(&[
        "worktree",
        "add",
        "-qb",
        "other",
        workspace.to_str().unwrap(),
    ]);
    let policy = CommandConfinement::new(&workspace, vec![], vec![], vec![]).unwrap();
    successful(&policy, "git status --porcelain && git diff", false);
    fs::write(workspace.join("new"), "contents").unwrap();
    let request = call("git add new");
    let prepared = match policy.prepare(&request, None) {
        ToolExecutionPreparation::ApprovalRequired(plan) => {
            assert!(plan.approval_scope().contains(repository.to_str().unwrap()));
            plan.into_payload().unwrap()
        },
        _ => panic!("external metadata write must request exact roots"),
    };
    assert_eq!(
        execute(prepared, &request).outcome(),
        ToolExecutionOutcome::Completed
    );
}

#[cfg(target_os = "linux")]
// 승인 뒤 제한을 바꾼 요청은 runner를 만들기 전에 거절한다.
#[test]
#[ignore = "Linux bwrap를 사용하는 실제 호스트 계획 소비 qualification"]
fn linux_qualification_rejects_changed_execution_limits_before_spawn() {
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let request = call("printf executed > forbidden");
    for change in 0..3 {
        let prepared = plan(&policy, &request, false);
        let mut changed = request.clone();
        match change {
            0 => changed.maximum_output_bytes += 1,
            1 => changed.maximum_retained_output_bytes = Some(1_000_000),
            _ => changed.absolute_execution_timeout = None,
        }
        assert!(CommandExecution::spawn_confined(prepared, &changed).is_err());
        assert!(!root.0.join("forbidden").exists());
    }
}

#[cfg(target_os = "linux")]
// 전용 PID namespace의 자손이 setsid로 process group을 벗어나도 취소 뒤 작업을 계속하지 못한다.
#[test]
#[ignore = "실제 Linux 자손 종료·reap·출력 drain qualification"]
fn linux_qualification_cancels_descendants_and_drains_output() {
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let request = call(&python(
        r#"
import os,time,pathlib,sys
if os.fork()==0:
 os.setsid()
 pathlib.Path('ready').write_text('ready')
 while True:
  pathlib.Path('heartbeat').write_text(str(time.monotonic_ns()))
  time.sleep(0.02)
while True:
 print('running',flush=True)
 time.sleep(0.02)
"#,
    ));
    let mut execution =
        CommandExecution::spawn_confined(plan(&policy, &request, false), &request).unwrap();
    let start = Instant::now();
    while !root.0.join("heartbeat").exists() && start.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(root.0.join("heartbeat").exists());
    execution.cancel();
    let start = Instant::now();
    while execution.poll().unwrap() != ToolExecutionPoll::Ready
        && start.elapsed() < Duration::from_secs(10)
    {
        thread::sleep(Duration::from_millis(10));
    }
    let result = execution
        .take_result()
        .expect("bounded cancellation completes");
    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    execution.shutdown().unwrap();
    let final_heartbeat = fs::read(root.0.join("heartbeat")).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fs::read(root.0.join("heartbeat")).unwrap(), final_heartbeat);
}

#[cfg(target_os = "linux")]
// 승인한 외부 파일만 쓰고 인접 파일·비밀 root는 열지 않는다. Configured script도 같은 계획을 쓴다.
#[test]
#[ignore = "실제 Linux exact grant와 configured command qualification"]
fn linux_qualification_scopes_external_grants_and_configured_artifacts() {
    use yo_core::ToolId;

    use super::super::PreparedCommand;
    use crate::{execution::tools::command::PreparedCommandTools, state::config};
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let granted = root.0.join("granted");
    fs::write(&granted, "before").unwrap();
    let neighbor = root.0.join("neighbor");
    fs::write(&neighbor, "hidden").unwrap();
    let secret = root.0.join("secret");
    fs::create_dir(&secret).unwrap();
    fs::write(secret.join("token"), "denied").unwrap();
    let policy = CommandConfinement::new(&workspace, vec![secret.clone()], vec![], vec![])
        .unwrap()
        .with_protected_file(&secret.join("token"))
        .unwrap();
    let known_alias = root.0.join("known-credential-alias");
    fs::hard_link(secret.join("token"), &known_alias).unwrap();
    assert!(matches!(
        policy.prepare(
            &call(&format!("printf forbidden > {}", known_alias.display())),
            None
        ),
        ToolExecutionPreparation::Unavailable(_)
    ));
    let command = format!(
        "printf after > {}; test ! -e {}",
        granted.display(),
        neighbor.display()
    );
    successful(&policy, &command, true);
    assert_eq!(fs::read_to_string(&granted).unwrap(), "after");
    assert!(matches!(
        policy.prepare(
            &call(&format!(
                "printf forbidden > {}",
                secret.join("token").display()
            )),
            None
        ),
        ToolExecutionPreparation::Unavailable(_)
    ));
    let script = root.0.join("configured.sh");
    fs::write(&script, "cat; printf configured > configured-output\n").unwrap();
    let config_path = root.0.join("config.json");
    fs::write(&config_path,json!({"tools":{"commands":[{"id":"configured","name":"configured","description":"Execute fixture","executable":"/bin/sh","script":script,"parameters":{"type":"object","properties":{},"additionalProperties":false}}]}}).to_string()).unwrap();
    let config = config::load_from(&config_path).unwrap();
    let tools = PreparedCommandTools::prepare_revision(
        LocalToolRegistryRevision::CommandToolsV2,
        config.command_tools(),
        &workspace,
        &secret.join("token"),
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let mut request = call("unused");
    request.call = tools
        .registry()
        .validate_call("configured-call", "configured", "{}", 4096)
        .unwrap();
    let configured: PreparedCommand = tools.command(&ToolId::new("configured").unwrap()).unwrap();
    let prepared = match policy.prepare(&request, Some(configured)) {
        ToolExecutionPreparation::ApprovalRequired(plan) => plan.into_payload().unwrap(),
        _ => panic!("configured calls remain always approved"),
    };
    let result = execute(prepared, &request);
    assert_eq!(
        result.outcome(),
        ToolExecutionOutcome::Completed,
        "{}",
        result.output()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("configured-output")).unwrap(),
        "configured"
    );
}

// Git 포인터는 첫 초과 바이트와 FIFO·symlink에서 준비를 거부하고 정상 worktree 경로만 읽는다.
#[test]
fn git_metadata_pointer_reads_are_bounded_regular_files() {
    use std::os::unix::fs::symlink;

    use nix::{sys::stat::Mode, unistd::mkfifo};

    let root = TestDirectory::new();
    let metadata = root.0.join("metadata");
    fs::create_dir(&metadata).unwrap();
    let entry = root.0.join(".git");
    let valid = format!("gitdir: {}\n", metadata.display());
    fs::write(&entry, &valid).unwrap();
    assert_eq!(
        super::git_metadata_roots(&root.0).unwrap(),
        slice::from_ref(&metadata)
    );
    fs::write(&entry, format!("{valid}{}", " ".repeat(4096 - valid.len()))).unwrap();
    assert!(super::git_metadata_roots(&root.0).is_ok());
    fs::write(&entry, format!("{valid}{}", " ".repeat(4097 - valid.len()))).unwrap();
    assert!(super::git_metadata_roots(&root.0).is_err());
    fs::remove_file(&entry).unwrap();
    mkfifo(&entry, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    assert_git_metadata_rejected_promptly(&root.0);
    fs::remove_file(&entry).unwrap();
    let pointer = root.0.join("pointer");
    fs::write(&pointer, &valid).unwrap();
    symlink(&pointer, &entry).unwrap();
    assert!(super::git_metadata_roots(&root.0).is_err());
    fs::remove_file(&entry).unwrap();
    fs::write(&entry, &valid).unwrap();
    let common = metadata.join("commondir");
    mkfifo(&common, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    assert_git_metadata_rejected_promptly(&root.0);
    fs::remove_file(&common).unwrap();
    let common_root = root.0.join("common");
    fs::create_dir(&common_root).unwrap();
    let valid_common = "../common\n";
    fs::write(
        &common,
        format!("{valid_common}{}", " ".repeat(4096 - valid_common.len())),
    )
    .unwrap();
    assert_eq!(
        super::git_metadata_roots(&root.0).unwrap(),
        [metadata.clone(), common_root]
    );
    fs::write(
        &common,
        format!("{valid_common}{}", " ".repeat(4097 - valid_common.len())),
    )
    .unwrap();
    assert!(super::git_metadata_roots(&root.0).is_err());
    fs::remove_file(&common).unwrap();
    symlink(&entry, &common).unwrap();
    assert!(super::git_metadata_roots(&root.0).is_err());
    fs::remove_file(&common).unwrap();
    fs::create_dir(&common).unwrap();
    assert!(super::git_metadata_roots(&root.0).is_err());
    fs::remove_dir(&common).unwrap();
    fs::remove_file(&entry).unwrap();
    fs::create_dir(&entry).unwrap();
    assert_eq!(super::git_metadata_roots(&root.0).unwrap(), [entry]);
}

fn assert_git_metadata_rejected_promptly(workspace: &Path) {
    let workspace = workspace.to_owned();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = sender.send(super::git_metadata_roots(&workspace).is_err());
    });
    assert_eq!(receiver.recv_timeout(Duration::from_secs(2)), Ok(true));
    worker.join().unwrap();
}

// 상대 설정에서 유래한 credential 경로도 실제 cwd 기준 절대 endpoint로 고정한다.
#[test]
fn relative_config_credential_is_frozen_against_startup_directory() {
    use std::env;

    let root = TestDirectory::new();
    let config = config::load_from(Path::new("config.yaml")).unwrap();
    let credential = config.credential_path();
    assert_eq!(credential, Path::new("./credentials.yaml"));
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![])
        .unwrap()
        .with_protected_file(&credential)
        .unwrap();
    let expected = env::current_dir().unwrap().join("credentials.yaml");
    assert!(policy.secret_endpoints.contains(&expected));
    assert!(
        policy
            .secret_endpoints
            .iter()
            .all(|path| path.is_absolute())
    );
}

#[cfg(target_os = "linux")]
// 실제 Git의 exclude 값·dry-run 순서와 소비한 계획의 승인 판정을 함께 비교한다.
#[test]
#[ignore = "실제 Linux 격리 자식과 Git clean 의미 qualification"]
fn linux_qualification_git_clean_option_values_and_effective_dry_run() {
    for (arguments, approval, removed) in [
        ("-f -e --help", true, true),
        ("-f -e -h", true, true),
        ("-f -e -n", true, true),
        ("-f -e --dry-run", true, true),
        ("-f --exclude --help", true, true),
        ("-f --exclude=--help", true, true),
        ("-f -e--help", true, true),
        ("-fe-n", true, true),
        ("-n --no-dry-run -f", true, true),
        ("-n -e $PATTERN", true, true),
        ("-n -f -e --help", false, false),
        ("-n -f -e --dry-run", false, false),
        ("-ne--help", false, false),
        ("--no-dry-run -n -f", false, false),
        ("--no-dry-run --dry-run -f", false, false),
        ("-n -e '*.kept'", false, false),
        ("-n --exclude 'ignored pattern'", false, false),
        ("-f -- --help", true, true),
        ("-n -f -- --help", false, false),
        ("-h", false, false),
    ] {
        let root = TestDirectory::new();
        let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
        successful(&policy, "git init -q", false);
        let victim = root.0.join(if arguments.contains(" -- ") {
            "--help"
        } else {
            "victim"
        });
        fs::write(&victim, "synthetic untracked fixture").unwrap();
        let command = if arguments.contains("$PATTERN") {
            format!("PATTERN='ignored --no-dry-run -f'; git clean {arguments}")
        } else {
            format!("git clean {arguments}")
        };
        let request = call(&command);
        let result = execute(plan(&policy, &request, approval), &request);
        if arguments != "-h" {
            assert_eq!(
                result.outcome(),
                ToolExecutionOutcome::Completed,
                "{arguments}: {}",
                result.output()
            );
        } else {
            assert!(result.output().contains("usage: git clean"));
        }
        assert_eq!(!victim.exists(), removed, "{arguments}");
    }
    for (arguments, approval, restored) in [
        ("-sS tracked", true, true),
        ("-sfooS tracked", true, true),
        ("-SsHEAD tracked", false, false),
        ("--pathspec-from-file --help", true, true),
        ("--pathspec-from-file --staged", true, true),
        ("--worktree --staged --no-staged tracked", true, true),
        ("--staged --pathspec-from-file --worktree", false, false),
    ] {
        let root = TestDirectory::new();
        let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
        successful(&policy, "git init -q", false);
        let tracked = root.0.join("tracked");
        fs::write(&tracked, "before").unwrap();
        successful(
            &policy,
            "git add tracked && git -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm baseline",
            false,
        );
        successful(&policy, "git branch S && git branch fooS", false);
        fs::write(&tracked, "after").unwrap();
        for name in ["--help", "--staged", "--worktree"] {
            fs::write(root.0.join(name), "tracked\n").unwrap();
        }
        let request = call(&format!("git restore {arguments}"));
        let result = execute(plan(&policy, &request, approval), &request);
        assert_eq!(
            result.outcome(),
            ToolExecutionOutcome::Completed,
            "{arguments}: {}",
            result.output()
        );
        assert_eq!(
            fs::read_to_string(tracked).unwrap(),
            if restored { "before" } else { "after" },
            "{arguments}"
        );
    }
}

#[cfg(target_os = "linux")]
// spawn 전 취소·검증 시 이미 만료한 기한·진짜 setup 오류는 서로 다른 결과이며 marker가 없다.
#[test]
#[ignore = "실제 Linux 계획의 pre-spawn interruption qualification"]
fn linux_qualification_setup_preserves_interruption_reasons_without_spawn() {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    use yo_core::ToolId;

    use super::super::{
        CommandOutput, CommandPlan, CommandTestHooks, LaunchTestHooks, PreparedCommandTools,
        limits::CommandExecutionLimits, pipe::ProgressSnapshot, run_command,
    };

    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let marker = workspace.join("executed");
    let script = root.0.join("fixture.sh");
    fs::write(&script, "printf ran > executed\n").unwrap();
    let config_path = root.0.join("config.json");
    fs::write(&config_path, json!({"tools":{"commands":[{"id":"configured","name":"configured","description":"Execute fixture","executable":"/bin/sh","script":script,"parameters":{"type":"object","properties":{},"additionalProperties":false}}]}}).to_string()).unwrap();
    let config = config::load_from(&config_path).unwrap();
    let tools = PreparedCommandTools::prepare_revision(
        LocalToolRegistryRevision::CommandToolsV2,
        config.command_tools(),
        &workspace,
        &root.0.join("credentials.yaml"),
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let policy = CommandConfinement::new(&workspace, vec![], vec![], vec![]).unwrap();
    let mut configured_request = call("unused");
    configured_request.absolute_execution_timeout = Some(Duration::from_secs(1));
    configured_request.call = tools
        .registry()
        .validate_call("configured-call", "configured", "{}", 4096)
        .unwrap();
    let configured_plan = || {
        let command = tools.command(&ToolId::new("configured").unwrap()).unwrap();
        match policy.prepare(&configured_request, Some(command)) {
            ToolExecutionPreparation::ApprovalRequired(plan) => {
                plan.into_payload::<ConfinedPlan>().unwrap()
            },
            _ => panic!("configured fixture needs its frozen approval plan"),
        }
    };
    let run = |plan: ConfinedPlan, cancelled, started| {
        run_command(
            &workspace,
            &CommandPlan::Confined(Box::new(plan)),
            CommandOutput {
                maximum_output_bytes: 16384,
                maximum_retained_output_bytes: None,
                progress: Arc::new(Mutex::new(ProgressSnapshot::default())),
            },
            CommandExecutionLimits::for_agent(Some(Duration::from_secs(1))),
            &AtomicBool::new(cancelled),
            &Mutex::new(None),
            CommandTestHooks {
                launch: LaunchTestHooks {
                    attempt_started: started,
                    ..LaunchTestHooks::default()
                },
                ..CommandTestHooks::default()
            },
        )
    };
    let shell_request = call("printf ran > executed");
    for prepared in [plan(&policy, &shell_request, false), configured_plan()] {
        let result = run(prepared, true, None);
        assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
        assert_eq!(result.output(), "run_command cancelled");
        assert!(!marker.exists());
    }
    let result = run(
        configured_plan(),
        false,
        Some(Instant::now() - Duration::from_secs(2)),
    );
    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(
        result.output(),
        "run_command absolute execution deadline expired"
    );
    assert!(!marker.exists());
    let prepared = configured_plan();
    fs::write(&script, "printf changed > executed\n").unwrap();
    let result = run(prepared, false, None);
    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert_eq!(
        result.output(),
        "workspace command confinement setup failed"
    );
    assert!(!marker.exists());
}
