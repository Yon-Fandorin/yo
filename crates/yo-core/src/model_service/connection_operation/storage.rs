//! 제한된 operation journal 저장소의 구성과 외부에 노출하는 저장 연산.

mod capture;
mod residue;
mod security;

pub(super) use capture::{abandon_intent, advance, capture, clear_complete, publish_intent};
pub(super) use residue::cleanup_pending_residues;
#[cfg(test)]
pub(super) use residue::{
    cleanup_pending_residues_in_order_for_test, pending_residue_name_for_test,
    pending_residue_path_for_test, validate_pending_residue_owner_for_test,
};
