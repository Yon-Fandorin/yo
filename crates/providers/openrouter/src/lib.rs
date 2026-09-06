//! OpenRouter authenticated account catalog discovery.

mod discovery;

pub use discovery::{
    OpenRouterAuthoredModel, OpenRouterDisabledReason, OpenRouterDiscoveredModel,
    OpenRouterDiscoveryError, OpenRouterDiscoveryFailureKind, OpenRouterDiscoverySeed,
    OpenRouterModelAvailability, OpenRouterModelCapabilities, discover_openrouter_models,
};
