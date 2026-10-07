//! Mac 계획의 호스트 독립 불변식과 실제 Seatbelt qualification을 구분한다.

#[cfg(target_os = "macos")]
use std::process::{self, Command, Stdio};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

use yo_core::ToolExecutionRequest;

use super::{
    super::{CommandConfinement, ConfinedPlan, Platform, macos},
    TestDirectory, call,
};

fn prepare(
    policy: &CommandConfinement,
    request: &ToolExecutionRequest,
    approval: bool,
) -> ConfinedPlan {
    let (plan, actual_approval, _, _) = policy.plan_on(request, None, Platform::Macos).unwrap();
    assert_eq!(actual_approval, approval);
    plan
}

fn private_root(plan: &ConfinedPlan) -> &Path {
    Path::new(plan.manifest["macos"]["private_root"].as_str().unwrap())
}

// production release가 없는 동안 테스트용 계획 생성은 공개 준비 경계를 활성화하지 않는다.
#[test]
fn unqualified_release_has_no_production_admission() {
    assert!(!macos::available());
    #[cfg(target_os = "macos")]
    {
        let root = TestDirectory::new();
        let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
        assert!(matches!(
            policy.prepare(&call("cargo test"), None),
            yo_core::ToolExecutionPreparation::Unavailable(
                yo_core::ToolPlanUnavailable::UnqualifiedPlatform
            )
        ));
    }
}

// 승인 전에 만든 실제 HOME/TMP/cache는 0700이며 계획 종료까지 같은 identity를 유지한다.
#[test]
fn physical_resources_are_private_and_frozen_in_consumed_plan() {
    let root = TestDirectory::new();
    let source = root.0.join("source-cache");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("cached"), "cached").unwrap();
    let policy = CommandConfinement::new(
        &root.0,
        vec![],
        vec![],
        vec![(
            source.clone(),
            Path::new(super::super::PRIVATE_HOME).join(".cargo/registry"),
        )],
    )
    .unwrap();
    let request = call("git clean -fd");
    let plan = prepare(&policy, &request, true);
    let resource = private_root(&plan).to_owned();
    for path in [
        &resource,
        &resource.join("home"),
        &resource.join("tmp"),
        &resource.join("home/.cargo"),
    ] {
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o700);
    }
    assert!(plan.matches(&request));
    assert_eq!(
        plan.manifest["platform"],
        "macos-seatbelt-path-process-group-v1alpha1"
    );
    assert_eq!(
        plan.manifest["process_scope"],
        "process-group-bounded-cleanup"
    );
    assert_eq!(plan.manifest["private_unix_ipc"], false);
    assert_eq!(
        plan.manifest["macos"]["mach_lookups"],
        serde_json::json!([])
    );
    assert_eq!(
        fs::read_link(resource.join("home/.cargo/registry")).unwrap(),
        source.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read_to_string(resource.join("home/.cargo/registry/cached")).unwrap(),
        "cached"
    );
    assert_eq!(
        plan.environment
            .iter()
            .find(|(name, _)| name == "HOME")
            .unwrap()
            .1,
        resource.join("home").to_str().unwrap()
    );
    let second = prepare(&policy, &request, true);
    assert_ne!(plan.manifest, second.manifest);
    plan.macos.as_ref().unwrap().verify(&plan.roots).unwrap();
    drop(plan);
    assert!(!resource.exists());
    assert_eq!(fs::read_to_string(source.join("cached")).unwrap(), "cached");
}

// 교체한 이름은 거부하며 원래 계획 drop이 대체 디렉터리의 파일을 지우지 않는다.
#[test]
fn replaced_private_directory_is_rejected_and_preserved() {
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let plan = prepare(&policy, &call("printf ok"), false);
    let resource = private_root(&plan).to_owned();
    let moved = resource.with_extension("moved");
    fs::rename(&resource, &moved).unwrap();
    fs::create_dir(&resource).unwrap();
    fs::write(resource.join("replacement"), "retain").unwrap();
    assert!(plan.macos.as_ref().unwrap().verify(&plan.roots).is_err());
    drop(plan);
    assert_eq!(
        fs::read_to_string(resource.join("replacement")).unwrap(),
        "retain"
    );
    fs::remove_dir_all(resource).unwrap();
    fs::remove_dir_all(moved).unwrap();
}

