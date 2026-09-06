//! Provider-neutral Yo-managed model/tool loop over an admitted API-dialect connector.

mod backend;

pub use backend::{
    ModelRequestObserver, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
};
