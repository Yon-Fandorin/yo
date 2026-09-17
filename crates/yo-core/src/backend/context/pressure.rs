use serde::{Deserialize, Serialize};

use super::accounting::ContextAccounting;

const CONTEXT_PRESSURE_SCHEMA: &str = "yo.context-pressure/v1alpha1";
const IMAGE_CONTEXT_PRESSURE_SCHEMA: &str = "yo.context-pressure/v2alpha1";

fn optional_present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    decoder: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(decoder).map(Some)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressureDecision {
    Admit,
    Compact,
    Reject,
}

/// 하나의 durable context-pressure Activity snapshot을 typed 값으로 해석한 결과입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextPressureObservation {
    input_tokens: u64,
    input_token_limit: u64,
    warning_percent: u8,
    trigger_percent: u8,
    decision: ContextPressureDecision,
    accounting: Option<ContextAccounting>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextPressureWire {
    schema: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_present"
    )]
    input_tokens: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_present"
    )]
    accounting: Option<ContextAccounting>,
    input_token_limit: u64,
    warning_percent: u8,
    trigger_percent: u8,
    decision: ContextPressureDecision,
}

impl ContextPressureObservation {
    pub fn new(
        input_tokens: u64,
        input_token_limit: u64,
        warning_percent: u8,
        trigger_percent: u8,
        decision: ContextPressureDecision,
    ) -> Result<Self, &'static str> {
        if input_token_limit == 0
            || !(1..=99).contains(&warning_percent)
            || !(2..=100).contains(&trigger_percent)
            || warning_percent >= trigger_percent
        {
            return Err("context pressure observation is invalid");
        }
        Ok(Self {
            input_tokens,
            input_token_limit,
            warning_percent,
            trigger_percent,
            decision,
            accounting: None,
        })
    }

    pub fn from_snapshot_json(value: &str) -> Option<Self> {
        let wire = serde_json::from_str::<ContextPressureWire>(value).ok()?;
        let input_tokens = match (&*wire.schema, wire.input_tokens, &wire.accounting) {
            (CONTEXT_PRESSURE_SCHEMA, Some(tokens), None) => tokens,
            (IMAGE_CONTEXT_PRESSURE_SCHEMA, None, Some(accounting)) => accounting.planning_tokens(),
            _ => return None,
        };
        let mut observation = Self::new(
            input_tokens,
            wire.input_token_limit,
            wire.warning_percent,
            wire.trigger_percent,
            wire.decision,
        )
        .ok()?;
        observation.accounting = wire.accounting;
        Some(observation)
    }

    /// 현재 binding에 이미지가 없어도 명시적 accounting을 보존합니다.
    pub fn with_accounting(mut self, accounting: ContextAccounting) -> Result<Self, &'static str> {
        if self.input_tokens != accounting.planning_tokens() {
            return Err("pressure planning total differs from its accounting");
        }
        self.accounting = Some(accounting);
        Ok(self)
    }

    /// legacy exact scalar profile에서만 없는 선택적 image-aware accounting입니다.
    pub const fn accounting(&self) -> Option<&ContextAccounting> {
        self.accounting.as_ref()
    }

    pub fn to_snapshot_json(&self) -> String {
        serde_json::to_string(&ContextPressureWire {
            schema: if self.accounting.is_some() {
                IMAGE_CONTEXT_PRESSURE_SCHEMA
            } else {
                CONTEXT_PRESSURE_SCHEMA
            }
            .to_owned(),
            input_tokens: self.accounting.is_none().then_some(self.input_tokens),
            accounting: self.accounting.clone(),
            input_token_limit: self.input_token_limit,
            warning_percent: self.warning_percent,
            trigger_percent: self.trigger_percent,
            decision: self.decision,
        })
        .expect("a bounded context pressure observation is JSON serializable")
    }

    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub const fn input_token_limit(&self) -> u64 {
        self.input_token_limit
    }

    pub const fn trigger_percent(&self) -> u8 {
        self.trigger_percent
    }

    pub const fn decision(&self) -> ContextPressureDecision {
        self.decision
    }
}
