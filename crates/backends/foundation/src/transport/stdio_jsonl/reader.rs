use std::{
    io::{BufRead, BufReader, Read},
    process::{ChildStderr, ChildStdout},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

use serde_json::Value;

use super::config::initialization_failure;

const MESSAGE_QUEUE_CAPACITY: usize = 256;
const STDERR_TAIL_BYTES: usize = 16 * 1024;

pub(super) enum ReaderMessage {
    Value(Value),
    Failed(String),
    Closed,
}

pub(super) fn spawn_stdout_reader(
    stdout: ChildStdout,
    process_name: &'static str,
    thread_name: &'static str,
    maximum_message_bytes: usize,
) -> Result<(Receiver<ReaderMessage>, JoinHandle<()>), crate::BackendFailure> {
    let (sender, receiver) = mpsc::sync_channel(MESSAGE_QUEUE_CAPACITY);
    let reader = thread::Builder::new()
        .name(format!("yo-{}-jsonl-reader", thread_name))
        .spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                match read_jsonl_message(&mut stdout, process_name, maximum_message_bytes) {
                    Ok(Some(value)) => {
                        if sender.send(ReaderMessage::Value(value)).is_err() {
                            return;
                        }
                    },
                    Ok(None) => {
                        let _ = sender.send(ReaderMessage::Closed);
                        return;
                    },
                    Err(error) => {
                        let _ = sender.send(ReaderMessage::Failed(error));
                        return;
                    },
                }
            }
        })
        .map_err(|error| {
            initialization_failure(
                process_name,
                format!("failed to start stdout reader: {error}"),
            )
        })?;

    Ok((receiver, reader))
}

pub(super) fn spawn_stderr_reader(
    stderr: ChildStderr,
    process_name: &'static str,
    thread_name: &'static str,
) -> Result<(Arc<Mutex<String>>, JoinHandle<()>), crate::BackendFailure> {
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let captured_stderr = Arc::clone(&stderr_tail);
    let stderr_reader = thread::Builder::new()
        .name(format!("yo-{}-stderr-reader", thread_name))
        .spawn(move || capture_stderr(stderr, captured_stderr))
        .map_err(|error| {
            initialization_failure(
                process_name,
                format!("failed to start stderr reader: {error}"),
            )
        })?;

    Ok((stderr_tail, stderr_reader))
}

fn capture_stderr(stderr: impl Read, captured: Arc<Mutex<String>>) {
    let mut stderr = BufReader::new(stderr);
    let mut chunk = [0_u8; 4096];
    loop {
        let Ok(read) = stderr.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        let Ok(mut tail) = captured.lock() else {
            return;
        };
        tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
        if tail.len() > STDERR_TAIL_BYTES {
            let mut start = tail.len() - STDERR_TAIL_BYTES;
            while !tail.is_char_boundary(start) {
                start += 1;
            }
            tail.drain(..start);
        }
    }
}

pub(super) fn read_jsonl_message(
    reader: &mut impl BufRead,
    process_name: &str,
    maximum_message_bytes: usize,
) -> Result<Option<Value>, String> {
    loop {
        let mut line = Vec::new();
        let read = {
            let read_limit = maximum_message_bytes
                .checked_add(2)
                .expect("validated maximum JSONL message size must fit usize");
            let mut limited = Read::by_ref(reader).take(read_limit as u64);
            limited
                .read_until(b'\n', &mut line)
                .map_err(|error| format!("failed reading {process_name} stdout: {error}"))?
        };
        if read == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        if line.len() > maximum_message_bytes {
            return Err(format!(
                "{process_name} JSONL message exceeds the {maximum_message_bytes}-byte limit"
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return serde_json::from_slice(&line)
            .map(Some)
            .map_err(|error| format!("invalid JSONL from {process_name}: {error}"));
    }
}
