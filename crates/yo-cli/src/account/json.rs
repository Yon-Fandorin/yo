use serde::{Serialize, Serializer};
use yo_core::{AccountCapacityBucket, AccountCapacityWindow, AccountCredits};

use super::AccountCapacityReport;

const SCHEMA: &str = "yo.account-capacity/v1alpha2";

pub(super) fn render(report: &AccountCapacityReport) -> Result<String, serde_json::Error> {
    let mut output = serde_json::to_string(&AccountCapacityOutput::from(report))?;
    output.push('\n');
    Ok(output)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountCapacityOutput<'a> {
    schema: &'static str,
    provider: &'a str,
    account: &'a str,
    limits: Vec<AccountLimitOutput<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_data: Option<&'a super::AccountProviderData>,
}

impl<'a> From<&'a AccountCapacityReport> for AccountCapacityOutput<'a> {
    fn from(report: &'a AccountCapacityReport) -> Self {
        let snapshot = &report.snapshot;
        Self {
            schema: SCHEMA,
            provider: snapshot.provider().as_str(),
            account: snapshot.account().as_str(),
            limits: snapshot
                .buckets()
                .iter()
                .map(AccountLimitOutput::from)
                .collect(),
            provider_data: report.provider_data.as_ref(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountLimitOutput<'a> {
    id: Option<&'a str>,
    name: Option<&'a str>,
    plan: Option<&'a str>,
    windows: Vec<AccountWindowOutput>,
    credits: Option<AccountCreditsOutput<'a>>,
    limit_reason: Option<&'a str>,
}

impl<'a> From<&'a AccountCapacityBucket> for AccountLimitOutput<'a> {
    fn from(bucket: &'a AccountCapacityBucket) -> Self {
        let mut windows = Vec::with_capacity(2);
        if let Some(window) = bucket.primary() {
            windows.push(AccountWindowOutput::new("primary", *window));
        }
        if let Some(window) = bucket.secondary() {
            windows.push(AccountWindowOutput::new("secondary", *window));
        }
        Self {
            id: bucket.id(),
            name: bucket.name(),
            plan: bucket.plan(),
            windows,
            credits: bucket.credits().map(AccountCreditsOutput::from),
            limit_reason: bucket.limit_reason(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountWindowOutput {
    kind: &'static str,
    window_minutes: Option<u64>,
    used_percent: PercentOutput,
    remaining_percent: PercentOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    resets_at_unix_seconds: Option<i64>,
}

impl AccountWindowOutput {
    fn new(kind: &'static str, window: AccountCapacityWindow) -> Self {
        let (used, limit) = window
            .reported_usage()
            .map_or((None, None), |(used, limit)| (Some(used), Some(limit)));
        Self {
            kind,
            window_minutes: window.window_duration_minutes(),
            used_percent: PercentOutput(window.used_percent_basis_points()),
            remaining_percent: PercentOutput(window.remaining_percent_basis_points()),
            used,
            limit,
            resets_at_unix_seconds: window.resets_at_unix_seconds(),
        }
    }
}

#[derive(Clone, Copy)]
struct PercentOutput(u16);

impl Serialize for PercentOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0.is_multiple_of(100) {
            serializer.serialize_u16(self.0 / 100)
        } else {
            serializer.serialize_f64(f64::from(self.0) / 100.0)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountCreditsOutput<'a> {
    balance: Option<&'a str>,
    has_credits: bool,
    unlimited: bool,
}

impl<'a> From<&'a AccountCredits> for AccountCreditsOutput<'a> {
    fn from(credits: &'a AccountCredits) -> Self {
        Self {
            balance: credits.balance(),
            has_credits: credits.has_credits(),
            unlimited: credits.unlimited(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yo_core::{AccountCapacitySnapshot, AccountId, ProviderId};

    use super::*;

    // Agent용 JSON은 일반 Codex 주간 창과 Spark의 5시간·주간 창을 모두 보존하고,
    // 내부 limit ID와 표시 이름을 별도 필드로 제공하여 화면 텍스트 파싱을 요구하지 않습니다.
    #[test]
    fn renders_all_capacity_windows_with_stable_identity_fields() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![
                AccountCapacityBucket::new(
                    Some("codex".to_owned()),
                    None,
                    Some("prolite".to_owned()),
                    Some(
                        AccountCapacityWindow::new(60, Some(10_080), Some(1_800_000_000)).unwrap(),
                    ),
                    None,
                    Some(AccountCredits::new(Some("0".to_owned()), false, false)),
                    None,
                ),
                AccountCapacityBucket::new(
                    Some("codex_bengalfox".to_owned()),
                    Some("GPT-5.3-Codex-Spark".to_owned()),
                    Some("prolite".to_owned()),
                    Some(AccountCapacityWindow::new(0, Some(300), None).unwrap()),
                    Some(AccountCapacityWindow::new(0, Some(10_080), None).unwrap()),
                    None,
                    None,
                ),
            ],
        );

        let output = render(&AccountCapacityReport::plain(snapshot)).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(output.lines().count(), 1);
        assert!(output.ends_with('\n'));
        assert_eq!(
            decoded,
            json!({
                "schema": "yo.account-capacity/v1alpha2",
                "provider": "codex",
                "account": "default",
                "limits": [
                    {
                        "id": "codex",
                        "name": null,
                        "plan": "prolite",
                        "windows": [{
                            "kind": "primary",
                            "windowMinutes": 10080,
                            "usedPercent": 60,
                            "remainingPercent": 40,
                            "resetsAtUnixSeconds": 1800000000_i64
                        }],
                        "credits": {
                            "balance": "0",
                            "hasCredits": false,
                            "unlimited": false
                        },
                        "limitReason": null
                    },
                    {
                        "id": "codex_bengalfox",
                        "name": "GPT-5.3-Codex-Spark",
                        "plan": "prolite",
                        "windows": [
                            {
                                "kind": "primary",
                                "windowMinutes": 300,
                                "usedPercent": 0,
                                "remainingPercent": 100,
                                "resetsAtUnixSeconds": null
                            },
                            {
                                "kind": "secondary",
                                "windowMinutes": 10080,
                                "usedPercent": 0,
                                "remainingPercent": 100,
                                "resetsAtUnixSeconds": null
                            }
                        ],
                        "credits": null,
                        "limitReason": null
                    }
                ]
            })
        );
    }

    // JSON number는 소수 비율을 잃지 않으면서 정수 비율의 기존 표현도 보존합니다.
    #[test]
    fn preserves_fractional_percentages_as_json_numbers() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("qwencloud".to_owned()),
                None,
                None,
                Some(
                    AccountCapacityWindow::from_used_percent_basis_points(163, None, None).unwrap(),
                ),
                None,
                None,
                None,
            )],
        );

        let output = render(&AccountCapacityReport::plain(snapshot)).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["limits"][0]["windows"][0]["usedPercent"], 1.63);
        assert_eq!(
            decoded["limits"][0]["windows"][0]["remainingPercent"],
            98.37
        );
        assert!(decoded["limits"][0]["windows"][0].get("used").is_none());
        assert!(decoded.get("providerData").is_none());
    }

    // Count 기반 Provider는 정규화된 percentage뿐 아니라 Provider가 보고한 exact
    // numerator와 denominator도 보존하여 agent가 원 단위를 다시 확인할 수 있습니다.
    #[test]
    fn preserves_reported_usage_counts() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("weekly".to_owned()),
                None,
                None,
                Some(AccountCapacityWindow::from_usage_ratio(1, 3, None, None).unwrap()),
                None,
                None,
                None,
            )],
        );

        let output = render(&AccountCapacityReport::plain(snapshot)).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
        let window = &decoded["limits"][0]["windows"][0];

        assert_eq!(window["usedPercent"], 33.34);
        assert_eq!(window["remainingPercent"], 66.66);
        assert_eq!(window["used"], 1);
        assert_eq!(window["limit"], 3);
    }
}
