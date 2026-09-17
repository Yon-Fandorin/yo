use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) enum ExpectedMedia {
    Html,
    Json,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QwenCloudProviderData {
    pub(super) spec_code: String,
    pub(super) usage: QwenCloudUsageData,
    pub(super) quota: QwenCloudQuotaData,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QwenCloudUsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) per5_hour_percentage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) per5_hour_reset_time: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) per1_week_percentage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) per1_week_reset_time: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct QwenCloudQuotaData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) five_hour: Option<Value>,
    pub(super) weekly: Value,
}
