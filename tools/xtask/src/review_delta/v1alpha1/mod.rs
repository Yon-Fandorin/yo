//! Canonical evidence-path review-delta wire contract.
//!
//! New v1alpha1 chains require affected evidence to move to a distinct
//! canonical filesystem identity.

use super::{AffectedPathPolicy, WireContract};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-delta-request/v1alpha1";
pub(super) const PLAN_SCHEMA: &str = "yo.slice-review-delta-plan/v1alpha1";
pub(super) const MANIFEST_SCHEMA: &str = "yo.slice-review-delta-manifest/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-delta-result/v1alpha1";
pub(super) const DELIVERY_PROFILE: &str = "yo.slice-review-delta-markdown/v1alpha1";

pub(super) fn contract() -> WireContract {
    WireContract {
        plan_schema: PLAN_SCHEMA,
        manifest_schema: MANIFEST_SCHEMA,
        delivery_profile: DELIVERY_PROFILE,
        review_id_domain: b"yo.slice-review-delta/v1alpha1",
        affected_path_policy: AffectedPathPolicy::CanonicalIdentity,
    }
}
