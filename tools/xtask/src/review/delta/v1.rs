//! Frozen review-delta v1 wire contract.
//!
//! Published v1 chains retain their original path-string transition policy.

use super::{AffectedPathPolicy, WireContract};

pub(super) const PLAN_SCHEMA: &str = "yo.slice-review-delta-plan/v1";
pub(super) const MANIFEST_SCHEMA: &str = "yo.slice-review-delta-manifest/v1";
pub(super) const DELIVERY_PROFILE: &str = "yo.slice-review-delta-markdown/v1";

pub(super) fn contract() -> WireContract {
    WireContract {
        plan_schema: PLAN_SCHEMA,
        manifest_schema: MANIFEST_SCHEMA,
        delivery_profile: DELIVERY_PROFILE,
        review_id_domain: b"yo.slice-review-delta/v1",
        affected_path_policy: AffectedPathPolicy::LegacyStringIdentity,
    }
}
