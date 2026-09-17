use serde::{Deserialize, Serialize, de::Error};

use crate::VersionedProfileId;

/// 입력 이미지 기능과 분리된 검토 완료 advisory accounting identity입니다.
pub const OPENROUTER_FREE_IMAGE_ACCOUNTING_PROFILE: &str = "openrouter-free-image-advisory/v1";

/// General API 이미지 추정치는 이미지가 없는 요청에도 advisory로 유지됩니다.
pub const QWENCLOUD_GENERAL_IMAGE_ACCOUNTING_PROFILE: &str = "qwencloud-general-image-advisory/v1";

/// 입력 이미지 기능과 분리된 검토 완료 Kimi advisory accounting identity입니다.
pub const KIMI_CODE_IMAGE_ACCOUNTING_PROFILE: &str = "kimi-code-image-advisory/v1";

/// complete request planning 추정치의 신뢰 수준입니다.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextAccountingQuality {
    Exact,
    VerifiedUpperBound,
    AdvisoryEstimate,
}

/// Complete request planning 근거이며 두 수치 모두 provider 사용량을 측정한 값이 아닙니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextAccounting {
    quality: ContextAccountingQuality,
    policy: String,
    input_estimate: u64,
    reserve_tokens: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountingWire {
    quality: ContextAccountingQuality,
    policy: String,
    input_estimate: u64,
    reserve_tokens: u64,
}

impl<'de> Deserialize<'de> for ContextAccounting {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = AccountingWire::deserialize(decoder)?;
        Self::new(
            wire.quality,
            VersionedProfileId::new(wire.policy).map_err(Error::custom)?,
            wire.input_estimate,
            wire.reserve_tokens,
        )
        .map_err(Error::custom)
    }
}

impl ContextAccounting {
    /// 승인된 policy, 호환되는 quality와 overflow가 검사된 planning sum을 검증합니다.
    pub fn new(
        quality: ContextAccountingQuality,
        policy: VersionedProfileId,
        input_estimate: u64,
        reserve_tokens: u64,
    ) -> Result<Self, &'static str> {
        if !matches!(
            policy.as_str(),
            KIMI_CODE_IMAGE_ACCOUNTING_PROFILE
                | OPENROUTER_FREE_IMAGE_ACCOUNTING_PROFILE
                | QWENCLOUD_GENERAL_IMAGE_ACCOUNTING_PROFILE
        ) || quality != ContextAccountingQuality::AdvisoryEstimate
            || !matches!(reserve_tokens, 0 | 1024)
            || input_estimate.checked_add(reserve_tokens).is_none()
        {
            return Err("context accounting policy, quality, reserve or planning sum is invalid");
        }
        Ok(Self {
            quality,
            policy: policy.as_str().to_owned(),
            input_estimate,
            reserve_tokens,
        })
    }

    /// 이 추정치의 명시적 quality입니다.
    pub const fn quality(&self) -> ContextAccountingQuality {
        self.quality
    }

    /// 승인된 versioned accounting identity입니다.
    pub fn policy(&self) -> &str {
        &self.policy
    }

    /// 요청마다 한 번 적용되는 reserve 전의 complete request 추정치입니다.
    pub const fn input_estimate(&self) -> u64 {
        self.input_estimate
    }

    /// complete request에 한 번 적용되는 reserve입니다.
    pub const fn reserve_tokens(&self) -> u64 {
        self.reserve_tokens
    }

    /// planning admission에 사용하는 검사 완료 추정치와 reserve의 합입니다.
    pub const fn planning_tokens(&self) -> u64 {
        self.input_estimate + self.reserve_tokens
    }
}
