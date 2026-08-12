//! End-to-end Context Resolution contracts against isolated trusted Git repos.

#[path = "context_flow/contract.rs"]
mod contract;
#[path = "context_flow/failures.rs"]
mod failures;
#[allow(dead_code)]
#[path = "checkpoint_flow/support.rs"]
mod repository;
#[path = "context_flow/support.rs"]
mod support;
#[path = "context_flow/verification.rs"]
mod verification;