// 호출이 HOME을 바꾸어도 소유한 root는 제거하고 외부 symlink 대상은 따라가서 지우지 않는다.
#[test]
fn cleanup_removes_owned_tree_without_following_replaced_home() {
    use std::os::unix::fs::symlink;
    let root = TestDirectory::new();
    let outside = root.0.join("external");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("retain"), "retain").unwrap();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let plan = prepare(&policy, &call("printf ok"), false);
    let resource = private_root(&plan).to_owned();
    fs::rename(resource.join("home"), resource.join("moved-home")).unwrap();
    symlink(&outside, resource.join("home")).unwrap();
    assert!(plan.macos.as_ref().unwrap().verify(&plan.roots).is_err());
    drop(plan);
    assert!(!resource.exists());
    assert_eq!(
        fs::read_to_string(outside.join("retain")).unwrap(),
        "retain"
    );
}

// 명령 결과를 전달하기 전에 물리 자원을 제거하고 성공 output은 그대로 보존한다.
#[test]
fn completed_result_is_published_after_private_cleanup() {
    use std::time::Duration;

    use yo_core::{ToolExecutionOutcome, ToolExecutionResult};
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let plan = prepare(&policy, &call("printf ok"), false);
    let resource = private_root(&plan).to_owned();
    fs::write(resource.join("home/command-data"), "private").unwrap();
    let result = plan.finalize(
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, "command output", false),
        Duration::from_secs(1),
    );
    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert_eq!(result.output(), "command output");
    assert!(!resource.exists());
}

// cleanup 시 root가 바뀌면 대체 디렉터리를 보존하고 Completed를 정적 Failed 결과로 바꾼다.
#[test]
fn cleanup_failure_is_visible_without_deleting_replaced_root() {
    use std::time::Duration;

    use yo_core::{ToolExecutionOutcome, ToolExecutionResult};
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let plan = prepare(&policy, &call("printf ok"), false);
    let resource = private_root(&plan).to_owned();
    let moved = resource.with_extension("moved");
    fs::rename(&resource, &moved).unwrap();
    fs::create_dir(&resource).unwrap();
    fs::write(resource.join("retain"), "retain").unwrap();
    let result = plan.finalize(
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, "ok", false),
        Duration::from_secs(1),
    );
    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert_eq!(result.output(), "workspace command private cleanup failed");
    assert_eq!(
        fs::read_to_string(resource.join("retain")).unwrap(),
        "retain"
    );
    fs::remove_dir_all(resource).unwrap();
    fs::remove_dir_all(moved).unwrap();
}

// 정지한 삭제 worker는 기한에 실패를 돌려주고 소유권을 유지하다 release 뒤 삭제·join을 마친다.
#[test]
fn cleanup_timeout_retains_ownership_until_eventual_join() {
    use std::{sync::mpsc, time::Duration};
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let mut plan = prepare(&policy, &call("printf ok"), false);
    let resource = private_root(&plan).to_owned();
    let (entered_sender, entered) = mpsc::sync_channel(1);
    let (resume, resume_receiver) = mpsc::sync_channel(1);
    let (completed_sender, completed) = mpsc::sync_channel(1);
    let (outcome, worker) = plan
        .macos
        .take()
        .unwrap()
        .test_cleanup(
            Duration::from_millis(10),
            macos::CleanupHooks {
                entered: Some(entered_sender),
                resume: Some(resume_receiver),
                completed: Some(completed_sender),
            },
        )
        .unwrap();
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(outcome.is_err());
    assert!(resource.is_dir());
    resume.send(()).unwrap();
    completed.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert!(!resource.exists());
}

