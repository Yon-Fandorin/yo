//! Complete-request planning counts; measured provider usage remains separate.

use yo_core::{
    BackendFailure, BackendFailureKind, ContextAccounting, ContextAccountingQuality,
    ContextCheckpointProposal, ModelConnectorRequest, VersionedProfileId,
};

use super::{NativeModelBackend, failure, map_connector_turn};

#[derive(Clone, Debug)]
pub(super) struct InputCount {
    tokens: u64,
    accounting: Option<ContextAccounting>,
}

impl InputCount {
    pub(super) const fn planning_tokens(&self) -> u64 {
        self.tokens
    }
    pub(super) fn accounting(&self) -> Option<&ContextAccounting> {
        self.accounting.as_ref()
    }

    pub(super) fn bind_checkpoint(
        &self,
        proposal: ContextCheckpointProposal,
        after: &Self,
    ) -> Result<ContextCheckpointProposal, BackendFailure> {
        match (&self.accounting, &after.accounting) {
            (None, None) => Ok(proposal),
            (Some(before), Some(after)) => proposal
                .with_accounting(before.clone(), after.clone())
                .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail)),
            _ => Err(failure(
                BackendFailureKind::Protocol,
                "context accounting policy changed within one binding",
            )),
        }
    }
}

impl NativeModelBackend {
    pub(super) fn count_request_input(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<InputCount, BackendFailure> {
        if self.image_accounting.is_none() && request.image_count() > 0 {
            return Err(failure(
                BackendFailureKind::Unsupported,
                "image input requires the complete binding's reviewed image accounting profile",
            ));
        }
        let payload = self
            .connector
            .tokenization_payload(request)
            .map_err(map_connector_turn)?;
        let text_tokens = self
            .token_counter
            .count_input_tokens(self.model_context.tokenizer_profile(), &payload)
            .map_err(|_| {
                failure(
                    BackendFailureKind::Turn,
                    "model token counting failed before remote request dispatch",
                )
            })?;
        request_count(
            text_tokens,
            request.image_count(),
            self.image_accounting.as_ref(),
        )
    }
}

fn request_count(
    text_tokens: u64,
    image_count: usize,
    image_accounting: Option<&VersionedProfileId>,
) -> Result<InputCount, BackendFailure> {
    let Some(policy) = image_accounting else {
        return Ok(InputCount {
            tokens: text_tokens,
            accounting: None,
        });
    };
    let estimate = u64::try_from(image_count)
        .ok()
        .and_then(|images| images.checked_mul(2000))
        .and_then(|images| text_tokens.checked_add(images))
        .ok_or_else(|| {
            failure(
                BackendFailureKind::ContextExhausted,
                "image input estimate overflow",
            )
        })?;
    let accounting = ContextAccounting::new(
        ContextAccountingQuality::AdvisoryEstimate,
        policy.clone(),
        estimate,
        if image_count == 0 { 0 } else { 1024 },
    )
    .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail))?;
    Ok(InputCount {
        tokens: accounting.planning_tokens(),
        accounting: Some(accounting),
    })
}

#[cfg(test)]
mod tests {
    use super::request_count;
    fn policy() -> yo_core::VersionedProfileId {
        yo_core::VersionedProfileId::new(yo_core::KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap()
    }

    // 전체 요청에 한 번만 reserve를 더하며 이미지가 없어도 선택된 추정 정책은 유지한다.
    #[test]
    fn counts_occurrences_and_charges_one_reserve() {
        let count = request_count(300, 3, Some(&policy())).unwrap();
        assert_eq!(count.planning_tokens(), 7324);
        assert_eq!(count.accounting().unwrap().input_estimate(), 6300);
        assert_eq!(count.accounting().unwrap().reserve_tokens(), 1024);
        let empty = request_count(300, 0, Some(&policy())).unwrap();
        assert_eq!(empty.planning_tokens(), 300);
        assert_eq!(empty.accounting().unwrap().reserve_tokens(), 0);
        assert!(request_count(300, 0, None).unwrap().accounting().is_none());
    }

    // 추정값과 reserve의 첫 초과도 조용히 포화시키지 않고 요청 전에 거절한다.
    #[test]
    fn rejects_estimate_and_reserve_overflow() {
        assert!(request_count(u64::MAX, 1, Some(&policy())).is_err());
        assert!(request_count(u64::MAX - 2000, 1, Some(&policy())).is_err());
        assert_eq!(
            request_count(u64::MAX, 0, Some(&policy()))
                .unwrap()
                .planning_tokens(),
            u64::MAX
        );
    }
}
