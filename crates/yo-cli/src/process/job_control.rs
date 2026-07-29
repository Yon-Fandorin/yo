use std::{error::Error, fmt};

use nix::sys::signal::{
    SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, pthread_sigmask, raise,
};

use super::termination::disposition;

pub(crate) struct JobControl<O = UnixJobControlOs> {
    os: O,
}

impl JobControl<UnixJobControlOs> {
    pub(crate) const fn new() -> Self {
        Self {
            os: UnixJobControlOs,
        }
    }
}

impl<O> JobControl<O>
where
    O: JobControlOs,
{
    pub(crate) fn suspend(&mut self) -> Result<(), JobControlError> {
        let original_mask = self
            .os
            .block_suspend()
            .map_err(|error| JobControlError::single("blocking SIGTSTP", error))?;
        let prior_action = match self.os.install_default() {
            Ok(action) => action,
            Err(primary) => {
                let mut failures = vec![format!("installing default SIGTSTP action: {primary}")];
                if let Err(error) = self.os.restore_mask(&original_mask) {
                    failures.push(format!("restoring the signal mask: {error}"));
                }
                return Err(JobControlError::many(failures));
            },
        };
        if let Err(primary) = self.os.unblock_suspend() {
            let mut failures = vec![format!("unblocking SIGTSTP before suspension: {primary}")];
            if let Err(error) = self.os.restore_action(&prior_action) {
                failures.push(format!("restoring the prior SIGTSTP action: {error}"));
            }
            if let Err(error) = self.os.restore_mask(&original_mask) {
                failures.push(format!("restoring the signal mask: {error}"));
            }
            return Err(JobControlError::many(failures));
        }

        let suspend = self.os.suspend_self();
        let mut failures = Vec::new();
        if let Err(error) = suspend {
            failures.push(format!("sending SIGTSTP to the process: {error}"));
        }
        if let Err(error) = self.os.block_suspend() {
            failures.push(format!("blocking SIGTSTP after continuation: {error}"));
        }
        if let Err(error) = self.os.restore_action(&prior_action) {
            failures.push(format!("restoring the prior SIGTSTP action: {error}"));
        }
        if let Err(error) = self.os.restore_mask(&original_mask) {
            failures.push(format!("restoring the signal mask: {error}"));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(JobControlError::many(failures))
        }
    }
}

pub(crate) trait JobControlOs {
    type Action;
    type Mask;

    fn block_suspend(&mut self) -> Result<Self::Mask, String>;
    fn unblock_suspend(&mut self) -> Result<(), String>;
    fn install_default(&mut self) -> Result<Self::Action, String>;
    fn restore_mask(&mut self, mask: &Self::Mask) -> Result<(), String>;
    fn suspend_self(&mut self) -> Result<(), String>;
    fn restore_action(&mut self, action: &Self::Action) -> Result<(), String>;
}

pub(crate) struct UnixJobControlOs;

impl JobControlOs for UnixJobControlOs {
    type Action = SigAction;
    type Mask = SigSet;

    fn block_suspend(&mut self) -> Result<Self::Mask, String> {
        let mut suspend = SigSet::empty();
        suspend.add(Signal::SIGTSTP);
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&suspend), Some(&mut previous))
            .map_err(|error| error.to_string())?;
        Ok(previous)
    }

    fn install_default(&mut self) -> Result<Self::Action, String> {
        let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        disposition::replace(Signal::SIGTSTP, &action).map_err(|error| error.to_string())
    }

    fn unblock_suspend(&mut self) -> Result<(), String> {
        let mut suspend = SigSet::empty();
        suspend.add(Signal::SIGTSTP);
        pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&suspend), None)
            .map_err(|error| error.to_string())
    }

    fn restore_mask(&mut self, mask: &Self::Mask) -> Result<(), String> {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(mask), None)
            .map_err(|error| error.to_string())
    }

    fn suspend_self(&mut self) -> Result<(), String> {
        raise(Signal::SIGTSTP).map_err(|error| error.to_string())
    }

    fn restore_action(&mut self, action: &Self::Action) -> Result<(), String> {
        disposition::replace(Signal::SIGTSTP, action)
            .map(drop)
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug)]
pub(crate) struct JobControlError {
    failures: Vec<String>,
}

impl JobControlError {
    fn single(context: &'static str, error: String) -> Self {
        Self::many([format!("{context}: {error}")])
    }

    fn many(failures: impl IntoIterator<Item = String>) -> Self {
        Self {
            failures: failures.into_iter().collect(),
        }
    }
}

impl fmt::Display for JobControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.failures.join("; additionally, "))
    }
}

impl Error for JobControlError {}

#[cfg(test)]
mod tests;
