use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

/// Coalesced readiness shared by one producer and its nonblocking consumer.
pub(crate) struct Readiness {
    ready: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Readiness {
    pub(crate) const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            waker: Mutex::new(None),
        }
    }

    pub(crate) fn notify(&self) {
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

    pub(crate) fn poll(&self, context: &mut Context<'_>) -> Poll<()> {
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

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
    };

    use super::Readiness;

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
}
