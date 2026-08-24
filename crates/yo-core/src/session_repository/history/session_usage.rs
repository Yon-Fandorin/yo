use std::{collections::BTreeMap, fmt};

use serde_json::Value;

use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, TranscriptRecord,
};

pub const MANAGED_USAGE_SCHEMA: &str = "yo.model-usage-receipt/v1";
pub const GROK_USAGE_SCHEMA: &str = "grok.acp-prompt-usage-receipt/v1";
pub const CODEX_USAGE_SCHEMA: &str = "codex.app-server-token-usage-receipt/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionUsageProvider {
    Managed,
    Grok,
    Codex,
}

impl SessionUsageProvider {
    #[must_use]
    pub const fn schema(self) -> &'static str {
        match self {
            Self::Managed => MANAGED_USAGE_SCHEMA,
            Self::Grok => GROK_USAGE_SCHEMA,
            Self::Codex => CODEX_USAGE_SCHEMA,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionUsageSource {
    Managed {
        response_id: String,
        round: u64,
        provider: String,
        account: String,
        model: String,
        connector: String,
        api_dialect: String,
        base_url: String,
    },
    Grok {
        source_profile: String,
        prompt_request_id: u64,
    },
    Codex {
        source_profile: String,
        turn_id: String,
        model_context_window: Option<u64>,
    },
}

impl SessionUsageSource {
    #[must_use]
    pub const fn provider(&self) -> SessionUsageProvider {
        match self {
            Self::Managed { .. } => SessionUsageProvider::Managed,
            Self::Grok { .. } => SessionUsageProvider::Grok,
            Self::Codex { .. } => SessionUsageProvider::Codex,
        }
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.provider().schema()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageValue {
    Reported(u64),
    Absent,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsage {
    input_tokens: UsageValue,
    output_tokens: UsageValue,
    total_tokens: UsageValue,
    reasoning_tokens: UsageValue,
    cache_read_input_tokens: UsageValue,
    cache_write_input_tokens: UsageValue,
}

impl SessionUsage {
    #[must_use]
    pub const fn input_tokens(&self) -> UsageValue {
        self.input_tokens
    }

    #[must_use]
    pub const fn output_tokens(&self) -> UsageValue {
        self.output_tokens
    }

    #[must_use]
    pub const fn total_tokens(&self) -> UsageValue {
        self.total_tokens
    }

    #[must_use]
    pub const fn reasoning_tokens(&self) -> UsageValue {
        self.reasoning_tokens
    }

    #[must_use]
    pub const fn cache_read_input_tokens(&self) -> UsageValue {
        self.cache_read_input_tokens
    }

    #[must_use]
    pub const fn cache_write_input_tokens(&self) -> UsageValue {
        self.cache_write_input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageCoverage {
    Complete,
    Partial { reported: usize, total: usize },
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageAggregate {
    tokens: u64,
    coverage: UsageCoverage,
}

impl UsageAggregate {
    #[must_use]
    pub const fn tokens(self) -> Option<u64> {
        match self.coverage {
            UsageCoverage::Unavailable => None,
            UsageCoverage::Complete | UsageCoverage::Partial { .. } => Some(self.tokens),
        }
    }

    #[must_use]
    pub const fn coverage(self) -> UsageCoverage {
        self.coverage
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionUsageAggregates {
    input_tokens: UsageAggregate,
    output_tokens: UsageAggregate,
    total_tokens: UsageAggregate,
    reasoning_tokens: UsageAggregate,
    cache_read_input_tokens: UsageAggregate,
}

impl SessionUsageAggregates {
    #[must_use]
    pub const fn input_tokens(self) -> UsageAggregate {
        self.input_tokens
    }

    #[must_use]
    pub const fn output_tokens(self) -> UsageAggregate {
        self.output_tokens
    }

    #[must_use]
    pub const fn total_tokens(self) -> UsageAggregate {
        self.total_tokens
    }

    #[must_use]
    pub const fn reasoning_tokens(self) -> UsageAggregate {
        self.reasoning_tokens
    }

    #[must_use]
    pub const fn cache_read_input_tokens(self) -> UsageAggregate {
        self.cache_read_input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheReadShare {
    cache_read_tokens: u64,
    input_tokens: u64,
}

impl CacheReadShare {
    #[must_use]
    pub const fn cache_read_tokens(self) -> u64 {
        self.cache_read_tokens
    }

    #[must_use]
    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheReadSummary {
    cache_read_tokens: u64,
    input_tokens: u64,
    eligible_receipts: usize,
    total_receipts: usize,
}

impl CacheReadSummary {
    #[must_use]
    pub const fn cache_read_tokens(self) -> u64 {
        self.cache_read_tokens
    }

    #[must_use]
    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }

    #[must_use]
    pub const fn eligible_receipts(self) -> usize {
        self.eligible_receipts
    }

    #[must_use]
    pub const fn total_receipts(self) -> usize {
        self.total_receipts
    }

    #[must_use]
    pub const fn coverage(self) -> UsageCoverage {
        coverage(self.eligible_receipts, self.total_receipts)
    }

    #[must_use]
    pub const fn share(self) -> Option<CacheReadShare> {
        if self.input_tokens == 0 {
            None
        } else {
            Some(CacheReadShare {
                cache_read_tokens: self.cache_read_tokens,
                input_tokens: self.input_tokens,
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageReceipt {
    activity: ActivityRef,
    source: SessionUsageSource,
    usage: SessionUsage,
}

impl SessionUsageReceipt {
    #[must_use]
    pub const fn activity(&self) -> ActivityRef {
        self.activity
    }

    #[must_use]
    pub const fn source(&self) -> &SessionUsageSource {
        &self.source
    }

    #[must_use]
    pub const fn provider(&self) -> SessionUsageProvider {
        self.source.provider()
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.source.schema()
    }

    #[must_use]
    pub const fn usage(&self) -> &SessionUsage {
        &self.usage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageProjection {
    receipts: Vec<SessionUsageReceipt>,
    aggregates: SessionUsageAggregates,
    cache_read: CacheReadSummary,
}

impl SessionUsageProjection {
    #[must_use]
    pub fn receipts(&self) -> &[SessionUsageReceipt] {
        &self.receipts
    }

    #[must_use]
    pub const fn aggregates(&self) -> SessionUsageAggregates {
        self.aggregates
    }

    #[must_use]
    pub const fn cache_read(&self) -> CacheReadSummary {
        self.cache_read
    }

    #[must_use]
    pub const fn has_receipts(&self) -> bool {
        !self.receipts.is_empty()
    }

    pub fn from_records(records: &[TranscriptRecord]) -> Result<Self, SessionUsageError> {
        project_session_usage(records)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageError {
    activity: ActivityRef,
    schema: String,
    detail: String,
}

impl SessionUsageError {
    #[must_use]
    pub const fn activity(&self) -> ActivityRef {
        self.activity
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    fn overflow(activity: ActivityRef, schema: &'static str, field: &'static str) -> Self {
        Self {
            activity,
            schema: schema.to_owned(),
            detail: format!("{field} aggregate overflowed u64"),
        }
    }
}

impl fmt::Display for SessionUsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} receipt for activity {:?}: {}",
            self.schema, self.activity, self.detail
        )
    }
}

impl std::error::Error for SessionUsageError {}

#[derive(Default)]
struct PendingModelWork {
    text: Option<String>,
    final_update_was_snapshot: bool,
}

fn project_session_usage(
    records: &[TranscriptRecord],
) -> Result<SessionUsageProjection, SessionUsageError> {
    let mut pending = BTreeMap::<ActivityRef, PendingModelWork>::new();
    let mut receipts = Vec::new();

    for record in records {
        let TranscriptRecord::EventCommitted(event) = record else {
            continue;
        };
        match event {
            AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            } => {
                pending.insert(*activity, PendingModelWork::default());
            },
            AgentEvent::ActivityStarted { .. } => {},
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(work) = pending.get_mut(activity) else {
                    continue;
                };
                match update {
                    ActivityUpdate::TextSnapshot(text) => {
                        work.text = Some(text.clone());
                        work.final_update_was_snapshot = true;
                    },
                    ActivityUpdate::TextDelta(_) => {
                        work.final_update_was_snapshot = false;
                    },
                }
            },
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            } => {
                let Some(work) = pending.remove(activity) else {
                    continue;
                };
                if !work.final_update_was_snapshot {
                    continue;
                }
                let Some(text) = work.text else {
                    continue;
                };
                if let Some(receipt) = parse_receipt(&text, *activity)? {
                    receipts.push(receipt);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                pending.remove(activity);
            },
            AgentEvent::SessionCreated { .. }
            | AgentEvent::TurnStarted { .. }
            | AgentEvent::TurnFinished { .. } => {},
        }
    }

    build_projection(receipts)
}

fn build_projection(
    receipts: Vec<SessionUsageReceipt>,
) -> Result<SessionUsageProjection, SessionUsageError> {
    let total_receipts = receipts.len();
    let aggregates = SessionUsageAggregates {
        input_tokens: aggregate(&receipts, "input_tokens", |usage| usage.input_tokens)?,
        output_tokens: aggregate(&receipts, "output_tokens", |usage| usage.output_tokens)?,
        total_tokens: aggregate(&receipts, "total_tokens", |usage| usage.total_tokens)?,
        reasoning_tokens: aggregate(&receipts, "reasoning_tokens", |usage| {
            usage.reasoning_tokens
        })?,
        cache_read_input_tokens: aggregate(&receipts, "cache_read_input_tokens", |usage| {
            usage.cache_read_input_tokens
        })?,
    };

    let mut cache_read_tokens = 0_u64;
    let mut input_tokens = 0_u64;
    let mut eligible_receipts = 0_usize;
    for receipt in &receipts {
        let (UsageValue::Reported(cache_read), UsageValue::Reported(input)) = (
            receipt.usage.cache_read_input_tokens,
            receipt.usage.input_tokens,
        ) else {
            continue;
        };
        cache_read_tokens = cache_read_tokens.checked_add(cache_read).ok_or_else(|| {
            SessionUsageError::overflow(
                receipt.activity,
                receipt.schema(),
                "cache_read_input_tokens",
            )
        })?;
        input_tokens = input_tokens.checked_add(input).ok_or_else(|| {
            SessionUsageError::overflow(receipt.activity, receipt.schema(), "input_tokens")
        })?;
        eligible_receipts += 1;
    }

    Ok(SessionUsageProjection {
        receipts,
        aggregates,
        cache_read: CacheReadSummary {
            cache_read_tokens,
            input_tokens,
            eligible_receipts,
            total_receipts,
        },
    })
}

fn aggregate(
    receipts: &[SessionUsageReceipt],
    field: &'static str,
    value: impl Fn(&SessionUsage) -> UsageValue,
) -> Result<UsageAggregate, SessionUsageError> {
    let mut total = 0_u64;
    let mut reported = 0_usize;
    let mut count = 0_usize;
    for receipt in receipts {
        count += 1;
        if let UsageValue::Reported(tokens) = value(&receipt.usage) {
            total = total.checked_add(tokens).ok_or_else(|| {
                SessionUsageError::overflow(receipt.activity, receipt.schema(), field)
            })?;
            reported += 1;
        }
    }
    Ok(UsageAggregate {
        tokens: total,
        coverage: coverage(reported, count),
    })
}

const fn coverage(reported: usize, total: usize) -> UsageCoverage {
    match (reported, total) {
        (0, _) => UsageCoverage::Unavailable,
        (reported, total) if reported == total => UsageCoverage::Complete,
        (reported, total) => UsageCoverage::Partial { reported, total },
    }
}

fn parse_receipt(
    text: &str,
    activity: ActivityRef,
) -> Result<Option<SessionUsageReceipt>, SessionUsageError> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(None);
    };
    let Some(schema) = value.get("schema").and_then(Value::as_str) else {
        return Ok(None);
    };
    let result = match schema {
        MANAGED_USAGE_SCHEMA => parse_managed(&value),
        GROK_USAGE_SCHEMA => parse_grok(&value),
        CODEX_USAGE_SCHEMA => parse_codex(&value),
        _ => return Ok(None),
    };
    result
        .map(|(source, usage)| {
            Some(SessionUsageReceipt {
                activity,
                source,
                usage,
            })
        })
        .map_err(|detail| SessionUsageError {
            activity,
            schema: schema.to_owned(),
            detail,
        })
}

fn parse_managed(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let root = closed_object(
        value,
        "managed receipt",
        &[
            "schema",
            "response_id",
            "round",
            "provider",
            "account",
            "model",
            "connector",
            "api_dialect",
            "base_url",
            "usage",
            "cache_read_input_tokens",
        ],
    )?;
    let usage = closed_object_at(
        root,
        "usage",
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "reasoning_tokens",
        ],
    )?;
    Ok((
        SessionUsageSource::Managed {
            response_id: required_string(value, "response_id")?,
            round: required_root_u64(value, "round")?,
            provider: required_string(value, "provider")?,
            account: required_string(value, "account")?,
            model: required_string(value, "model")?,
            connector: required_string(value, "connector")?,
            api_dialect: required_string(value, "api_dialect")?,
            base_url: required_string(value, "base_url")?,
        },
        SessionUsage {
            input_tokens: optional_usage(usage, "input_tokens")?,
            output_tokens: optional_usage(usage, "output_tokens")?,
            total_tokens: optional_usage(usage, "total_tokens")?,
            reasoning_tokens: optional_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: parse_managed_cache(value.get("cache_read_input_tokens"))?,
            cache_write_input_tokens: UsageValue::Unsupported,
        },
    ))
}

fn parse_managed_cache(value: Option<&Value>) -> Result<UsageValue, String> {
    let Some(value) = value else {
        return Err("cache_read_input_tokens must be present".to_owned());
    };
    let object = value
        .as_object()
        .ok_or_else(|| "cache_read_input_tokens must be an object".to_owned())?;
    let availability = object
        .get("availability")
        .and_then(Value::as_str)
        .ok_or_else(|| "cache_read_input_tokens.availability must be a string".to_owned())?;
    match availability {
        "reported" => {
            validate_closed_fields(
                object,
                "cache_read_input_tokens",
                &["availability", "tokens", "source_profile"],
            )?;
            required_profile_id(object, "source_profile")?;
            Ok(UsageValue::Reported(
                object
                    .get("tokens")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        "cache_read_input_tokens.tokens must be a non-negative integer".to_owned()
                    })?,
            ))
        },
        "absent" => {
            validate_closed_fields(
                object,
                "cache_read_input_tokens",
                &["availability", "source_profile"],
            )?;
            required_profile_id(object, "source_profile")?;
            Ok(UsageValue::Absent)
        },
        "unsupported" => {
            validate_closed_fields(object, "cache_read_input_tokens", &["availability"])?;
            Ok(UsageValue::Unsupported)
        },
        availability => Err(format!(
            "unsupported cache_read_input_tokens availability {availability:?}"
        )),
    }
}

fn parse_grok(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let usage = object_at(value, "usage")?;
    Ok((
        SessionUsageSource::Grok {
            source_profile: required_string(value, "source_profile")?,
            prompt_request_id: required_root_u64(value, "prompt_request_id")?,
        },
        SessionUsage {
            input_tokens: required_usage(usage, "input_tokens")?,
            output_tokens: required_usage(usage, "output_tokens")?,
            total_tokens: required_usage(usage, "total_tokens")?,
            reasoning_tokens: required_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: required_usage(usage, "cache_read_input_tokens")?,
            cache_write_input_tokens: required_usage(usage, "cache_write_input_tokens")?,
        },
    ))
}

fn parse_codex(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let usage = object_at(value, "usage")?;
    Ok((
        SessionUsageSource::Codex {
            source_profile: required_string(value, "source_profile")?,
            turn_id: required_string(value, "turn_id")?,
            model_context_window: optional_root_u64(value, "model_context_window")?,
        },
        SessionUsage {
            input_tokens: required_usage(usage, "input_tokens")?,
            output_tokens: required_usage(usage, "output_tokens")?,
            total_tokens: required_usage(usage, "total_tokens")?,
            reasoning_tokens: required_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: required_usage(usage, "cache_read_input_tokens")?,
            cache_write_input_tokens: required_usage(usage, "cache_write_input_tokens")?,
        },
    ))
}

fn object_at<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} must be an object"))
}

fn closed_object<'a>(
    value: &'a Value,
    name: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{name} must be an object"))?;
    validate_closed_fields(object, name, allowed)?;
    Ok(object)
}

fn closed_object_at<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} must be an object"))?;
    validate_closed_fields(object, field, allowed)?;
    Ok(object)
}

fn validate_closed_fields(
    value: &serde_json::Map<String, Value>,
    name: &str,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(field) = value
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("{name} has unsupported field {field:?}"));
    }
    Ok(())
}

fn required_profile_id(value: &serde_json::Map<String, Value>, field: &str) -> Result<(), String> {
    let profile = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cache_read_input_tokens.{field} must be a string"))?;
    crate::VersionedProfileId::new(profile)
        .map(|_| ())
        .map_err(|_| format!("cache_read_input_tokens.{field} must be a versioned profile ID"))
}

fn required_usage(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<UsageValue, String> {
    Ok(UsageValue::Reported(required_u64(value, field)?))
}

fn optional_usage(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<UsageValue, String> {
    let Some(raw) = value.get(field) else {
        return Ok(UsageValue::Absent);
    };
    if raw.is_null() {
        return Ok(UsageValue::Absent);
    }
    required_usage(value, field)
}

fn required_u64(value: &serde_json::Map<String, Value>, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn required_root_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

fn optional_root_u64(value: &Value, field: &str) -> Result<Option<u64>, String> {
    let Some(raw) = value.get(field) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    raw.as_u64()
        .map(Some)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;

    // 알려진 사용량 영수증이 없는 기록은 성공하지만 모든 집계를 사용할 수 없음으로 남긴다.
    #[test]
    fn empty_history_has_unavailable_aggregates() {
        let projection = project_session_usage(&[]).unwrap();

        assert!(!projection.has_receipts());
        assert_eq!(
            projection.aggregates().input_tokens().coverage(),
            UsageCoverage::Unavailable
        );
        assert_eq!(
            projection.cache_read().coverage(),
            UsageCoverage::Unavailable
        );
    }

    // 관리형 영수증의 0, 선택적 부재, 제공자 미지원 상태를 서로 다른 값으로 보존한다.
    #[test]
    fn managed_receipt_preserves_zero_absent_and_unsupported() {
        let value = serde_json::json!({
            "schema": MANAGED_USAGE_SCHEMA,
            "response_id": "response-1",
            "round": 1,
            "provider": "managed",
            "account": "account-1",
            "model": "model-1",
            "connector": "connector-1",
            "api_dialect": "dialect-1",
            "base_url": "https://managed.invalid",
            "usage": {
                "input_tokens": 0,
                "output_tokens": 2,
                "total_tokens": 2,
                "reasoning_tokens": null
            },
            "cache_read_input_tokens": {
                "availability": "absent",
                "source_profile": "managed.cache-read/v1"
            }
        });
        let (source, usage) = parse_managed(&value).unwrap();
        let receipt = SessionUsageReceipt {
            activity: activity(1),
            source,
            usage,
        };

        assert_eq!(receipt.usage().input_tokens(), UsageValue::Reported(0));
        assert_eq!(receipt.usage().reasoning_tokens(), UsageValue::Absent);
        assert_eq!(
            receipt.usage().cache_read_input_tokens(),
            UsageValue::Absent
        );
        assert_eq!(
            receipt.usage().cache_write_input_tokens(),
            UsageValue::Unsupported
        );
    }

    // 관리형 Option token의 null과 누락은 absent로 보존하되 비정상 non-null은 거부한다.
    #[test]
    fn managed_optional_tokens_are_absent_but_invalid_values_fail() {
        let absent =
            serde_json::from_str::<Value>(&managed_text(None, None, None, None, "absent", 0))
                .unwrap();
        let (_, usage) = parse_managed(&absent).unwrap();
        assert_eq!(usage.input_tokens(), UsageValue::Absent);
        assert_eq!(usage.reasoning_tokens(), UsageValue::Absent);

        let mut malformed = serde_json::from_str::<Value>(&managed_text(
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            "absent",
            0,
        ))
        .unwrap();
        malformed["usage"]["input_tokens"] = serde_json::json!("not a number");
        assert!(parse_managed(&malformed).is_err());
    }

    // managed cache availability는 reported·absent·unsupported마다 허용 필드와 versioned
    // source profile이 다르므로 누락·초과·잘못된 profile을 모두 malformed로 닫습니다.
    #[test]
    fn managed_cache_availability_shape_is_closed() {
        let malformed = [
            serde_json::json!({"availability": "reported", "tokens": 1}),
            serde_json::json!({
                "availability": "reported",
                "tokens": 1,
                "source_profile": "managed.cache/v1",
                "extra": true
            }),
            serde_json::json!({
                "availability": "absent",
                "source_profile": "managed.cache/v1",
                "tokens": 1
            }),
            serde_json::json!({"availability": "absent"}),
            serde_json::json!({
                "availability": "unsupported",
                "source_profile": "managed.cache/v1"
            }),
            serde_json::json!({"availability": "unsupported", "tokens": 1}),
            serde_json::json!({
                "availability": "reported",
                "tokens": 1,
                "source_profile": "not-versioned"
            }),
        ];

        for cache in malformed {
            let mut receipt = serde_json::from_str::<Value>(&managed_text(
                Some(1),
                Some(1),
                Some(2),
                Some(0),
                "reported",
                0,
            ))
            .unwrap();
            receipt["cache_read_input_tokens"] = cache;
            assert!(parse_managed(&receipt).is_err(), "{receipt}");
        }
    }

    // recognized managed receipt의 root와 usage는 닫힌 형식이므로 알려지지 않은 필드를
    // 조용히 버리지 않고 전체 projection 오류로 되돌릴 parse 오류를 만듭니다.
    #[test]
    fn managed_receipt_rejects_unknown_root_and_usage_fields() {
        let mut root = serde_json::from_str::<Value>(&managed_text(
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            "reported",
            0,
        ))
        .unwrap();
        root["unexpected"] = serde_json::json!(true);
        assert!(parse_managed(&root).is_err());

        let mut usage = serde_json::from_str::<Value>(&managed_text(
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            "reported",
            0,
        ))
        .unwrap();
        usage["usage"]["unexpected"] = serde_json::json!(true);
        assert!(parse_managed(&usage).is_err());
    }

    // Codex 집계는 현재 턴 usage만 사용하고 누적 thread_total은 합산하지 않는다.
    #[test]
    fn codex_ignores_thread_total() {
        let value = serde_json::json!({
            "schema": CODEX_USAGE_SCHEMA,
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": "turn-1",
            "model_context_window": 8192,
            "usage": {
                "input_tokens": 3,
                "output_tokens": 4,
                "total_tokens": 7,
                "reasoning_tokens": 0,
                "cache_read_input_tokens": 1,
                "cache_write_input_tokens": 2
            },
            "thread_total": {
                "input_tokens": 300,
                "output_tokens": 400,
                "total_tokens": 700,
                "reasoning_tokens": 200,
                "cache_read_input_tokens": 100,
                "cache_write_input_tokens": 20
            }
        });
        let (source, usage) = parse_codex(&value).unwrap();
        let receipt = SessionUsageReceipt {
            activity: activity(2),
            source,
            usage,
        };

        assert_eq!(receipt.usage().input_tokens(), UsageValue::Reported(3));
        assert_eq!(receipt.usage().output_tokens(), UsageValue::Reported(4));
        assert_eq!(
            receipt.usage().cache_read_input_tokens(),
            UsageValue::Reported(1)
        );
    }

    // 일부 영수증만 필드를 보고하면 합계와 함께 부분 x/y 커버리지를 표시한다.
    #[test]
    fn aggregates_report_coverage_without_hiding_partial_sum() {
        let first = SessionUsageReceipt {
            activity: activity(1),
            source: managed_source(),
            usage: SessionUsage {
                input_tokens: UsageValue::Reported(2),
                output_tokens: UsageValue::Reported(3),
                total_tokens: UsageValue::Reported(5),
                reasoning_tokens: UsageValue::Absent,
                cache_read_input_tokens: UsageValue::Reported(0),
                cache_write_input_tokens: UsageValue::Unsupported,
            },
        };
        let second = SessionUsageReceipt {
            activity: activity(2),
            source: grok_source(),
            usage: SessionUsage {
                input_tokens: UsageValue::Reported(4),
                output_tokens: UsageValue::Reported(1),
                total_tokens: UsageValue::Reported(5),
                reasoning_tokens: UsageValue::Reported(1),
                cache_read_input_tokens: UsageValue::Absent,
                cache_write_input_tokens: UsageValue::Reported(2),
            },
        };
        let projection = build_projection(vec![first, second]).unwrap();

        assert_eq!(projection.aggregates().input_tokens().tokens(), Some(6));
        assert_eq!(
            projection.aggregates().reasoning_tokens().coverage(),
            UsageCoverage::Partial {
                reported: 1,
                total: 2
            }
        );
        assert_eq!(projection.cache_read().eligible_receipts(), 1);
        assert_eq!(projection.cache_read().total_receipts(), 2);
    }

    // 토큰 합계가 u64 범위를 넘으면 포화시키지 않고 typed overflow error로 닫는다.
    #[test]
    fn aggregation_overflow_is_reported_as_an_error() {
        let mut first_usage = SessionUsage {
            input_tokens: UsageValue::Reported(u64::MAX),
            output_tokens: UsageValue::Reported(0),
            total_tokens: UsageValue::Reported(0),
            reasoning_tokens: UsageValue::Unsupported,
            cache_read_input_tokens: UsageValue::Unsupported,
            cache_write_input_tokens: UsageValue::Unsupported,
        };
        let first = SessionUsageReceipt {
            activity: activity(1),
            source: managed_source(),
            usage: first_usage.clone(),
        };
        first_usage.input_tokens = UsageValue::Reported(1);
        let second = SessionUsageReceipt {
            activity: activity(2),
            source: managed_source(),
            usage: first_usage,
        };

        let error = build_projection(vec![first, second]).unwrap_err();

        assert_eq!(error.activity(), activity(2));
        assert_eq!(error.schema(), MANAGED_USAGE_SCHEMA);
        assert!(error.detail().contains("input_tokens"));
    }

    // 세 제공자의 완료된 ModelWork 수명주기를 ActivityRef와 영수증 순서대로 보존한다.
    #[test]
    fn lifecycle_projection_preserves_sources_activity_and_chronology() {
        let mut records = completed(
            activity(1),
            managed_text(Some(10), Some(2), Some(12), None, "reported", 0),
        );
        records.extend(completed(activity(2), grok_text(20)));
        records.extend(completed(activity(3), codex_text(30)));

        let projection = project_session_usage(&records).unwrap();

        assert_eq!(projection.receipts().len(), 3);
        assert_eq!(projection.receipts()[0].activity(), activity(1));
        assert_eq!(
            projection.receipts()[0].provider(),
            SessionUsageProvider::Managed
        );
        assert_eq!(projection.receipts()[0].source(), &managed_source());
        assert_eq!(projection.receipts()[1].activity(), activity(2));
        assert_eq!(
            projection.receipts()[1].provider(),
            SessionUsageProvider::Grok
        );
        assert_eq!(projection.receipts()[1].source(), &grok_source());
        assert_eq!(projection.receipts()[2].activity(), activity(3));
        assert_eq!(
            projection.receipts()[2].provider(),
            SessionUsageProvider::Codex
        );
        assert_eq!(projection.receipts()[2].source(), &codex_source());
    }

    // 미완료, 실패, 비영수증, 마지막 snapshot이 아닌 ModelWork는 집계하지 않는다.
    #[test]
    fn lifecycle_projection_ignores_non_completed_or_non_receipt_work() {
        let mut records = unfinished(
            activity(1),
            managed_text(Some(1), Some(1), Some(2), Some(0), "reported", 0),
        );
        records.extend(failed(activity(2), grok_text(2)));
        records.extend(completed(activity(3), "not a receipt".to_owned()));
        records.extend(completed_with_delta_after_snapshot(
            activity(4),
            codex_text(3),
        ));

        let projection = project_session_usage(&records).unwrap();

        assert!(!projection.has_receipts());
    }

    // 알려진 schema의 완료된 영수증 구조가 깨지면 부분 결과 대신 typed error를 반환한다.
    #[test]
    fn lifecycle_projection_rejects_malformed_known_schema() {
        let malformed = serde_json::json!({
            "schema": CODEX_USAGE_SCHEMA,
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": "turn-1",
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2
            }
        })
        .to_string();

        let error = project_session_usage(&completed(activity(1), malformed)).unwrap_err();

        assert_eq!(error.schema(), CODEX_USAGE_SCHEMA);
        assert!(error.detail().contains("total_tokens"));
    }

    // cache-read의 보고된 0, absent, unsupported를 보존하고 eligible 커버리지를 부분으로 표시한다.
    #[test]
    fn cache_read_zero_absent_and_unsupported_have_partial_coverage() {
        let mut records = completed(
            activity(1),
            managed_text(Some(10), Some(1), Some(11), Some(0), "reported", 0),
        );
        records.extend(completed(
            activity(2),
            managed_text(Some(20), Some(2), Some(22), Some(0), "absent", 0),
        ));
        records.extend(completed(
            activity(3),
            managed_text(Some(30), Some(3), Some(33), Some(0), "unsupported", 0),
        ));

        let projection = project_session_usage(&records).unwrap();

        assert_eq!(
            projection.receipts()[0].usage().cache_read_input_tokens(),
            UsageValue::Reported(0)
        );
        assert_eq!(
            projection.receipts()[1].usage().cache_read_input_tokens(),
            UsageValue::Absent
        );
        assert_eq!(
            projection.receipts()[2].usage().cache_read_input_tokens(),
            UsageValue::Unsupported
        );
        assert_eq!(
            projection.cache_read().coverage(),
            UsageCoverage::Partial {
                reported: 1,
                total: 3
            }
        );
        assert_eq!(
            projection.cache_read().share(),
            Some(CacheReadShare {
                cache_read_tokens: 0,
                input_tokens: 10,
            })
        );
    }

    fn activity(number: u64) -> ActivityRef {
        let session_id: crate::SessionId = "018f0a00-0000-7000-8000-000000000001".parse().unwrap();
        let turn = crate::TurnRef::new(
            session_id,
            crate::TurnId::new(NonZeroU64::new(number).unwrap()),
        );
        ActivityRef::new(
            turn,
            crate::ActivityId::new(NonZeroU64::new(number).unwrap()),
        )
    }

    fn managed_source() -> SessionUsageSource {
        SessionUsageSource::Managed {
            response_id: "response-1".to_owned(),
            round: 1,
            provider: "managed".to_owned(),
            account: "account-1".to_owned(),
            model: "model-1".to_owned(),
            connector: "connector-1".to_owned(),
            api_dialect: "dialect-1".to_owned(),
            base_url: "https://managed.invalid".to_owned(),
        }
    }

    fn grok_source() -> SessionUsageSource {
        SessionUsageSource::Grok {
            source_profile: "grok.acp.prompt-response.usage/v1".to_owned(),
            prompt_request_id: 42,
        }
    }

    fn codex_source() -> SessionUsageSource {
        SessionUsageSource::Codex {
            source_profile: "codex.app-server.thread-token-usage-updated/v1".to_owned(),
            turn_id: "turn-1".to_owned(),
            model_context_window: Some(8192),
        }
    }

    fn managed_text(
        input: Option<u64>,
        output: Option<u64>,
        total: Option<u64>,
        reasoning: Option<u64>,
        cache_availability: &str,
        cache_tokens: u64,
    ) -> String {
        let cache_read_input_tokens = match cache_availability {
            "reported" => serde_json::json!({
                "availability": "reported",
                "tokens": cache_tokens,
                "source_profile": "managed.cache-read/v1"
            }),
            "absent" => serde_json::json!({
                "availability": "absent",
                "source_profile": "managed.cache-read/v1"
            }),
            "unsupported" => serde_json::json!({
                "availability": "unsupported"
            }),
            availability => serde_json::json!({
                "availability": availability
            }),
        };
        serde_json::json!({
            "schema": MANAGED_USAGE_SCHEMA,
            "response_id": "response-1",
            "round": 1,
            "provider": "managed",
            "account": "account-1",
            "model": "model-1",
            "connector": "connector-1",
            "api_dialect": "dialect-1",
            "base_url": "https://managed.invalid",
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": total,
                "reasoning_tokens": reasoning
            },
            "cache_read_input_tokens": cache_read_input_tokens
        })
        .to_string()
    }

    fn grok_text(input: u64) -> String {
        serde_json::json!({
            "schema": GROK_USAGE_SCHEMA,
            "source_profile": "grok.acp.prompt-response.usage/v1",
            "prompt_request_id": 42,
            "usage": {
                "input_tokens": input,
                "output_tokens": 2,
                "total_tokens": input + 2,
                "reasoning_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_write_input_tokens": 0
            }
        })
        .to_string()
    }

    fn codex_text(input: u64) -> String {
        serde_json::json!({
            "schema": CODEX_USAGE_SCHEMA,
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": "turn-1",
            "model_context_window": 8192,
            "usage": {
                "input_tokens": input,
                "output_tokens": 2,
                "total_tokens": input + 2,
                "reasoning_tokens": 0,
                "cache_read_input_tokens": 1,
                "cache_write_input_tokens": 0
            },
            "thread_total": {
                "input_tokens": 900,
                "output_tokens": 900,
                "total_tokens": 1800,
                "reasoning_tokens": 900,
                "cache_read_input_tokens": 900,
                "cache_write_input_tokens": 900
            }
        })
        .to_string()
    }

    fn completed(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
        vec![
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
        ]
    }

    fn unfinished(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
        completed(activity, text)[..2].to_vec()
    }

    fn failed(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
        let mut records = completed(activity, text);
        records.pop();
        records.push(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(crate::Failure::new("failed")),
            },
        ));
        records
    }

    fn completed_with_delta_after_snapshot(
        activity: ActivityRef,
        text: String,
    ) -> Vec<TranscriptRecord> {
        let mut records = completed(activity, text);
        records.insert(
            records.len() - 1,
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextDelta("trailing delta".to_owned()),
            }),
        );
        records
    }
}