// 보호 파일·조상·workspace root의 관측 가능한 교체와 늦게 생긴 endpoint를 실행 전에 거부한다.
#[test]
fn observed_root_and_exclusion_changes_invalidate_frozen_plan() {
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    let parent = workspace.join("control");
    fs::create_dir_all(&parent).unwrap();
    let secret = parent.join("credential");
    let policy = CommandConfinement::new(&workspace, vec![], vec![secret.clone()], vec![]).unwrap();
    let late = prepare(&policy, &call("printf ok"), false);
    fs::write(&secret, "late").unwrap();
    assert!(late.macos.as_ref().unwrap().verify(&late.roots).is_err());
    let changed = prepare(&policy, &call("printf ok"), false);
    fs::rename(&secret, parent.join("original")).unwrap();
    fs::write(&secret, "replacement").unwrap();
    assert!(
        changed
            .macos
            .as_ref()
            .unwrap()
            .verify(&changed.roots)
            .is_err()
    );
    let changed = prepare(&policy, &call("printf ok"), false);
    fs::rename(&parent, workspace.join("moved-control")).unwrap();
    fs::create_dir(&parent).unwrap();
    assert!(
        changed
            .macos
            .as_ref()
            .unwrap()
            .verify(&changed.roots)
            .is_err()
    );
    let changed = prepare(&policy, &call("printf ok"), false);
    fs::rename(&workspace, root.0.join("moved-workspace")).unwrap();
    fs::create_dir(&workspace).unwrap();
    assert!(
        changed
            .macos
            .as_ref()
            .unwrap()
            .verify(&changed.roots)
            .is_err()
    );
}

// SBPL로 보이는 경로도 정책으로 보간하지 않고 -D의 단일 문자열 값에 고정한다.
#[test]
fn hostile_path_bytes_remain_parameter_data() {
    let root = TestDirectory::new();
    let workspace = root.0.join("quote\") (allow default) ;\nspace");
    fs::create_dir(&workspace).unwrap();
    let policy = CommandConfinement::new(&workspace, vec![], vec![], vec![]).unwrap();
    let plan = prepare(&policy, &call("printf ok"), false);
    let mac = &plan.manifest["macos"];
    assert!(!mac["policy"].as_str().unwrap().contains("(allow default)"));
    assert!(
        mac["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pair| pair[1].as_str() == policy.workspace.to_str())
    );
    plan.macos.as_ref().unwrap().verify(&plan.roots).unwrap();
}

#[cfg(target_os = "macos")]
fn successful(policy: &CommandConfinement, command: &str, approved: bool) {
    let request = call(command);
    let result = super::execute(prepare(policy, &request, approved), &request);
    assert_eq!(
        result.outcome(),
        yo_core::ToolExecutionOutcome::Completed,
        "{}",
        result.output()
    );
}

#[cfg(target_os = "macos")]
fn host_success(command: &mut Command, fixture: &Path) -> bool {
    use std::{
        os::unix::process::CommandExt,
        thread,
        time::{Duration, Instant},
    };

    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    command
        .current_dir(fixture)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", fixture)
        .env("TMPDIR", fixture)
        .env("LANG", "en_US.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                thread::sleep(Duration::from_millis(10))
            },
            result => {
                let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
                let _ = child.kill();
                let _ = child.wait();
                panic!("native fixture host command exceeded its bound: {result:?}");
            },
        }
    }
}

