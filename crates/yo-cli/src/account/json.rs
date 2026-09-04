use std::borrow::Cow;

use serde::{Serialize, Serializer};
use yo_core::{AccountCapacityBucket, AccountCapacityWindow, AccountCredits};

use super::{AccountCapacityRecord, AccountCapacityReport, AccountQuery, AccountRefreshFailure};

const SCHEMA: &str = "yo.account-capacity/v1alpha3";
const LIST_SCHEMA: &str = "yo.account-capacity-list/v1alpha2";

#[cfg(test)]
pub(super) fn render(records: &[AccountCapacityRecord]) -> Result<String, serde_json::Error> {
    let query = match records {
        [record] => match &record.target {
            super::AccountTarget::Exact(coordinate) => AccountQuery::Exact(coordinate.clone()),
            super::AccountTarget::LocalHost { provider } => {
                AccountQuery::Provider(provider.clone())
            },
        },
        _ => AccountQuery::All,
    };
    render_for_query(records, &query, &[])
}

pub(super) fn render_for_query(
    records: &[AccountCapacityRecord],
    query: &AccountQuery,
    failures: &[AccountRefreshFailure],
) -> Result<String, serde_json::Error> {
    let value = match query {
        AccountQuery::Exact(_) => {
            serde_json::to_value(AccountCapacityOutput::new(&records[0], failures))?
        },
        AccountQuery::All | AccountQuery::Provider(_) => {
            serde_json::to_value(AccountCapacityListOutput::new(records, failures))?
        },
    };
    let mut output = serde_json::to_string(&value)?;
    output.push('\n');
    Ok(output)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountCapacityOutput<'a> {
    schema: &'static str,
    provider: &'a str,
    account: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<&'a str>,
    limits: Vec<AccountLimitOutput<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_data: Option<&'a super::AccountProviderData>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<AccountRefreshErrorOutput<'a>>,
}

impl<'a> AccountCapacityOutput<'a> {
    fn new(record: &'a AccountCapacityRecord, failures: &'a [AccountRefreshFailure]) -> Self {
        let report = record.report.as_ref();
        let snapshot = report.map(AccountCapacityReport::snapshot);
        let account = public_account_label(record);
        let account_id = public_account_id(record, account.as_ref());
        Self {
            schema: SCHEMA,
            provider: record.target.provider().as_str(),
            account,
            account_id,
            observed_at: report.and_then(AccountCapacityReport::observed_at),
            limits: snapshot
                .map(|snapshot| {
                    snapshot
                        .buckets()
                        .iter()
                        .map(AccountLimitOutput::from)
                        .collect()
                })
                .unwrap_or_default(),
            provider_data: report.and_then(AccountCapacityReport::provider_data),
            errors: failure_outputs(failures),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountCapacityListOutput<'a> {
    schema: &'static str,
    accounts: Vec<AccountCapacityListItemOutput<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<AccountRefreshErrorOutput<'a>>,
}

impl<'a> AccountCapacityListOutput<'a> {
    fn new(records: &'a [AccountCapacityRecord], failures: &'a [AccountRefreshFailure]) -> Self {
        Self {
            schema: LIST_SCHEMA,
            accounts: records
                .iter()
                .map(AccountCapacityListItemOutput::from)
                .collect(),
            errors: failure_outputs(failures),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountCapacityListItemOutput<'a> {
    provider: &'a str,
    account: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<&'a str>,
    limits: Vec<AccountLimitOutput<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_data: Option<&'a super::AccountProviderData>,
}

impl<'a> From<&'a AccountCapacityRecord> for AccountCapacityListItemOutput<'a> {
    fn from(record: &'a AccountCapacityRecord) -> Self {
        let report = record.report.as_ref();
        let snapshot = report.map(AccountCapacityReport::snapshot);
        let account = public_account_label(record);
        let account_id = public_account_id(record, account.as_ref());
        Self {
            provider: record.target.provider().as_str(),
            account,
            account_id,
            observed_at: report.and_then(AccountCapacityReport::observed_at),
            limits: snapshot
                .map(|snapshot| {
                    snapshot
                        .buckets()
                        .iter()
                        .map(AccountLimitOutput::from)
                        .collect()
                })
                .unwrap_or_default(),
            provider_data: report.and_then(AccountCapacityReport::provider_data),
        }
    }
}

fn public_account_label<'a>(record: &'a AccountCapacityRecord) -> Cow<'a, str> {
    if let Some(report) = record.report.as_ref() {
        return Cow::Borrowed(report.account_label());
    }
    match &record.target {
        super::AccountTarget::Exact(coordinate) => Cow::Borrowed(coordinate.account.as_str()),
        super::AccountTarget::LocalHost { provider } => {
            Cow::Owned(super::local_host_label(provider))
        },
    }
}

fn public_account_id<'a>(record: &'a AccountCapacityRecord, account: &str) -> Option<&'a str> {
    if let Some(report) = record.report.as_ref() {
        return (account != report.snapshot().account().as_str())
            .then_some(report.snapshot().account().as_str());
    }
    record
        .target
        .exact_coordinate()
        .filter(|coordinate| account != coordinate.account.as_str())
        .map(|coordinate| coordinate.account.as_str())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountRefreshErrorOutput<'a> {
    target: &'a str,
    message: &'a str,
}

