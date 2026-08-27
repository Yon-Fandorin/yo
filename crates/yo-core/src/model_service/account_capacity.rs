use super::{AccountId, ModelServiceError, ProviderId};

/// One Provider-and-Account capacity observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountCapacitySnapshot {
    provider: ProviderId,
    account: AccountId,
    buckets: Vec<AccountCapacityBucket>,
}

impl AccountCapacitySnapshot {
    pub fn new(
        provider: ProviderId,
        account: AccountId,
        buckets: Vec<AccountCapacityBucket>,
    ) -> Self {
        Self {
            provider,
            account,
            buckets,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub fn buckets(&self) -> &[AccountCapacityBucket] {
        &self.buckets
    }
}

/// One independently metered capacity bucket reported by an Account source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountCapacityBucket {
    id: Option<String>,
    name: Option<String>,
    plan: Option<String>,
    primary: Option<AccountCapacityWindow>,
    secondary: Option<AccountCapacityWindow>,
    credits: Option<AccountCredits>,
    limit_reason: Option<String>,
}

impl AccountCapacityBucket {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Option<String>,
        name: Option<String>,
        plan: Option<String>,
        primary: Option<AccountCapacityWindow>,
        secondary: Option<AccountCapacityWindow>,
        credits: Option<AccountCredits>,
        limit_reason: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            plan,
            primary,
            secondary,
            credits,
            limit_reason,
        }
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn plan(&self) -> Option<&str> {
        self.plan.as_deref()
    }

    #[must_use]
    pub const fn primary(&self) -> Option<&AccountCapacityWindow> {
        self.primary.as_ref()
    }

    #[must_use]
    pub const fn secondary(&self) -> Option<&AccountCapacityWindow> {
        self.secondary.as_ref()
    }

    #[must_use]
    pub const fn credits(&self) -> Option<&AccountCredits> {
        self.credits.as_ref()
    }

    #[must_use]
    pub fn limit_reason(&self) -> Option<&str> {
        self.limit_reason.as_deref()
    }
}

/// One rolling capacity window, normalized to a bounded percentage from Provider-reported data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountCapacityWindow {
    used_percent: u8,
    window_duration_minutes: Option<u64>,
    resets_at_unix_seconds: Option<i64>,
}

impl AccountCapacityWindow {
    pub fn new(
        used_percent: u8,
        window_duration_minutes: Option<u64>,
        resets_at_unix_seconds: Option<i64>,
    ) -> Result<Self, ModelServiceError> {
        if used_percent > 100 {
            return Err(ModelServiceError::new(
                "account capacity used percent must be between 0 and 100",
            ));
        }
        Ok(Self {
            used_percent,
            window_duration_minutes,
            resets_at_unix_seconds,
        })
    }

    pub fn from_usage_ratio(
        used: u64,
        limit: u64,
        window_duration_minutes: Option<u64>,
        resets_at_unix_seconds: Option<i64>,
    ) -> Result<Self, ModelServiceError> {
        if limit == 0 {
            return Err(ModelServiceError::new(
                "account capacity limit must be positive",
            ));
        }
        let used = u128::from(used).min(u128::from(limit));
        let limit = u128::from(limit);
        let used_percent = if used == 0 {
            0
        } else {
            ((used * 100).div_ceil(limit)).min(100) as u8
        };
        Self::new(
            used_percent,
            window_duration_minutes,
            resets_at_unix_seconds,
        )
    }

    #[must_use]
    pub const fn used_percent(self) -> u8 {
        self.used_percent
    }

    #[must_use]
    pub const fn remaining_percent(self) -> u8 {
        100 - self.used_percent
    }

    #[must_use]
    pub const fn window_duration_minutes(self) -> Option<u64> {
        self.window_duration_minutes
    }

    #[must_use]
    pub const fn resets_at_unix_seconds(self) -> Option<i64> {
        self.resets_at_unix_seconds
    }
}

/// Optional account credit state reported alongside rolling windows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountCredits {
    balance: Option<String>,
    has_credits: bool,
    unlimited: bool,
}

impl AccountCredits {
    pub fn new(balance: Option<String>, has_credits: bool, unlimited: bool) -> Self {
        Self {
            balance,
            has_credits,
            unlimited,
        }
    }

    #[must_use]
    pub fn balance(&self) -> Option<&str> {
        self.balance.as_deref()
    }

    #[must_use]
    pub const fn has_credits(&self) -> bool {
        self.has_credits
    }

    #[must_use]
    pub const fn unlimited(&self) -> bool {
        self.unlimited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Provider가 보고한 사용률만 받아 남은 비율을 정확한 보수 산술로 계산하고,
    // 100%를 넘는 값은 잔여량처럼 보이는 잘못된 값으로 투영하지 않습니다.
    #[test]
    fn capacity_window_derives_only_bounded_remaining_percent() {
        let window = AccountCapacityWindow::new(37, Some(300), Some(1_800_000_000)).unwrap();

        assert_eq!(window.used_percent(), 37);
        assert_eq!(window.remaining_percent(), 63);
        assert_eq!(window.window_duration_minutes(), Some(300));
        assert_eq!(window.resets_at_unix_seconds(), Some(1_800_000_000));
        assert!(AccountCapacityWindow::new(101, None, None).is_err());
    }

    // Count-based Providers normalize conservatively: the displayed remaining percentage never
    // exceeds the exact remaining ratio, and overage saturates instead of wrapping or rejecting a
    // still meaningful exhausted window.
    #[test]
    fn capacity_window_normalizes_provider_usage_counts() {
        let partial = AccountCapacityWindow::from_usage_ratio(1, 3, Some(300), None).unwrap();
        assert_eq!(partial.used_percent(), 34);
        assert_eq!(partial.remaining_percent(), 66);

        let exhausted = AccountCapacityWindow::from_usage_ratio(110, 100, None, None).unwrap();
        assert_eq!(exhausted.used_percent(), 100);
        assert!(AccountCapacityWindow::from_usage_ratio(0, 0, None, None).is_err());
    }
}