#[cfg(target_os = "macos")]
// 실제 deny-default 자식은 workspace/private HOME에 쓰되 비밀·alias·외부 쓰기와 host IPC를
// 거부한다.
#[test]
#[ignore = "실제 Mac deny-default Seatbelt 파일·IPC·환경 qualification"]
fn macos_qualification_enforces_files_environment_and_host_ipc() {
    use std::{
        net::TcpListener,
        os::{
            fd::AsRawFd,
            unix::{
                fs::symlink,
                net::{UnixDatagram, UnixListener},
            },
        },
    };

    use nix::{
        fcntl::{FcntlArg, fcntl},
        sys::stat::fstat,
        unistd::{close, dup},
    };
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    let state = workspace.join("state");
    fs::create_dir_all(&state).unwrap();
    let token = state.join("token");
    fs::write(&token, "fixture-secret").unwrap();
    let outside = root.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("read"), "fixture-read").unwrap();
    let stream = workspace.join("agent.sock");
    let datagram = workspace.join("agent-dgram.sock");
    let listener = UnixListener::bind(&stream).unwrap();
    let _datagram = UnixDatagram::bind(&datagram).unwrap();
    let tcp = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let tcp6 = TcpListener::bind(("::1", 0)).unwrap();
    let inherited_file = dup(fs::File::open(&token).unwrap()).unwrap();
    let inherited_socket = dup(&listener).unwrap();
    struct HighFd(i32);
    impl Drop for HighFd {
        fn drop(&mut self) {
            let _ = close(self.0);
        }
    }
    let high_file = HighFd(fcntl(&inherited_file, FcntlArg::F_DUPFD(128)).unwrap());
    let socket_stat = fstat(&inherited_socket).unwrap();
    let file_stat = fs::metadata(&token).unwrap();
    let file_identity = format!("({}, {})", file_stat.dev(), file_stat.ino());
    let socket_identity = format!("({}, {})", socket_stat.st_dev, socket_stat.st_ino);
    symlink(&state, workspace.join("alias")).unwrap();
    let policy = CommandConfinement::new(
        &workspace,
        vec![state.clone()],
        vec![stream.clone(), datagram.clone()],
        vec![(outside.clone(), outside.clone())],
    )
    .unwrap();
    let script = format!(
        r#"
import os,pathlib,socket
state={state:?};outside={outside:?}
for path in [state+'/token','alias/token']:
 try:pathlib.Path(path).read_text()
 except OSError:pass
 else:raise AssertionError('secret readable')
for path in [state+'/token',outside+'/write']:
 try:pathlib.Path(path).write_text('forbidden')
 except OSError:pass
 else:raise AssertionError('forbidden write')
try:os.rename(state,state+'-moved')
except OSError:pass
else:raise AssertionError('protected directory renamed')
assert pathlib.Path(outside+'/read').read_text()=='fixture-read'
for kind,path in [(socket.SOCK_STREAM,{stream:?}),(socket.SOCK_DGRAM,{datagram:?})]:
 try:
  s=socket.socket(socket.AF_UNIX,kind);s.settimeout(1)
  if kind==socket.SOCK_DGRAM:s.sendto(b'forbidden',path)
  else:s.connect(path)
 except OSError:pass
 else:raise AssertionError('host endpoint reachable')
 finally:
  if 's' in locals():s.close()
for family,address in [(socket.AF_INET,('127.0.0.1',{port})),(socket.AF_INET6,('::1',{port6}))]:
 try:s=socket.socket(family);s.settimeout(1);s.connect(address)
 except OSError:pass
 else:raise AssertionError('live network endpoint reachable')
 finally:
  if 's' in locals():s.close()
assert set(os.environ)<=set(['PATH','HOME','TMPDIR','LANG','CARGO_HOME','RUSTUP_HOME','RUSTUP_AUTO_INSTALL','GIT_AUTHOR_NAME','GIT_COMMITTER_NAME','GIT_AUTHOR_EMAIL','GIT_COMMITTER_EMAIL','PWD','SHLVL','_','LC_CTYPE'])
assert os.environ['HOME'].startswith('/private/tmp/yo-command-')
assert os.environ['TMPDIR'].startswith('/private/tmp/yo-command-')
assert os.environ['CARGO_HOME']==os.environ['HOME']+'/.cargo'
for key in ['HOME','TMPDIR']:pathlib.Path(os.environ[key],'ordinary-cache').write_text('ok')
for fd in [{file_fd},{socket_fd},{high_fd}]:
 try:m=os.fstat(fd)
 except OSError:continue
 assert (m.st_dev,m.st_ino) not in [{file_identity},{socket_identity}], 'inherited descriptor leaked'
pathlib.Path('ordinary-write').write_text('ok')
"#,
        state = state.to_str().unwrap(),
        outside = outside.to_str().unwrap(),
        stream = stream.to_str().unwrap(),
        datagram = datagram.to_str().unwrap(),
        port = tcp.local_addr().unwrap().port(),
        port6 = tcp6.local_addr().unwrap().port(),
        file_fd = inherited_file.as_raw_fd(),
        socket_fd = inherited_socket.as_raw_fd(),
        high_fd = high_file.0,
        file_identity = file_identity,
        socket_identity = socket_identity
    );
    successful(&policy, &super::python(&script), false);
    assert_eq!(fs::read_to_string(&token).unwrap(), "fixture-secret");
    assert!(!outside.join("write").exists());
    assert_eq!(
        fs::read_to_string(workspace.join("ordinary-write")).unwrap(),
        "ok"
    );
}

