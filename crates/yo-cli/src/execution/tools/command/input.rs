//! Nonblocking one-shot JSON stdin, serviced alongside output and process lifecycle.

use std::{io::Write, process::ChildStdin};

use nix::fcntl::{FcntlArg, OFlag, fcntl};

pub(super) struct CommandInput {
    pipe: Option<ChildStdin>,
    bytes: Vec<u8>,
    written: usize,
}

impl CommandInput {
    pub(super) fn new(pipe: Option<ChildStdin>, bytes: Vec<u8>) -> Result<Self, ()> {
        if let Some(pipe) = &pipe {
            let flags = fcntl(pipe, FcntlArg::F_GETFL).map_err(|_| ())?;
            fcntl(
                pipe,
                FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
            )
            .map_err(|_| ())?;
        } else if !bytes.is_empty() {
            return Err(());
        }
        Ok(Self {
            pipe,
            bytes,
            written: 0,
        })
    }

    pub(super) fn complete(&self) -> bool {
        self.written == self.bytes.len()
    }

    pub(super) fn close(&mut self) {
        self.pipe = None;
    }

    pub(super) fn poll(&mut self) -> Result<(), ()> {
        if self.complete() {
            self.close();
            return Ok(());
        }
        let pipe = self.pipe.as_mut().ok_or(())?;
        let end = self.bytes.len().min(self.written.saturating_add(64 * 1024));
        match pipe.write(&self.bytes[self.written..end]) {
            Ok(0) => Err(()),
            Ok(written) => {
                self.written += written;
                if self.complete() {
                    self.close();
                }
                Ok(())
            },
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            },
            Err(_) => {
                self.close();
                Err(())
            },
        }
    }
}
