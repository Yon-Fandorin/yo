use std::{io::Read, os::fd::AsFd};

use nix::fcntl::{FcntlArg, OFlag, fcntl};

pub(super) fn set_nonblocking(descriptor: &impl AsFd) -> Result<(), ()> {
    let flags = fcntl(descriptor, FcntlArg::F_GETFL).map_err(|_| ())?;
    fcntl(
        descriptor,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map(|_| ())
    .map_err(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PipeState {
    Open,
    Closed,
    Truncated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PipeRead {
    pub(super) state: PipeState,
    pub(super) progressed: bool,
}

pub(super) fn read_nonblocking_bounded(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    limit: usize,
) -> Result<PipeRead, ()> {
    let mut chunk = [0_u8; 8 * 1024];
    let mut progressed = false;
    loop {
        let target = limit.saturating_add(1);
        if output.len() >= target {
            output.truncate(limit);
            return Ok(PipeRead {
                state: PipeState::Truncated,
                progressed,
            });
        }
        let read_len = chunk.len().min(target - output.len());
        match reader.read(&mut chunk[..read_len]) {
            Ok(0) => {
                return Ok(PipeRead {
                    state: PipeState::Closed,
                    progressed,
                });
            },
            Ok(count) => {
                progressed = true;
                output.extend_from_slice(&chunk[..count]);
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(PipeRead {
                    state: PipeState::Open,
                    progressed,
                });
            },
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
            Err(_) => return Err(()),
        }
    }
}