#[cfg(target_os = "macos")]
// 계획 뒤 root/endpoint가 교체되면 명령 시작 전에 실패하고 marker를 만들지 않는다.
#[test]
#[ignore = "실제 Mac 이름 재검증과 실행 전 실패 qualification"]
fn macos_qualification_rejects_changed_names_before_spawn() {
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let secret = workspace.join("token");
    fs::write(&secret, "secret").unwrap();
    let policy = CommandConfinement::new(&workspace, vec![], vec![secret.clone()], vec![]).unwrap();
    let request = call("printf forbidden > executed");
    let plan = prepare(&policy, &request, false);
    fs::rename(&secret, workspace.join("original-token")).unwrap();
    fs::write(&secret, "replaced").unwrap();
    let result = super::execute(plan, &request);
    assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Failed);
    assert!(!workspace.join("executed").exists());
    let plan = prepare(&policy, &request, false);
    fs::rename(&workspace, root.0.join("moved-workspace")).unwrap();
    fs::create_dir(&workspace).unwrap();
    assert_eq!(
        super::execute(plan, &request).outcome(),
        yo_core::ToolExecutionOutcome::Failed
    );
    assert!(!workspace.join("executed").exists());
}

#[cfg(target_os = "macos")]
// Cargo offline dependency와 자동 Git은 일반 HOME/credential 없이 읽기 전용 source cache로
// 동작한다.
#[test]
#[ignore = "실제 Mac Cargo·Git 및 private cache qualification"]
fn macos_qualification_runs_ordinary_cargo_and_git() {
    let root = TestDirectory::new();
    fs::create_dir(root.0.join("src")).unwrap();
    fs::write(root.0.join("Cargo.toml"), "[package]\nname='yo_mac_qualification'\nversion='0.0.0'\nedition='2024'\n[dependencies]\nserde='=1.0.229'\n").unwrap();
    fs::write(
        root.0.join("src/lib.rs"),
        "#[test] fn ordinary() { assert_eq!(2 + 2, 4); }\n",
    )
    .unwrap();
    let policy = CommandConfinement::from_environment(&root.0, vec![]).unwrap();
    successful(
        &policy,
        "CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 cargo test --offline",
        false,
    );
    successful(
        &policy,
        "git init && git add Cargo.toml src/lib.rs && git -c user.name=Fixture -c user.email=fixture@example.invalid commit -m fixture && git diff --cached --exit-code && git rev-parse --verify HEAD",
        false,
    );
    assert!(root.0.join(".git/index").is_file());
    assert!(
        root.0
            .join(".git/refs/heads")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}

#[cfg(target_os = "macos")]
// 원래 group의 출력 보유 자식을 취소하고 bounded reap/drain 뒤 private 자원을 제거한다.
#[test]
#[ignore = "실제 Mac process-group 취소·출력 drain qualification"]
fn macos_qualification_cancels_original_group_and_releases_resources() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use yo_core::{ToolExecution, ToolExecutionPoll};
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let request = call(&super::python(
        "import os,time,pathlib,signal\nchild=os.fork()\nsignal.alarm(20)\nif child==0:\n while True:pathlib.Path('heartbeat').write_text(str(time.monotonic()));time.sleep(.02)\nelse:\n while True:time.sleep(1)",
    ));
    let plan = prepare(&policy, &request, false);
    let resource = private_root(&plan).to_owned();
    let mut execution =
        super::super::super::CommandExecution::spawn_confined(plan, &request).unwrap();
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
    let result = execution.take_result().expect("bounded group cancellation");
    assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Interrupted);
    execution.shutdown().unwrap();
    let heartbeat = fs::read(root.0.join("heartbeat")).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fs::read(root.0.join("heartbeat")).unwrap(), heartbeat);
    assert!(!resource.exists());
}

