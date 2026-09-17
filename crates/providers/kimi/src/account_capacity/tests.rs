mod parser;
mod transport;

use serde_json::json;
use yo_core::{AccountId, ProviderId, VersionedProfileId};

use crate::catalog::KimiCatalogSeed;

fn code_seed() -> KimiCatalogSeed {
    KimiCatalogSeed::resolve(
        VersionedProfileId::new("kimi-code-membership/v1").unwrap(),
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        None,
        None,
    )
    .unwrap()
}

fn platform_seed() -> KimiCatalogSeed {
    KimiCatalogSeed::resolve(
        VersionedProfileId::new("kimi-platform-ai/v1").unwrap(),
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        None,
        None,
    )
    .unwrap()
}

fn usage_payload() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "usage": {
            "used": 92,
            "limit": 100,
            "resetTime": "2026-08-30T04:05:00Z"
        },
        "limits": [{
            "name": "rolling",
            "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
            "detail": {
                "used": "7",
                "limit": "100",
                "resetTime": "2026-08-27T17:36:00Z"
            }
        }],
        "boosterWallet": {
            "balance": {
                "type": "BOOSTER",
                "amount": "1000000000",
                "amountLeft": "500000000"
            },
            "monthlyChargeLimit": {"currency": "USD"}
        }
    }))
    .unwrap()
}
