use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nix::{
    errno::Errno,
    sys::{
        signal::{Signal, kill},
        wait::{WaitPidFlag, waitpid},
    },
    unistd::{Pid, setsid},
};
use yo_core::{ToolExecution, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult};

use super::{
    CommandExecution, ProcessGroupLease,
    limits::{CommandExecutionLimits, OUTPUT_INACTIVITY_TIMEOUT},
    process::WaiterTestHooks,
    shell_command,
};

// local command가 사용자 로그인 profile을 읽어 PATH·stderr·부작용을 바꾸지 않도록
// 비로그인 `/bin/sh -c` argv만 구성하는지 고정합니다.
#[test]
fn command_shell_does_not_load_a_login_profile() {
    let command = shell_command(std::path::Path::new("/tmp"), "printf exact");
    let arguments: Vec<_> = command.get_args().collect();

    assert_eq!(
        arguments,
        [
            std::ffi::OsStr::new("-c"),
            std::ffi::OsStr::new("printf exact")
        ]
    );
}

// reap이 WNOWAIT identity pin을 해제한 뒤에는 같은 숫자 process-group ID가 재사용될 수
// 있으므로, clear된 lease가 이후 signal을 보내지 않는지 검증합니다.
#[test]
fn cleared_process_group_lease_cannot_signal_a_reused_identity() {
    let shared = std::sync::Mutex::new(None);
    let process_group = Pid::from_raw(42);
    let signals = AtomicUsize::new(0);
    let mut lease = ProcessGroupLease::publish(&shared, process_group).unwrap();

    lease.terminate_with(|observed| {
        assert_eq!(observed, process_group);
        signals.fetch_add(1, Ordering::Relaxed);
    });
    lease.clear().unwrap();
    lease.terminate_with(|_| {
        signals.fetch_add(1, Ordering::Relaxed);
    });

    assert_eq!(signals.load(Ordering::Relaxed), 1);
    assert_eq!(*shared.lock().unwrap(), None);
}

fn limits(
    output_inactivity_timeout: Duration,
    absolute_execution_timeout: Option<Duration>,
) -> CommandExecutionLimits {
    CommandExecutionLimits {
        output_inactivity_timeout,
        absolute_execution_timeout,
        cleanup_grace: Duration::from_secs(1),
    }
}

fn spawn(command: &str, limits: CommandExecutionLimits) -> CommandExecution {
    spawn_with_output_limit(command, 64 * 1024, limits)
}

fn spawn_with_output_limit(
    command: &str,
    maximum_output_bytes: usize,
    limits: CommandExecutionLimits,
) -> CommandExecution {
    CommandExecution::spawn_with_limits(
        PathBuf::from("/tmp"),
        command.to_owned(),
        maximum_output_bytes,
        limits,
    )
    .unwrap()
}

fn spawn_with_waiter_hooks(
    command: &str,
    maximum_output_bytes: usize,
    limits: CommandExecutionLimits,
    waiter_hooks: WaiterTestHooks,
) -> CommandExecution {
    CommandExecution::spawn_with_limits_and_hooks(
        PathBuf::from("/tmp"),
        command.to_owned(),
        maximum_output_bytes,
        limits,
        waiter_hooks,
    )
    .unwrap()
}

