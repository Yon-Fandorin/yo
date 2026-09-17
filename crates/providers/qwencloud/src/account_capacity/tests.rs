use serde_json::Value;
use yo_core::{AccountId, ProviderId};

mod parser;
mod transport;

fn provider() -> ProviderId {
    ProviderId::new("qwencloud").unwrap()
}

fn account() -> AccountId {
    AccountId::new("default").unwrap()
}

fn gateway_payload(data: Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "code": "200", "data": { "DataV2": { "data": {
            "code": "SUCCESS", "success": true, "data": data
        }}}
    }))
    .unwrap()
}
