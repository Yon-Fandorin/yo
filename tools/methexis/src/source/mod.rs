//! Source record validation and freshness evaluation.

use std::collections::BTreeMap;

mod freshness;
mod records;
mod revision;
mod validation;
mod working_tree;

#[cfg(test)]
mod tests;

pub(crate) use freshness::evaluate;
pub(crate) use records::load;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Eligibility {
    Active,
    Stale,
    Invalid,
}

impl Eligibility {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
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
}
