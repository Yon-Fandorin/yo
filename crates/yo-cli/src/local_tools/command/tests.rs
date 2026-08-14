use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nix::{
    errno::Errno,
    sys::{
        signal::kill,
        wait::{WaitPidFlag, waitpid},
    },
    unistd::Pid,
};
use yo_core::{ToolExecution, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult};

use super::{
    CommandExecution,
    limits::{CommandExecutionLimits, OUTPUT_INACTIVITY_TIMEOUT},
};

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
    CommandExecution::spawn_with_limits(
        PathBuf::from("/tmp"),
        command.to_owned(),
        64 * 1024,
        limits,
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

fn read_published_pids(path: &PathBuf) -> Vec<i32> {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "command did not publish its PIDs"
        );
        thread::sleep(Duration::from_millis(5));
    }
    fs::read_to_string(path)
        .unwrap()
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
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
    let pid = read_published_pids(&pid_path)[0];

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
    let pids = read_published_pids(&pid_path);

    let result = finish(&mut execution);
    let leader_probe = waitpid(Pid::from_raw(pids[0]), Some(WaitPidFlag::WNOHANG));
    assert_process_disappears(pids[1]);
    let _ = fs::remove_file(pid_path);

    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(leader_probe, Err(Errno::ECHILD));
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
    let pids = read_published_pids(&pid_path);

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