fn finish(execution: &mut CommandExecution) -> ToolExecutionResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if execution.poll().unwrap() == ToolExecutionPoll::Ready {
            let result = execution.take_result().unwrap();
            execution.shutdown().unwrap();
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "command fixture exceeded its outer test bound"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn unique_pid_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(format!(
        "/tmp/yo-command-{label}-{}-{nonce}.pid",
        std::process::id()
    ))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn read_published_pids(path: &PathBuf, expected_count: usize) -> Vec<i32> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Ok(contents) = fs::read_to_string(path) {
            let parsed = contents
                .split_whitespace()
                .map(str::parse::<i32>)
                .collect::<Result<Vec<_>, _>>();
            if let Ok(pids) = parsed
                && pids.len() == expected_count
            {
                return pids;
            }
        }
        assert!(
            Instant::now() < deadline,
            "command did not publish {expected_count} complete PIDs"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_process_disappears(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match kill(Pid::from_raw(pid), None) {
            Err(Errno::ESRCH) => return,
            Ok(()) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            result => panic!("process {pid} remained after cleanup: {result:?}"),
        }
    }
}

// 별도 test process가 새 Session/process group으로 이동한 뒤 stdout pipe를 계속 쓰며,
// run_command cleanup이 local read end를 닫았을 때만 marker를 게시합니다.
#[test]
fn detached_pipe_holder_helper() {
    let Some(pid_path) = std::env::var_os("YO_COMMAND_DETACHED_PIPE_PID") else {
        return;
    };
    let closed_path = std::env::var_os("YO_COMMAND_DETACHED_PIPE_CLOSED").unwrap();
    setsid().unwrap();
    fs::write(&pid_path, std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut stdout = std::io::stdout().lock();
    while Instant::now() < deadline {
        if std::io::Write::write_all(&mut stdout, b"x")
            .and_then(|()| std::io::Write::flush(&mut stdout))
            .is_err()
        {
            fs::write(closed_path, "closed").unwrap();
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    fs::write(closed_path, "open").unwrap();
}

// 기본 local command policy는 stdout/stderr progress inactivity만 5분으로 제한하고,
// agent-owned absolute execution deadline은 설정하지 않는지 고정합니다.
#[test]
fn default_command_policy_has_five_minute_inactivity_without_absolute_deadline() {
    let limits = CommandExecutionLimits::for_agent(None);

    assert_eq!(limits.output_inactivity_timeout, OUTPUT_INACTIVITY_TIMEOUT);
    assert_eq!(
        limits.output_inactivity_timeout,
        Duration::from_secs(5 * 60)
    );
    assert_eq!(limits.absolute_execution_timeout, None);
    assert_eq!(limits.cleanup_grace, Duration::from_secs(1));
}

// 전체 실행 시간이 inactivity window보다 길어도 stdout과 stderr의 non-empty chunk가
// 번갈아 도착하면 각 chunk가 같은 progress clock을 reset하여 정상 완료되는지 관찰합니다.
#[test]
fn non_empty_stdout_and_stderr_chunks_reset_one_inactivity_clock() {
    let mut execution = spawn(
        "printf a; sleep 0.15; printf b >&2; sleep 0.15; printf c; sleep 0.15; printf d >&2",
        limits(Duration::from_millis(350), None),
    );

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(result.output().contains("ac"));
    assert!(result.output().contains("bd"));
}

// 살아 있지만 아무 output도 내지 않는 process는 짧은 test inactivity window에서 종료되고
// cancellation이나 absolute deadline이 아닌 정확한 inactivity diagnostic을 반환합니다.
#[test]
fn silent_process_expires_at_the_output_inactivity_deadline() {
    let started = Instant::now();
    let mut execution = spawn("sleep 5", limits(Duration::from_millis(100), None));

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(
        result.output(),
        "run_command output inactivity deadline expired"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

// inactivity timeout result가 준비된 시점에는 process-group leader가 zombie로 남지 않고
// exact PID가 사라져 finite termination 뒤 reap까지 완료되었는지 관찰합니다.
#[test]
fn inactivity_timeout_reaps_the_exact_process_group_leader() {
    let pid_path = unique_pid_path("timeout");
    let command = format!("printf '%s' $$ > {}; sleep 5", pid_path.display());
    let mut execution = spawn(&command, limits(Duration::from_millis(300), None));
    let pid = read_published_pids(&pid_path, 1)[0];

    let result = finish(&mut execution);
    let probe = waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG));
    let _ = fs::remove_file(pid_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(probe, Err(Errno::ECHILD));
}

// process-group leader가 정상 종료한 뒤 background descendant가 stdout pipe를 열어 둬도
// 즉시 group cleanup을 시작해 descendant를 종료하고 leader를 reap한 뒤 유한하게 완료합니다.
#[test]
fn leader_exit_terminates_a_descendant_that_holds_the_output_pipe() {
    let pid_path = unique_pid_path("held-pipe");
    let command = format!(
        "sleep 5 & printf '%s %s' $$ $! > {}; exit 0",
        pid_path.display()
    );
    let started = Instant::now();
    let mut execution = spawn(&command, limits(Duration::from_secs(3), None));
    let pids = read_published_pids(&pid_path, 2);

    let result = finish(&mut execution);
    let leader_probe = waitpid(Pid::from_raw(pids[0]), Some(WaitPidFlag::WNOHANG));
    assert_process_disappears(pids[1]);
    let _ = fs::remove_file(pid_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(leader_probe, Err(Errno::ECHILD));
}

// command process group을 벗어난 writer가 pipe를 계속 잡아도 cleanup grace 뒤 reader와
// local descriptor가 닫혀 writer가 EPIPE를 보고, Yo result와 thread 정리가 유한한지 검증합니다.
#[test]
fn cleanup_releases_pipe_readers_held_by_an_escaped_writer() {
    let pid_path = unique_pid_path("escaped-writer");
    let closed_path = unique_pid_path("escaped-writer-closed");
    let executable = std::env::current_exe().unwrap();
    let executable = shell_quote(executable.to_str().unwrap());
    let pid_argument = shell_quote(pid_path.to_str().unwrap());
    let closed_argument = shell_quote(closed_path.to_str().unwrap());
    let command = format!(
        "YO_COMMAND_DETACHED_PIPE_PID={pid_argument} YO_COMMAND_DETACHED_PIPE_CLOSED={closed_argument} {executable} --exact local_tools::command::tests::detached_pipe_holder_helper --nocapture --test-threads=1 & while [ ! -s {pid_argument} ]; do sleep 0.01; done; exit 0"
    );
    let mut execution = spawn(
        &command,
        CommandExecutionLimits {
            output_inactivity_timeout: Duration::from_secs(2),
            absolute_execution_timeout: None,
            cleanup_grace: Duration::from_millis(100),
        },
    );
    let escaped_pid = read_published_pids(&pid_path, 1)[0];
    let started = Instant::now();

    let result = finish(&mut execution);
    let close_deadline = Instant::now() + Duration::from_secs(1);
    while !closed_path.exists() {
        if Instant::now() >= close_deadline {
            let _ = kill(Pid::from_raw(escaped_pid), Signal::SIGKILL);
            let _ = fs::remove_file(&pid_path);
            panic!("escaped writer did not observe bounded pipe-reader shutdown");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let close_observation = fs::read_to_string(&closed_path).unwrap();
    assert_process_disappears(escaped_pid);
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(closed_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert_eq!(
        result.output(),
        "run_command cleanup failed after normal process exit"
    );
    assert!(result.truncated());
    assert_eq!(close_observation, "closed");
    assert!(started.elapsed() < Duration::from_secs(1));
}

// background descendant가 계속 stdout progress를 만들어 inactivity를 reset하려 해도
// leader exit가 우선 cleanup을 열어 descendant effect와 pipe drain을 유한하게 끝냅니다.
#[test]
fn leader_exit_terminates_a_descendant_that_keeps_writing_output() {
    let pid_path = unique_pid_path("writing-descendant");
    let command = format!(
        "(while :; do printf x; sleep 0.01; done) & printf '%s %s' $$ $! > {}; exit 0",
        pid_path.display()
    );
    let started = Instant::now();
    let mut execution = spawn(&command, limits(Duration::from_secs(3), None));
    let pids = read_published_pids(&pid_path, 2);

    let result = finish(&mut execution);
    let leader_probe = waitpid(Pid::from_raw(pids[0]), Some(WaitPidFlag::WNOHANG));
    assert_process_disappears(pids[1]);
    let _ = fs::remove_file(pid_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(leader_probe, Err(Errno::ECHILD));
}

// 계속 stdout progress를 내어 inactivity는 연장하더라도 attempt 시작 시 한 번 열린
// agent absolute deadline은 reset되지 않고 별도 diagnostic으로 종료되는지 검증합니다.
#[test]
fn output_progress_does_not_reset_an_absolute_execution_deadline() {
    let mut execution = spawn(
        "while :; do printf x; sleep 0.05; done",
        limits(Duration::from_millis(250), Some(Duration::from_millis(160))),
    );

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(
        result.output(),
        "run_command absolute execution deadline expired"
    );
}

// explicit cancellation은 두 deadline과 다른 cancellation diagnostic을 유지하고 같은
// finite process-group termination과 reap 경로로 worker를 닫는지 검증합니다.
#[test]
fn cancellation_remains_distinct_from_both_deadlines() {
    let mut execution = spawn("sleep 5", limits(Duration::from_secs(1), None));
    execution.cancel();

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(result.output(), "run_command cancelled");
}

// 0인 inactivity, cleanup, explicit absolute 값은 즉시 만료와 무제한 의미로 갈리지 않도록
// process spawn 전에 configuration error로 거절합니다.
#[test]
fn rejects_zero_command_deadlines_before_spawning() {
    for invalid in [
        CommandExecutionLimits {
            output_inactivity_timeout: Duration::ZERO,
            absolute_execution_timeout: None,
            cleanup_grace: Duration::from_secs(1),
        },
        CommandExecutionLimits {
            output_inactivity_timeout: Duration::from_secs(1),
            absolute_execution_timeout: Some(Duration::ZERO),
            cleanup_grace: Duration::from_secs(1),
        },
        CommandExecutionLimits {
            output_inactivity_timeout: Duration::from_secs(1),
            absolute_execution_timeout: None,
            cleanup_grace: Duration::ZERO,
        },
    ] {
        let error = CommandExecution::spawn_with_limits(
            PathBuf::from("/tmp"),
            "printf unreachable".to_owned(),
            1024,
            invalid,
        )
        .err()
        .expect("invalid limits must fail before returning an execution");
        assert!(error.to_string().contains("deadlines must be non-zero"));
    }
}

// output 관찰 한도에 도달해도 child stdout pipe를 닫지 않아, retained cap을 넘게 쓴
// shell이 모든 write 뒤의 effect까지 실행하는지 검증합니다.
#[test]
fn output_cap_does_not_change_a_later_command_effect() {
    let effect_path = unique_pid_path("output-cap-effect");
    let command = format!(
        "i=0; while [ \"$i\" -lt 20000 ]; do printf x; i=$((i + 1)); done; printf done > {}",
        effect_path.display()
    );
    let mut execution =
        spawn_with_output_limit(&command, 512, limits(Duration::from_secs(2), None));

    let result = finish(&mut execution);
    let effect = fs::read_to_string(&effect_path).unwrap();
    let _ = fs::remove_file(effect_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert_eq!(effect, "done");
    assert!(result.truncated());
    assert!(result.output().contains(" bytes omitted]"));
}

// stdout과 stderr reader가 독립적으로 drain하고 bounded head/tail을 유지하여 한 stream의
// flood가 다른 stream을 굶기지 않으며 각 생략 영역이 명시되는지 검증합니다.
#[test]
fn mixed_large_streams_retain_two_head_tail_views() {
    let mut execution = spawn_with_output_limit(
        "i=0; while [ \"$i\" -lt 5000 ]; do printf o; printf e >&2; i=$((i + 1)); done",
        2048,
        limits(Duration::from_secs(2), None),
    );

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(result.truncated());
    assert_eq!(result.output().matches(" bytes omitted]").count(), 2);
    assert!(result.output().contains("stdout:\noooo"));
    assert!(
        result.output().contains("stderr:\neeee"),
        "mixed stream output did not retain the stderr head: {:?}",
        result.output()
    );
}

// EPIPE-aware writer가 관찰 cap 때문에 인위적인 pipe failure를 보지 않고 전체 write 뒤
// `ok`를 기록하는지 검증합니다. 과거 close-at-cap 경로에서는 epipe를 기록했습니다.
#[test]
fn output_cap_is_not_reported_to_an_epipe_aware_writer() {
    let effect_path = unique_pid_path("epipe-aware");
    let command = format!(
        "trap '' PIPE; i=0; while [ \"$i\" -lt 20000 ]; do if ! printf x; then printf epipe > {path}; exit 7; fi; i=$((i + 1)); done; printf ok > {path}",
        path = effect_path.display()
    );
    let mut execution =
        spawn_with_output_limit(&command, 512, limits(Duration::from_secs(2), None));

    let result = finish(&mut execution);
    let effect = fs::read_to_string(&effect_path).unwrap();
    let _ = fs::remove_file(effect_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert_eq!(effect, "ok");
    assert!(result.truncated());
}

// retained byte cap 뒤에 관찰된 chunk도 progress로 계산되어, command가 한 test inactivity
// window보다 오래 실행되어도 pipe drain이 계속되면 완료되는지 검증합니다.
#[test]
fn output_after_the_cap_keeps_resetting_inactivity() {
    let mut execution = spawn_with_output_limit(
        "i=0; while [ \"$i\" -lt 12 ]; do printf 012345678901234567890123456789; sleep 0.03; i=$((i + 1)); done",
        192,
        limits(Duration::from_millis(80), None),
    );

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(result.truncated());
}

// drain된 각 chunk가 inactivity를 reset하더라도 continuous writer는 독립 absolute
// deadline으로 제한되고 termination, drain, result publication이 유한한지 검증합니다.
#[test]
fn continuous_writer_stops_at_the_absolute_deadline_with_bounded_output() {
    let started = Instant::now();
    let mut execution = spawn_with_output_limit(
        "while :; do printf 012345678901234567890123456789; done",
        512,
        limits(Duration::from_millis(100), Some(Duration::from_millis(160))),
    );

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Interrupted);
    assert_eq!(
        result.output(),
        "run_command absolute execution deadline expired"
    );
    assert!(result.truncated());
    assert!(started.elapsed() < Duration::from_secs(2));
}

// bounded result가 exit observation 전에 cleanup failure를 보고하더라도 detached waiter가
// ownership을 유지해 이후 exact child를 한 번만 reap하는지 검증합니다.
#[test]
fn delayed_exit_observation_returns_on_time_and_eventually_reaps_once() {
    let pid_path = unique_pid_path("delayed-waiter");
    let command = format!("printf '%s' $$ > {}; sleep 5", pid_path.display());
    let reap_count = Arc::new(AtomicUsize::new(0));
    let mut execution = spawn_with_waiter_hooks(
        &command,
        1024,
        CommandExecutionLimits {
            output_inactivity_timeout: Duration::from_secs(1),
            absolute_execution_timeout: None,
            cleanup_grace: Duration::from_millis(50),
        },
        WaiterTestHooks {
            observation_delay: Duration::from_millis(200),
            reap_count: Some(Arc::clone(&reap_count)),
        },
    );
    let pid = read_published_pids(&pid_path, 1)[0];
    let started = Instant::now();
    execution.cancel();

    let result = finish(&mut execution);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Failed);
    assert_eq!(
        result.output(),
        "run_command cleanup failed after run_command cancelled"
    );
    assert!(started.elapsed() < Duration::from_millis(180));
    let deadline = Instant::now() + Duration::from_secs(2);
    while reap_count.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline, "waiter did not eventually reap");
        thread::sleep(Duration::from_millis(5));
    }
    let probe = waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG));
    let _ = fs::remove_file(pid_path);

    assert_eq!(reap_count.load(Ordering::Acquire), 1);
    assert_eq!(probe, Err(Errno::ECHILD));
}
