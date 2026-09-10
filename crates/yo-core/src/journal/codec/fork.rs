//! Bounded child-owned fork seeds; source authority is checked during construction/recovery.

mod model;

pub(crate) use model::{
    ForkExactReplay, ForkGroup, ForkHistoryCoordinate, ForkHistoryEntry, ForkItemCoordinate,
    ForkItemOrigin, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint,
    INITIAL_FORK_SEED_PROFILE, InitialForkSeed,
};

#[cfg(test)]
mod tests;
