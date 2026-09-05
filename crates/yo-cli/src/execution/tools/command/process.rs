use std::{
    io,
    process::{Child, ExitStatus},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
};
#[cfg(test)]
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};

#[derive(Clone, Default)]
pub(super) struct WaiterTestHooks {
    #[cfg(test)]
    pub(super) observation_delay: Duration,
    #[cfg(test)]
    pub(super) reap_count: Option<Arc<AtomicUsize>>,
}

pub(super) struct ChildWaiter {
    child_sender: Option<SyncSender<Child>>,
    reap_sender: Option<SyncSender<()>>,
    leader_receiver: Receiver<Result<(), ()>>,
    status_receiver: Receiver<Result<ExitStatus, ()>>,
    thread: Option<JoinHandle<()>>,
}

impl ChildWaiter {
    pub(super) fn spawn(hooks: WaiterTestHooks) -> Result<Self, ()> {
        #[cfg(not(test))]
        let _ = hooks;
        let (child_sender, child_receiver) = mpsc::sync_channel(1);
        let (reap_sender, reap_receiver) = mpsc::sync_channel(1);
        let (leader_sender, leader_receiver) = mpsc::sync_channel(1);
        let (status_sender, status_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("yo-command-waiter".to_owned())
            .spawn(move || {
                let Ok(mut child): Result<Child, _> = child_receiver.recv() else {
                    return;
                };
                let leader = i32::try_from(child.id())
                    .map(Pid::from_raw)
                    .map_err(|_| ())
                    .and_then(wait_for_child_exit_without_reaping);
                #[cfg(test)]
                if !hooks.observation_delay.is_zero() {
                    thread::sleep(hooks.observation_delay);
                }
                let _ = leader_sender.send(leader);

                // A disconnected request channel means the bounded result owner has
                // already returned. The waiter still retains the Child and performs
                // the one eventual wait instead of dropping an unreaped handle.
                let _ = reap_receiver.recv();
                let status = child.wait().map_err(|_| ());
                #[cfg(test)]
                if let Some(reap_count) = hooks.reap_count {
                    reap_count.fetch_add(1, Ordering::Release);
                }
                let _ = status_sender.send(status);
            })
            .map_err(|_| ())?;
        Ok(Self {
            child_sender: Some(child_sender),
            reap_sender: Some(reap_sender),
            leader_receiver,
            status_receiver,
            thread: Some(thread),
        })
    }

    pub(super) fn handoff(&mut self, child: Child) -> Result<(), Child> {
        let Some(sender) = self.child_sender.take() else {
            return Err(child);
        };
        sender.send(child).map_err(|error| error.0)
    }

    pub(super) fn try_leader_exit(&self) -> Result<bool, ()> {
        match self.leader_receiver.try_recv() {
            Ok(result) => result.map(|()| true),
            Err(TryRecvError::Empty) => Ok(false),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }

    pub(super) fn request_reap(&mut self) -> Result<(), ()> {
        let Some(sender) = self.reap_sender.take() else {
            return Ok(());
        };
        sender.send(()).map_err(|_| ())
    }

    pub(super) fn try_status(&mut self) -> Result<Option<ExitStatus>, ()> {
        match self.status_receiver.try_recv() {
            Ok(result) => {
                let status = result?;
                if let Some(thread) = self.thread.take() {
                    thread.join().map_err(|_| ())?;
                }
                Ok(Some(status))
            },
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }
}

pub(super) fn terminate_process_group_id(process_group: Pid) {
    let _ = kill(Pid::from_raw(-process_group.as_raw()), Signal::SIGKILL);
}

// WNOWAIT pins the exited group leader until the worker has killed descendants and
// explicitly permits the dedicated waiter to reap it. This prevents process-group ID
// reuse from redirecting a late cleanup signal to an unrelated process group.
#[allow(unsafe_code)]
fn wait_for_child_exit_without_reaping(child: Pid) -> Result<(), ()> {
    loop {
        // SAFETY: waitid initializes siginfo_t on success. P_PID names the exact child
        // owned by this process, and WNOWAIT observes without consuming its wait status.
        let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.as_raw() as libc::id_t,
                &mut information,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(());
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(());
        }
    }
}
