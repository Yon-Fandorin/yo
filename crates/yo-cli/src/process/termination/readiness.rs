//! Process-host signal readiness bridge.
//!
//! This intentionally remains separate from `yo-core`'s ordinary producer readiness: it owns a
//! process-global async-signal-safe file descriptor, tolerates poisoned cleanup locks, and joins
//! its relay after dispositions are restored but before the original thread mask is restored.

use std::{
    io::Read,
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream,
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
};

static SIGNAL_WAKE_FD: AtomicI32 = AtomicI32::new(-1);

pub(super) struct TerminationReadiness {
    ready: AtomicBool,
    waiter: Mutex<Option<Waker>>,
}

impl TerminationReadiness {
    pub(super) fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            waiter: Mutex::new(None),
        }
    }

    pub(super) fn poll(&self, context: &mut Context<'_>) -> Poll<()> {
        if self.ready.swap(false, Ordering::AcqRel) {
            return Poll::Ready(());
        }
        let mut waiter = self
            .waiter
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if waiter
            .as_ref()
            .is_none_or(|current| !current.will_wake(context.waker()))
        {
            *waiter = Some(context.waker().clone());
        }
        if self.ready.swap(false, Ordering::AcqRel) {
            waiter.take();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn notify(&self) {
        self.ready.store(true, Ordering::Release);
        let waiter = self
            .waiter
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

pub(super) struct TerminationNotifier {
    writer: Option<UnixStream>,
    worker: Option<JoinHandle<()>>,
}

impl TerminationNotifier {
    pub(super) fn install(readiness: Arc<TerminationReadiness>) -> Result<Self, std::io::Error> {
        let (reader, writer) = UnixStream::pair()?;
        writer.set_nonblocking(true)?;
        let worker = thread::Builder::new()
            .name("yo-signal-readiness".to_owned())
            .spawn(move || relay(reader, &readiness))?;
        let fd = writer.as_raw_fd();
        if SIGNAL_WAKE_FD
            .compare_exchange(-1, fd, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            drop(writer);
            let _ = worker.join();
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "the process signal readiness bridge is already installed",
            ));
        }
        Ok(Self {
            writer: Some(writer),
            worker: Some(worker),
        })
    }
}

impl Drop for TerminationNotifier {
    fn drop(&mut self) {
        SIGNAL_WAKE_FD.store(-1, Ordering::SeqCst);
        drop(self.writer.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn signal_wake_fd() -> Option<RawFd> {
    let fd = SIGNAL_WAKE_FD.load(Ordering::SeqCst);
    (fd >= 0).then_some(fd)
}

fn relay(mut reader: UnixStream, readiness: &TerminationReadiness) {
    let mut buffer = [0_u8; 64];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => readiness.notify(),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
    };

    use super::TerminationReadiness;

    struct CountingWake(AtomicUsize);

    impl Wake for CountingWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    // waiter 등록 전에 도착한 알림도 level 상태로 남아 첫 poll에서 즉시 소비됩니다.
    #[test]
    fn notification_before_registration_is_not_lost() {
        let readiness = TerminationReadiness::new();
        readiness.notify();
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        assert_eq!(readiness.poll(&mut context), Poll::Ready(()));
        assert_eq!(readiness.poll(&mut context), Poll::Pending);
    }

    // waiter 등록 뒤의 알림은 등록된 frontend를 한 번 깨우고 다음 poll에서 준비 상태를 냅니다.
    #[test]
    fn notification_after_registration_wakes_the_frontend() {
        let readiness = TerminationReadiness::new();
        let wake = Arc::new(CountingWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut context = Context::from_waker(&waker);

        assert_eq!(readiness.poll(&mut context), Poll::Pending);
        readiness.notify();

        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        assert_eq!(readiness.poll(&mut context), Poll::Ready(()));
    }
}
