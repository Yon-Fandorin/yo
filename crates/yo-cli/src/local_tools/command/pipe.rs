use std::{
    collections::VecDeque,
    io::Read,
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream,
    },
    sync::mpsc::{SyncSender, TrySendError},
    thread::{self, JoinHandle},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PipeKind {
    Stdout,
    Stderr,
}

pub(super) struct PipeDrain {
    pub(super) kind: PipeKind,
    pub(super) output: BoundedPipeOutput,
    pub(super) failed: bool,
}

impl PipeDrain {
    pub(super) fn truncated(&self) -> bool {
        self.failed || self.output.truncated()
    }
}

pub(super) struct PipeReader {
    shutdown: Option<UnixStream>,
    thread: Option<JoinHandle<()>>,
}

impl PipeReader {
    pub(super) fn request_shutdown(&mut self) {
        drop(self.shutdown.take());
    }

    pub(super) fn join(&mut self) -> Result<(), ()> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        let result = thread.join().map_err(|_| ());
        drop(self.shutdown.take());
        result
    }

    pub(super) fn shutdown_and_join(&mut self) -> Result<(), ()> {
        self.request_shutdown();
        self.join()
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        let _ = self.shutdown_and_join();
    }
}

pub(super) fn spawn_pipe_reader(
    kind: PipeKind,
    mut reader: impl Read + AsRawFd + Send + 'static,
    limit: usize,
    progress_sender: SyncSender<()>,
    drain_sender: SyncSender<PipeDrain>,
) -> Result<PipeReader, ()> {
    let thread_name = match kind {
        PipeKind::Stdout => "yo-command-stdout",
        PipeKind::Stderr => "yo-command-stderr",
    };
    let (shutdown_reader, shutdown_writer) = UnixStream::pair().map_err(|_| ())?;
    let thread = thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            let mut output = BoundedPipeOutput::new(limit);
            let mut chunk = [0_u8; 8 * 1024];
            let failed = loop {
                match wait_for_input_or_shutdown(reader.as_raw_fd(), shutdown_reader.as_raw_fd()) {
                    Ok(true) => {},
                    Ok(false) => break false,
                    Err(()) => break true,
                }
                match reader.read(&mut chunk) {
                    Ok(0) => break false,
                    Ok(count) => {
                        output.push(&chunk[..count]);
                        match progress_sender.try_send(()) {
                            Ok(()) | Err(TrySendError::Full(())) => {},
                            Err(TrySendError::Disconnected(())) => {},
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                    Err(_) => break true,
                }
            };
            let _ = drain_sender.send(PipeDrain {
                kind,
                output,
                failed,
            });
        })
        .map_err(|_| ())?;
    Ok(PipeReader {
        shutdown: Some(shutdown_writer),
        thread: Some(thread),
    })
}

// `poll` waits for ordinary pipe readiness without periodic wakeups. Dropping the
// peer control socket makes cleanup readiness immediate even when a foreign writer
// keeps the command pipe open indefinitely.
#[allow(unsafe_code)]
fn wait_for_input_or_shutdown(reader: RawFd, shutdown: RawFd) -> Result<bool, ()> {
    loop {
        let mut descriptors = [
            libc::pollfd {
                fd: reader,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: shutdown,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: both descriptors remain owned for this call, the array length is
        // exact, and `poll` writes only the two `revents` fields before returning.
        let result = unsafe {
            libc::poll(
                descriptors.as_mut_ptr(),
                descriptors.len() as libc::nfds_t,
                -1,
            )
        };
        if result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(());
        }
        let shutdown_events = descriptors[1].revents;
        if shutdown_events & libc::POLLNVAL != 0 {
            return Err(());
        }
        if shutdown_events & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return Ok(false);
        }
        let pipe_events = descriptors[0].revents;
        if pipe_events & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(true);
        }
        if pipe_events & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(());
        }
    }
}

pub(super) struct BoundedPipeOutput {
    limit: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u128,
}

