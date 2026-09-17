pub(crate) const CONTEXT_POLICY_PROFILE: &str = "yo.context-policy/v1alpha1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextStrategy {
    PortableSummaryV1Alpha1,
    ExactReplayOnlyV1Alpha1,
}
impl ContextStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PortableSummaryV1Alpha1 => "portable-summary/v1alpha1",
            Self::ExactReplayOnlyV1Alpha1 => "exact-replay-only/v1alpha1",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "portable-summary/v1alpha1" => Ok(Self::PortableSummaryV1Alpha1),
            "exact-replay-only/v1alpha1" => Ok(Self::ExactReplayOnlyV1Alpha1),
            _ => Err("context strategy is unsupported"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextPolicyChanged {
    policy_revision: u64,
    enabled: bool,
    strategy: ContextStrategy,
    warning_percent: u8,
    trigger_percent: u8,
    retained_raw_percent: Option<u8>,
    retained_raw_max_tokens: Option<u64>,
}

impl ContextPolicyChanged {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        policy_revision: u64,
        enabled: bool,
        strategy: ContextStrategy,
        warning_percent: u8,
        trigger_percent: u8,
        retained_raw_percent: Option<u8>,
        retained_raw_max_tokens: Option<u64>,
    ) -> Result<Self, &'static str> {
        let record = Self {
            policy_revision,
            enabled,
            strategy,
            warning_percent,
            trigger_percent,
            retained_raw_percent,
            retained_raw_max_tokens,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.policy_revision == 0 {
            return Err("context policy revision must be positive");
        }
        if !(1..=99).contains(&self.warning_percent)
            || !(2..=100).contains(&self.trigger_percent)
            || self.warning_percent >= self.trigger_percent
        {
            return Err("context policy warning and trigger bounds are invalid");
        }
        if self
            .retained_raw_percent
            .is_some_and(|value| !(1..=100).contains(&value))
            || self.retained_raw_max_tokens == Some(0)
        {
            return Err("context policy retained-raw bounds are invalid");
        }
        if self.strategy == ContextStrategy::ExactReplayOnlyV1Alpha1
            && (self.retained_raw_percent.is_some() || self.retained_raw_max_tokens.is_some())
        {
            return Err("exact-replay-only context policy forbids retained-raw bounds");
        }
        Ok(())
    }

    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub const fn strategy(&self) -> ContextStrategy {
        self.strategy
    }

    pub const fn warning_percent(&self) -> u8 {
        self.warning_percent
    }

    pub const fn trigger_percent(&self) -> u8 {
        self.trigger_percent
    }

    pub const fn retained_raw_percent(&self) -> Option<u8> {
        self.retained_raw_percent
    }

    pub const fn retained_raw_max_tokens(&self) -> Option<u64> {
        self.retained_raw_max_tokens
    }
}
