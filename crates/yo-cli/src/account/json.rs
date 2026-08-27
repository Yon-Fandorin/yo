use serde::Serialize;
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
};

const SCHEMA: &str = "yo.account-capacity/v1alpha1";

pub(super) fn render(snapshot: &AccountCapacitySnapshot) -> Result<String, serde_json::Error> {
    let mut output = serde_json::to_string(&AccountCapacityOutput::from(snapshot))?;
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
}

impl<'a> From<&'a AccountCapacitySnapshot> for AccountCapacityOutput<'a> {
    fn from(snapshot: &'a AccountCapacitySnapshot) -> Self {
        Self {
            schema: SCHEMA,
            provider: snapshot.provider().as_str(),
            account: snapshot.account().as_str(),
            limits: snapshot
                .buckets()
                .iter()
                .map(AccountLimitOutput::from)
                .collect(),
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
    used_percent: u8,
    remaining_percent: u8,
    resets_at_unix_seconds: Option<i64>,
}

impl AccountWindowOutput {
    fn new(kind: &'static str, window: AccountCapacityWindow) -> Self {
        Self {
            kind,
            window_minutes: window.window_duration_minutes(),
            used_percent: window.used_percent(),
            remaining_percent: window.remaining_percent(),
            resets_at_unix_seconds: window.resets_at_unix_seconds(),
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
    use yo_core::{AccountId, ProviderId};

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

        let output = render(&snapshot).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(output.lines().count(), 1);
        assert!(output.ends_with('\n'));
        assert_eq!(
            decoded,
            json!({
                "schema": "yo.account-capacity/v1alpha1",
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
}