impl BoundedPipeOutput {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            head: Vec::with_capacity(limit.div_ceil(2)),
            tail: VecDeque::with_capacity(limit / 2),
            total: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u128);
        let head_capacity = self.limit.div_ceil(2);
        let head_count = (head_capacity - self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_count]);

        let tail_capacity = self.limit / 2;
        if tail_capacity == 0 {
            return;
        }
        let remaining = &bytes[head_count..];
        if remaining.len() >= tail_capacity {
            self.tail.clear();
            self.tail
                .extend(&remaining[remaining.len() - tail_capacity..]);
            return;
        }
        let overflow = self
            .tail
            .len()
            .saturating_add(remaining.len())
            .saturating_sub(tail_capacity);
        self.tail.drain(..overflow);
        self.tail.extend(remaining);
    }

    pub(super) fn truncated(&self) -> bool {
        self.total > self.limit as u128
    }

    pub(super) fn render(self) -> Vec<u8> {
        if !self.truncated() {
            let mut output = self.head;
            output.extend(self.tail);
            return output;
        }

        let mut omitted = self.total.saturating_sub(self.limit as u128);
        let (marker, retained) = loop {
            let marker = format!("\n[yo: {omitted} bytes omitted]\n").into_bytes();
            let retained = self.limit.saturating_sub(marker.len());
            let exact_omitted = self.total.saturating_sub(retained as u128);
            if exact_omitted == omitted {
                break (marker, retained);
            }
            omitted = exact_omitted;
        };
        if retained == 0 {
            return marker.into_iter().take(self.limit).collect();
        }

        let head_count = retained.div_ceil(2).min(self.head.len());
        let tail_count = (retained - head_count).min(self.tail.len());
        let mut output = Vec::with_capacity(head_count + marker.len() + tail_count);
        output.extend_from_slice(&self.head[..head_count]);
        output.extend_from_slice(&marker);
        output.extend(self.tail.iter().skip(self.tail.len() - tail_count));
        output
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::{self, Read, Write},
        os::fd::{AsRawFd, RawFd},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use nix::unistd::pipe;

    use super::{BoundedPipeOutput, PipeKind, spawn_pipe_reader};

    struct DropObservedReader {
        reader: File,
        dropped: Arc<AtomicBool>,
    }

    struct ErrorAfterPrefixReader {
        readiness: File,
        returned_prefix: bool,
    }

    impl Read for DropObservedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.reader.read(buffer)
        }
    }

    impl AsRawFd for DropObservedReader {
        fn as_raw_fd(&self) -> RawFd {
            self.reader.as_raw_fd()
        }
    }

    impl Drop for DropObservedReader {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    impl Read for ErrorAfterPrefixReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.returned_prefix {
                return Err(io::Error::other("injected pipe read failure"));
            }
            self.returned_prefix = true;
            buffer[..6].copy_from_slice(b"prefix");
            Ok(6)
        }
    }

    impl AsRawFd for ErrorAfterPrefixReader {
        fn as_raw_fd(&self) -> RawFd {
            self.readiness.as_raw_fd()
        }
    }

    // writer가 열린 채 EOF를 주지 않아도 cleanup 신호가 reader의 poll을 깨우고 bounded
    // 시간 안에 thread와 local read descriptor를 닫는지 검증합니다.
    #[test]
    fn shutdown_releases_a_reader_while_its_writer_remains_open() {
        let (reader, writer) = pipe().unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let reader = DropObservedReader {
            reader: File::from(reader),
            dropped: Arc::clone(&dropped),
        };
        let mut writer = File::from(writer);
        let (progress_sender, progress_receiver) = mpsc::sync_channel(1);
        let (drain_sender, drain_receiver) = mpsc::sync_channel(1);
        let mut pipe_reader =
            spawn_pipe_reader(PipeKind::Stdout, reader, 32, progress_sender, drain_sender).unwrap();
        writer.write_all(b"prefix").unwrap();
        progress_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let started = Instant::now();

        pipe_reader.shutdown_and_join().unwrap();
        let drain = drain_receiver.recv_timeout(Duration::from_secs(1)).unwrap();

        assert_eq!(drain.output.render(), b"prefix");
        assert!(!drain.failed);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(dropped.load(Ordering::Acquire));
    }

    // 일부 bytes를 읽은 뒤 pipe read가 실패하면 보존 cap을 넘지 않았더라도 출력의
    // 완전성을 증명할 수 없으므로 drain이 실패와 truncation을 함께 표시하는지 검증합니다.
    #[test]
    fn read_failure_marks_the_partial_drain_as_truncated() {
        let (readiness, writer) = pipe().unwrap();
        let mut writer = File::from(writer);
        writer.write_all(b"ready").unwrap();
        let reader = ErrorAfterPrefixReader {
            readiness: File::from(readiness),
            returned_prefix: false,
        };
        let (progress_sender, _progress_receiver) = mpsc::sync_channel(1);
        let (drain_sender, drain_receiver) = mpsc::sync_channel(1);
        let mut pipe_reader =
            spawn_pipe_reader(PipeKind::Stdout, reader, 32, progress_sender, drain_sender).unwrap();

        let drain = drain_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        pipe_reader.join().unwrap();

        assert!(drain.failed);
        assert!(drain.truncated());
        assert_eq!(drain.output.render(), b"prefix");
    }

    // 긴 byte stream을 여러 chunk로 넣어도 exact head·tail과 실제 생략 byte 수를
    // 고정 크기 안에 함께 보존하는지 검증합니다.
    #[test]
    fn bounded_output_retains_head_tail_and_exact_omitted_byte_count() {
        let input: Vec<u8> = (0_u8..200).collect();
        let mut output = BoundedPipeOutput::new(80);

        for chunk in input.chunks(7) {
            output.push(chunk);
        }
        let rendered = output.render();
        let marker_start = rendered
            .windows(b"\n[yo: ".len())
            .position(|window| window == b"\n[yo: ")
            .expect("truncated output must contain an omission marker");
        let marker_end = rendered
            .windows(b" bytes omitted]\n".len())
            .position(|window| window == b" bytes omitted]\n")
            .map(|index| index + b" bytes omitted]\n".len())
            .expect("omission marker must be complete");
        let omitted: usize = std::str::from_utf8(
            &rendered[marker_start + b"\n[yo: ".len()..marker_end - b" bytes omitted]\n".len()],
        )
        .unwrap()
        .parse()
        .unwrap();

        assert_eq!(rendered.len(), 80);
        assert_eq!(&rendered[..marker_start], &input[..marker_start]);
        assert_eq!(
            &rendered[marker_end..],
            &input[input.len() - (80 - marker_end)..]
        );
        assert_eq!(omitted, input.len() - marker_start - (80 - marker_end));
    }
}
