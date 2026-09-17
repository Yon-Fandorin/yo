//! 직접 입력 승인, 질문 진행, 영수증 처리를 위한 파사드.

mod direct;
mod questions;
mod receipts;
mod response;

use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityRequestRef, ActivityResponse, BackendCommandEvidence, BackendFailure, UserInput,
};

use super::state::Backend;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn validate_direct_input(
        &mut self,
        input: &UserInput,
    ) -> Result<(), BackendFailure> {
        direct::validate_direct_input(self, input)
    }

    pub(super) fn require_image_capability(
        &mut self,
        expected_model: &str,
    ) -> Result<(), BackendFailure> {
        direct::require_image_capability(self, expected_model)
    }

    pub(super) fn refresh_model_capability(&mut self, model: &str) {
        direct::refresh_model_capability(self, model);
    }

    pub(super) fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        response::respond_to_activity(self, request, response)
    }
}

pub(super) fn project_input(input: &UserInput) -> Result<Vec<Value>, BackendFailure> {
    direct::project_input(input)
}
