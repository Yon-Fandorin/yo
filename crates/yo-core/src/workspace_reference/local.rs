//! Local execution-workspace discovery kept outside the terminal UI thread.

mod admission;
mod budget;
mod discovery;
mod filesystem;
mod git;
mod inventory;
mod output_links;
mod ranking;

pub use admission::LocalWorkspaceInputAdmission;
pub use discovery::LocalWorkspaceReferenceProvider;

#[cfg(test)]
use self::{
    budget::DiscoveryBudget,
    discovery::{pin_root, worker},
    filesystem::discover_entries,
    git::{classify_git_workspace, git_command, is_git_workspace},
    inventory::build_inventory,
    ranking::{rank, search},
};

#[cfg(test)]
mod tests;
