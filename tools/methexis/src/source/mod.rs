//! Source record validation and freshness evaluation.

use std::collections::BTreeMap;

mod freshness;
pub(crate) mod negative;
mod records;
mod revision;
mod validation;
mod working_tree;

#[cfg(test)]
mod tests;

pub(crate) use freshness::{FreshnessFailure, FreshnessGuard, evaluate, final_revalidate};
pub(crate) use negative::NegativeRecords;
pub(crate) use records::{load, load_captured};
pub(crate) use revision::calculate as calculate_revision;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Eligibility {
    Active,
    Stale,
    Suspect,
    Invalid,
}

impl Eligibility {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Suspect => "suspect",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UnitFreshness {
    pub(crate) eligibility: Eligibility,
    pub(crate) evidence: Vec<String>,
}

pub(crate) struct FreshnessEvaluation {
    pub(crate) units: BTreeMap<String, UnitFreshness>,
    pub(crate) checkpoint: &'static str,
    pub(crate) guard: FreshnessGuard,
}