fn failure_outputs<'a>(
    failures: &'a [AccountRefreshFailure],
) -> Vec<AccountRefreshErrorOutput<'a>> {
    failures
        .iter()
        .map(|failure| AccountRefreshErrorOutput {
            target: &failure.target,
            message: &failure.message,
        })
        .collect()
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

    use super::{
        super::{AccountCoordinate, AccountTarget},
        *,
    };

    fn record(snapshot: AccountCapacitySnapshot) -> AccountCapacityRecord {
        let report = AccountCapacityReport::plain(snapshot);
        AccountCapacityRecord {
            target: AccountTarget::Exact(report.coordinate()),
            report: Some(report),
        }
    }

    // 목록 JSON은 각 계정의 마지막 관측 시각을 보존하고 미확인 local host label을 누출하지
    // 않습니다.
    #[test]
    fn includes_observation_time_and_supports_multiple_accounts() {
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];

        let output = render(&records).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["schema"], "yo.account-capacity-list/v1alpha2");
        assert_eq!(decoded["accounts"].as_array().unwrap().len(), 2);
        assert_eq!(decoded["accounts"][0]["observedAt"], "2026-09-03T01:02:03Z");
        assert_eq!(decoded["accounts"][1]["account"], "Local Grok");
        assert!(decoded["accounts"][1].get("accountId").is_none());
        assert_ne!(decoded["accounts"][1]["account"], "current");
        assert!(decoded["accounts"][1].get("observedAt").is_none());
    }

    // Provider 범위가 계정 하나만 포함해도 machine consumer가 예측할 수 있도록 목록 envelope를
    // 유지합니다.
    #[test]
    fn provider_query_keeps_the_list_envelope_when_one_account_matches() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        );
        let records = [record(snapshot)];
        let query = AccountQuery::Provider(ProviderId::new("kimi").unwrap());

        let output = render_for_query(&records, &query, &[]).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["schema"], "yo.account-capacity-list/v1alpha2");
        assert_eq!(decoded["accounts"].as_array().unwrap().len(), 1);
        assert!(decoded.get("provider").is_none());
    }

    // 사람이 읽는 email label과 stable 내부 AccountId를 JSON에서 서로 다른 필드로 보존합니다.
    #[test]
    fn exposes_email_label_and_internal_account_id_separately() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("0123456789abcdef").unwrap(),
            Vec::new(),
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_account_label("person@example.test")
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let records = [AccountCapacityRecord {
            target: AccountTarget::Exact(report.coordinate()),
            report: Some(report),
        }];

        let output = render_for_query(
            &records,
            &AccountQuery::Exact(
                records[0]
                    .target
                    .exact_coordinate()
                    .expect("exact JSON test target")
                    .clone(),
            ),
            &[],
        )
        .unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["account"], "person@example.test");
        assert_eq!(decoded["accountId"], "0123456789abcdef");
    }

    // refresh 중 일부 오류는 성공 결과와 독립적인 구조화된 errors 배열로 전달합니다.
    #[test]
    fn keeps_refresh_failures_machine_readable() {
        let failures = [AccountRefreshFailure {
            target: "Local Grok".to_owned(),
            message: "login required".to_owned(),
        }];

        let output = render_for_query(
            &[AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            }],
            &AccountQuery::Provider(ProviderId::new("grok").unwrap()),
            &failures,
        )
        .unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["errors"][0]["target"], "Local Grok");
        assert_eq!(decoded["errors"][0]["message"], "login required");
    }

    // 아직 관측되지 않은 exact 계정도 빈 limits와 timestamp 부재를 가진 정상 JSON 결과로 남깁니다.
    #[test]
    fn keeps_single_unobserved_account_on_the_single_result_schema() {
        let coordinate = AccountCoordinate::new("kimi", "default").unwrap();
        let output = render(&[AccountCapacityRecord {
            target: AccountTarget::Exact(coordinate),
            report: None,
        }])
        .unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(decoded["schema"], "yo.account-capacity/v1alpha3");
        assert_eq!(decoded["provider"], "kimi");
        assert_eq!(decoded["account"], "default");
        assert_eq!(decoded["limits"], json!([]));
        assert!(decoded.get("observedAt").is_none());
    }

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

        let output = render(&[record(snapshot)]).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(output.lines().count(), 1);
        assert!(output.ends_with('\n'));
        assert_eq!(
            decoded,
            json!({
                "schema": "yo.account-capacity/v1alpha3",
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

        let output = render(&[record(snapshot)]).unwrap();
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

        let output = render(&[record(snapshot)]).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
        let window = &decoded["limits"][0]["windows"][0];

        assert_eq!(window["usedPercent"], 33.34);
        assert_eq!(window["remainingPercent"], 66.66);
        assert_eq!(window["used"], 1);
        assert_eq!(window["limit"], 3);
    }
}