#[cfg(target_os = "macos")]
// spawn 전 취소와 만료 기한은 marker를 만들지 않고 exec 오류 pipe도 정상 보고한다.
#[test]
#[ignore = "실제 Mac 취소·기한·exec-error pipe qualification"]
fn macos_qualification_preserves_prespawn_bounds_and_exec_failure() {
    use std::{
        io::ErrorKind,
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    let root = TestDirectory::new();
    let policy = CommandConfinement::new(&root.0, vec![], vec![], vec![]).unwrap();
    let request = call("printf forbidden > executed");
    let plan = prepare(&policy, &request, false);
    assert!(plan.launch(&AtomicBool::new(true), None).is_err());
    assert!(
        plan.launch(
            &AtomicBool::new(false),
            Some(Instant::now() - Duration::from_secs(1))
        )
        .is_err()
    );
    assert!(!root.0.join("executed").exists());
    assert_eq!(
        super::super::macos::test_exec_failure(&root.0.join("absent-executable")).kind(),
        ErrorKind::NotFound
    );
}

#[cfg(target_os = "macos")]
// 실제 존재하는 credential Mach 이름을 거부하며 새 group으로 spawn한 자식도 파일·Mach 정책을
// 상속한다.
#[test]
#[ignore = "실제 Mac credential Mach·taskport·posix_spawn 정책 상속 qualification"]
fn macos_qualification_denies_mach_and_preserves_policy_after_group_escape() {
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let secret = workspace.join("secret");
    fs::write(&secret, "fixture-secret").unwrap();
    let outside = root.0.join("outside");
    fs::write(&outside, "retain").unwrap();
    let source = workspace.join("probe.c");
    fs::write(
        &source,
        r#"
#include <errno.h>
#include <fcntl.h>
#include <mach/mach.h>
#include <servers/bootstrap.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;
static int denied_open(const char *path, int flags) {
  int fd = open(path, flags, 0600);
  if (fd >= 0) { close(fd); return 0; }
  return errno == EACCES || errno == EPERM;
}
int main(int argc, char **argv) {
  alarm(5);
  if (argc < 3) return 2;
  mach_port_t port = MACH_PORT_NULL;
  if (!strcmp(argv[1], "host-task")) {
    kern_return_t result = task_for_pid(mach_task_self(), (pid_t)strtol(argv[2], 0, 10), &port);
    if (result == KERN_SUCCESS) mach_port_deallocate(mach_task_self(), port);
    return result == KERN_SUCCESS ? 0 : 18;
  }
  kern_return_t found = bootstrap_look_up(bootstrap_port, argv[2], &port);
  if (!strcmp(argv[1], "host")) {
    if (found == KERN_SUCCESS) mach_port_deallocate(mach_task_self(), port);
    return found == KERN_SUCCESS ? 0 : 3;
  }
  if (found == KERN_SUCCESS) { mach_port_deallocate(mach_task_self(), port); return 10; }
  if (argc != 7) return 4;
  if (!strcmp(argv[1], "child")) {
    if (getpgrp() == (pid_t)strtol(argv[6], 0, 10)) return 11;
    if (!denied_open(argv[3], O_RDONLY) || !denied_open(argv[4], O_WRONLY)) return 12;
    port = MACH_PORT_NULL;
    if (task_for_pid(mach_task_self(), (pid_t)strtol(argv[5], 0, 10), &port) == KERN_SUCCESS) {
      mach_port_deallocate(mach_task_self(), port); return 13;
    }
    return 0;
  }
  posix_spawnattr_t attr;
  if (posix_spawnattr_init(&attr) || posix_spawnattr_setpgroup(&attr, 0) ||
      posix_spawnattr_setflags(&attr, POSIX_SPAWN_SETPGROUP)) return 14;
  char group[32]; snprintf(group, sizeof(group), "%d", (int)getpgrp());
  char *child[] = {argv[0], "child", argv[2], argv[3], argv[4], argv[5], group, NULL};
  pid_t pid;
  int result = posix_spawn(&pid, argv[0], NULL, &attr, child, environ);
  posix_spawnattr_destroy(&attr);
  if (result) return 15;
  int status;
  if (waitpid(pid, &status, 0) != pid) return 16;
  return WIFEXITED(status) ? WEXITSTATUS(status) : 17;
}
"#,
    )
    .unwrap();
    let probe = workspace.join("probe");
    assert!(host_success(
        Command::new("/usr/bin/cc")
            .args(["-Wall", "-Wextra", "-Werror"])
            .arg(&source)
            .arg("-o")
            .arg(&probe),
        &workspace,
    ));
    let names = [
        "com.apple.SecurityServer",
        "com.apple.securityd",
        "com.apple.trustd.agent",
    ];
    let service = names
        .into_iter()
        .find(|name| host_success(Command::new(&probe).args(["host", name]), &workspace))
        .expect("native qualification requires a live credential Mach service control");
    assert!(
        host_success(
            Command::new(&probe).args(["host-task", &process::id().to_string()]),
            &workspace
        ),
        "native task-port qualification unavailable: owned target requires debugger access"
    );
    let policy = CommandConfinement::new(&workspace, vec![], vec![secret.clone()], vec![]).unwrap();
    successful(
        &policy,
        &format!(
            "./probe spawn {service} {} {} {} unused",
            secret.display(),
            outside.display(),
            process::id()
        ),
        false,
    );
    assert_eq!(fs::read_to_string(&secret).unwrap(), "fixture-secret");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "retain");
}

