//! Kimi service catalog discovery and account capacity.

mod account_capacity;
mod catalog;

pub use account_capacity::{
    KimiAccountCapacityError, KimiAccountCapacityFailureKind, parse_kimi_account_capacity_snapshot,
    read_kimi_account_capacity,
};
pub use catalog::{
    KimiCatalogAvailability, KimiCatalogDisabledReason, KimiCatalogError, KimiCatalogFailureKind,
    KimiCatalogModel, KimiCatalogSeed, discover_kimi_models, parse_kimi_catalog_snapshot,
};
