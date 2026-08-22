use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    task::{Context, Poll, Waker},
};

/// Coalesced readiness shared by one producer and its nonblocking consumer.
pub struct Readiness {
    ready: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// A receiver whose readiness remains level-triggered while buffered work exists.
///
/// [`Readiness`] coalesces producer notifications, so a consumer that drains a bounded batch must
/// also observe the receiver itself before parking. This wrapper prefetches one value during
/// [`ReadyReceiver::poll_ready`] to keep a non-empty queue ready across batch boundaries.
pub struct ReadyReceiver<T> {
    receiver: Receiver<T>,
    readiness: Arc<Readiness>,
    prefetched: Option<T>,
    disconnected: bool,
}

impl<T> ReadyReceiver<T> {
    /// Wraps a receiver and the readiness signal used by its producer.
    pub fn new(receiver: Receiver<T>, readiness: Arc<Readiness>) -> Self {
        Self {
            receiver,
            readiness,
            prefetched: None,
            disconnected: false,
        }
    }

    /// Receives a prefetched or immediately available value without blocking.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        if let Some(value) = self.prefetched.take() {
            return Ok(value);
        }
        if self.disconnected {
            return Err(TryRecvError::Disconnected);
        }
        match self.receiver.try_recv() {
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Err(TryRecvError::Disconnected)
            },
            result => result,
        }
    }

    /// Polls queue readiness while preserving one available value for [`Self::try_recv`].
    pub fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if self.prefetched.is_some() || self.disconnected {
            return Poll::Ready(());
        }
        match self.receiver.try_recv() {
            Ok(value) => {
                self.prefetched = Some(value);
                Poll::Ready(())
            },
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Poll::Ready(())
            },
            Err(TryRecvError::Empty) => self.readiness.poll(context),
        }
    }
}

impl Readiness {
    /// Creates an idle readiness signal without a registered consumer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            waker: Mutex::new(None),
        }
    }

    /// Coalesces one producer notification and wakes the registered consumer, if any.
    pub fn notify(&self) {
        self.ready.store(true, Ordering::Release);
        let waker = self
            .waker
            .lock()
            .ok()
            .and_then(|mut registered| registered.take());
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Polls and consumes one coalesced readiness observation.
    pub fn poll(&self, context: &mut Context<'_>) -> Poll<()> {
        if self.ready.swap(false, Ordering::Acquire) {
            return Poll::Ready(());
        }

        let Ok(mut registered) = self.waker.lock() else {
            return Poll::Ready(());
        };
        if registered
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            *registered = Some(context.waker().clone());
        }

        if self.ready.swap(false, Ordering::Acquire) {
            registered.take();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Default for Readiness {
    fn default() -> Self {
        Self::new()
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

    use super::{Readiness, ReadyReceiver};

    struct CountingWake(AtomicUsize);

    impl Wake for CountingWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    // producer 알림은 등록된 consumer를 한 번 깨우고 coalesced readiness도 한 번 전달합니다.
    #[test]
    fn notification_wakes_and_preserves_one_ready_observation() {
        let readiness = Readiness::new();
        let wake = Arc::new(CountingWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut context = Context::from_waker(&waker);

        assert_eq!(readiness.poll(&mut context), Poll::Pending);
        readiness.notify();
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        assert_eq!(readiness.poll(&mut context), Poll::Ready(()));
        assert_eq!(readiness.poll(&mut context), Poll::Pending);
    }

    // 등록보다 먼저 온 여러 알림도 소실되지 않으며 하나의 level-triggered 관찰로 합쳐집니다.
    #[test]
    fn notifications_before_registration_coalesce_without_loss() {
        let readiness = Readiness::new();
        readiness.notify();
        readiness.notify();
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        assert_eq!(readiness.poll(&mut context), Poll::Ready(()));
        assert_eq!(readiness.poll(&mut context), Poll::Pending);
    }

    // 하나로 합쳐진 알림 뒤에도 bounded batch가 남긴 큐 항목은 다음 batch를 계속 깨웁니다.
    #[test]
    fn receiver_stays_ready_across_bounded_batch_boundaries() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let readiness = Arc::new(Readiness::new());
        let mut receiver = ReadyReceiver::new(receiver, Arc::clone(&readiness));
        for value in 0..513 {
            sender.send(value).unwrap();
        }
        readiness.notify();
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        for expected in 0..256 {
            assert_eq!(receiver.try_recv(), Ok(expected));
        }
        assert_eq!(receiver.poll_ready(&mut context), Poll::Ready(()));
        for expected in 256..512 {
            assert_eq!(receiver.try_recv(), Ok(expected));
        }
        assert_eq!(receiver.poll_ready(&mut context), Poll::Ready(()));
        assert_eq!(receiver.try_recv(), Ok(512));
        assert_eq!(receiver.poll_ready(&mut context), Poll::Ready(()));
        assert_eq!(receiver.poll_ready(&mut context), Poll::Pending);
    }
}
