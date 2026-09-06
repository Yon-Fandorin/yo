pub(crate) mod input;
pub(crate) mod presentation;

// Temporary state routes while prompts and presentation retain this owner.
#[cfg(test)]
pub(crate) use crate::state::connection::canonical_test_temp_dir;
pub(crate) use crate::state::connection::{
    absolute_config_path, admit_target, complete_binding_details, display_target,
    load_startup_connections, operation_repositories, selection_for_binding,
};