#[cfg(target_os = "macos")]
// 승인한 정확한 외부 파일만 쓰고 보호 inode alias와 인접 파일은 제공하지 않는다.
#[test]
#[ignore = "실제 Mac exact file grant·configured artifact qualification"]
fn macos_qualification_scopes_external_grants_and_configured_artifacts() {
    use crate::execution::tools::command::PreparedCommandTools;
    let root = TestDirectory::new();
    let workspace = root.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let granted = root.0.join("granted");
    let neighbor = root.0.join("neighbor");
    let secret = root.0.join("secret");
    fs::write(&granted, "before").unwrap();
    fs::write(&neighbor, "retain").unwrap();
    fs::write(&secret, "secret").unwrap();
    let alias = root.0.join("credential-alias");
    fs::hard_link(&secret, &alias).unwrap();
    let policy = CommandConfinement::new(&workspace, vec![], vec![secret], vec![]).unwrap();
    assert!(
        policy
            .plan_on(
                &call(&format!("printf forbidden > {}", alias.display())),
                None,
                Platform::Macos
            )
            .is_err()
    );
    successful(
        &policy,
        &format!(
            "printf after > {}; test ! -r {}",
            granted.display(),
            neighbor.display()
        ),
        true,
    );
    assert_eq!(fs::read_to_string(&granted).unwrap(), "after");
    let script = root.0.join("configured.sh");
    fs::write(&script, "cat; printf configured > configured-output\n").unwrap();
    let configuration = root.0.join("config.json");
    fs::write(&configuration, serde_json::json!({"tools":{"commands":[{"id":"configured","name":"configured","description":"Execute fixture","executable":"/bin/sh","script":script,"parameters":{"type":"object","properties":{},"additionalProperties":false}}]}}).to_string()).unwrap();
    let config = crate::state::config::load_from(&configuration).unwrap();
    let tools = PreparedCommandTools::prepare_revision(
        super::LocalToolRegistryRevision::CommandToolsV2,
        config.command_tools(),
        &workspace,
        &configuration,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let mut request = call("unused");
    request.call = tools
        .registry()
        .validate_call("configured-call", "configured", "{}", 4096)
        .unwrap();
    let configured = tools
        .command(&yo_core::ToolId::new("configured").unwrap())
        .unwrap();
    let (plan, approval, _, _) = policy
        .plan_on(&request, Some(configured), Platform::Macos)
        .unwrap();
    assert!(approval);
    let result = super::execute(plan, &request);
    assert_eq!(
        result.outcome(),
        yo_core::ToolExecutionOutcome::Completed,
        "{}",
        result.output()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("configured-output")).unwrap(),
        "configured"
    );
}

#[cfg(target_os = "macos")]
// linked worktree metadata는 자동 읽기만 허용하고 git add의 정확한 외부 root만 승인한다.
#[test]
#[ignore = "실제 Mac linked Git metadata 승인 qualification"]
fn macos_qualification_preserves_external_git_metadata_approval() {
    let root = TestDirectory::new();
    let repository = root.0.join("repository");
    fs::create_dir(&repository).unwrap();
    let workspace = root.0.join("worktree");
    let git = |args: &[&str]| {
        assert!(host_success(
            Command::new("/usr/bin/git").args(args),
            &repository,
        ));
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
    let index = super::super::git_metadata_roots(&workspace).unwrap()[0].join("index");
    let original_index = fs::read(&index).unwrap();
    fs::write(workspace.join("opaque.sh"), "git add new\n").unwrap();
    let opaque = call("/bin/sh opaque.sh");
    assert_eq!(
        super::execute(prepare(&policy, &opaque, false), &opaque).outcome(),
        yo_core::ToolExecutionOutcome::Failed
    );
    assert_eq!(fs::read(&index).unwrap(), original_index);
    let request = call("git add new");
    let (plan, approval, scope, _) = policy.plan_on(&request, None, Platform::Macos).unwrap();
    assert!(approval);
    assert!(scope.contains(repository.to_str().unwrap()));
    assert_eq!(
        super::execute(plan, &request).outcome(),
        yo_core::ToolExecutionOutcome::Completed
    );
    assert_ne!(fs::read(index).unwrap(), original_index);
}
