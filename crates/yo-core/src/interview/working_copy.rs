use std::{collections::HashSet, mem};

use serde::{Deserialize, Serialize};

use super::{
    Answer, COPY_LIMIT, CapturedInterview, InterviewCatalog, InterviewError, SecretAnswerState,
    invalid, recovery::SecretRecoveryReference, refs,
};
use crate::{ActivityRef, ActivityRequestRef, SubmissionId, TurnRef};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    #[serde(with = "refs::request")]
    interview: ActivityRequestRef,
    revision: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Submission {
    ActivityResponse {
        #[serde(with = "refs::request")]
        final_request: ActivityRequestRef,
        #[serde(with = "refs::activity")]
        response_activity: ActivityRef,
    },
    NewConversation {
        #[serde(with = "refs::turn")]
        turn: TurnRef,
        submission_id: String,
        accepted_request_sequence: u64,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkingCopy {
    schema: String,
    pub copy_id: String,
    pub generation: u64,
    source: Source,
    pub answers: Vec<Answer>,
    pub current_question_id: String,
    pub context: String,
    pub submission: Option<Submission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) secret_recovery: Vec<SecretRecoveryReference>,
}
impl WorkingCopy {
    pub const SCHEMA: &'static str = "yo.interview-working-copy/v1";
    pub const SCHEMA_V2: &'static str = "yo.interview-working-copy/v2";
    pub const SCHEMA_V3: &'static str = "yo.interview-working-copy/v3";
    pub const DRAFT_SCHEMA: &'static str = "yo.interview-draft/v1";
    pub fn new(capture: &CapturedInterview) -> Result<Self, InterviewError> {
        let answers = capture
            .questions
            .iter()
            .zip(&capture.answers)
            .map(|(question, answer)| {
                if question.is_secret {
                    Answer::reentry_required_secret(question.id.clone())
                } else {
                    answer.clone()
                }
            })
            .collect();
        Ok(Self {
            schema: if capture.questions.iter().any(|q| q.is_secret) {
                Self::SCHEMA_V2.into()
            } else {
                Self::SCHEMA.into()
            },
            copy_id: new_id()?,
            generation: 1,
            source: Source {
                interview: capture.interview,
                revision: capture.revision.clone(),
            },
            answers,
            current_question_id: capture.current_question_id.clone(),
            context: String::new(),
            submission: capture.submitted.map(|(final_request, response_activity)| {
                Submission::ActivityResponse {
                    final_request,
                    response_activity,
                }
            }),
            secret_recovery: Vec::new(),
        })
    }
    pub fn new_contextual(capture: &CapturedInterview) -> Result<Self, InterviewError> {
        let mut copy = Self::new(capture)?;
        if copy.submission.is_some() {
            return Err(invalid("a submitted interview cannot become a new draft"));
        }
        copy.schema = Self::DRAFT_SCHEMA.into();
        Ok(copy)
    }
    pub fn is_contextual_draft(&self) -> bool {
        self.schema == Self::DRAFT_SCHEMA
    }
    pub fn source(&self) -> (ActivityRequestRef, &str) {
        (self.source.interview, &self.source.revision)
    }
    pub fn validate<'a>(
        &self,
        catalog: &'a InterviewCatalog,
    ) -> Result<&'a CapturedInterview, InterviewError> {
        self.validate_shape()?;
        let capture=catalog.find(self.source.interview,&self.source.revision).ok_or_else(||invalid("complete validated question capture is unavailable; this copy remains unchanged"))?;
        if self.answers.len() != capture.questions.len()
            || !capture
                .questions
                .iter()
                .any(|q| q.id == self.current_question_id)
        {
            return Err(invalid("working copy does not match ordered capture"));
        }
        for (q, a) in capture.questions.iter().zip(&self.answers) {
            q.validate_answer(a, true)?;
        }
        let expected_schema = if self.is_contextual_draft() {
            Self::DRAFT_SCHEMA
        } else if !self.secret_recovery.is_empty() {
            Self::SCHEMA_V3
        } else if capture.questions.iter().any(|q| q.is_secret) {
            Self::SCHEMA_V2
        } else {
            Self::SCHEMA
        };
        if self.schema != expected_schema {
            return Err(invalid(
                "working copy schema does not match captured interview",
            ));
        }
        if let Some(Submission::ActivityResponse {
            final_request,
            response_activity,
        }) = &self.submission
            && capture.submitted != Some((*final_request, *response_activity))
        {
            return Err(invalid(
                "working copy has no matching successful answer seal",
            ));
        }
        Ok(capture)
    }
    fn validate_shape(&self) -> Result<(), InterviewError> {
        if (self.schema != Self::SCHEMA
            && self.schema != Self::SCHEMA_V2
            && self.schema != Self::SCHEMA_V3
            && self.schema != Self::DRAFT_SCHEMA)
            || !valid_id(&self.copy_id)
            || self.generation == 0
            || !super::profile::valid_revision(&self.source.revision)
        {
            return Err(invalid("unsupported or invalid interview working copy"));
        }
        if self.is_contextual_draft() && !self.secret_recovery.is_empty() {
            return Err(invalid(
                "contextual interview drafts cannot contain secret recovery references",
            ));
        }
        if self.schema == Self::SCHEMA && self.answers.iter().any(Answer::is_secret) {
            return Err(invalid(
                "v1 interview working copies cannot contain secret answers",
            ));
        }
        if matches!(
            self.schema.as_str(),
            Self::SCHEMA_V2 | Self::SCHEMA_V3 | Self::DRAFT_SCHEMA
        ) && self
            .answers
            .iter()
            .any(|answer| answer.secret_state() == Some(SecretAnswerState::Submitted))
        {
            return Err(invalid(
                "working copies retain only the secret re-entry marker",
            ));
        }
        if matches!(
            self.schema.as_str(),
            Self::SCHEMA_V2 | Self::SCHEMA_V3 | Self::DRAFT_SCHEMA
        ) && matches!(self.submission, Some(Submission::NewConversation { .. }))
        {
            return Err(invalid(
                "secret interview working copies cannot contain a new-conversation submission",
            ));
        }
        if let Some(Submission::NewConversation {
            submission_id,
            accepted_request_sequence,
            ..
        }) = &self.submission
            && (!valid_id(submission_id) || *accepted_request_sequence == 0)
        {
            return Err(invalid("invalid conversation submission evidence"));
        }
        if self.schema != Self::SCHEMA_V3 && !self.secret_recovery.is_empty() {
            return Err(invalid(
                "only v3 interview working copies may contain recovery references",
            ));
        }
        if self.schema == Self::SCHEMA_V3 && self.secret_recovery.is_empty() {
            return Err(invalid(
                "v3 interview working copies require a recovery reference",
            ));
        }
        let mut question_ids = HashSet::new();
        let mut entry_ids = HashSet::new();
        for reference in &self.secret_recovery {
            reference.validate()?;
            if !question_ids.insert(reference.question_id.as_str())
                || !entry_ids.insert(reference.entry_id.as_str())
                || !self
                    .answers
                    .iter()
                    .any(|answer| answer.question_id == reference.question_id && answer.is_secret())
            {
                return Err(invalid("invalid or duplicate secret recovery reference"));
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, InterviewError> {
        self.validate_shape()?;
        let bytes = serde_json::to_vec(self).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > COPY_LIMIT {
            return Err(invalid(
                "interview copy exceeds 256 KiB; editable changes are retained",
            ));
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, InterviewError> {
        if bytes.len() > COPY_LIMIT {
            return Err(invalid("interview copy exceeds 256 KiB"));
        }
        let copy: Self = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if copy.encode()? != bytes {
            return Err(invalid("noncanonical interview copy"));
        }
        Ok(copy)
    }
    pub(super) fn recovery_reference(&self, question_id: &str) -> Option<&SecretRecoveryReference> {
        self.secret_recovery
            .iter()
            .find(|reference| reference.question_id == question_id)
    }

    #[must_use]
    pub fn has_secret_recovery(&self, question_id: &str) -> bool {
        self.recovery_reference(question_id).is_some()
    }

    #[must_use]
    pub fn has_any_secret_recovery(&self) -> bool {
        !self.secret_recovery.is_empty()
    }

    pub(super) fn set_recovery_reference(&mut self, reference: SecretRecoveryReference) {
        self.secret_recovery
            .retain(|current| current.question_id != reference.question_id);
        self.secret_recovery.push(reference);
        self.secret_recovery
            .sort_by(|left, right| left.question_id.cmp(&right.question_id));
        self.schema = Self::SCHEMA_V3.into();
    }

    pub(super) fn take_recovery_reference(
        &mut self,
        question_id: &str,
    ) -> Option<SecretRecoveryReference> {
        let index = self
            .secret_recovery
            .iter()
            .position(|reference| reference.question_id == question_id)?;
        let reference = self.secret_recovery.remove(index);
        if self.secret_recovery.is_empty() {
            self.schema = Self::SCHEMA_V2.into();
        }
        Some(reference)
    }

    pub(super) fn take_all_recovery_references(&mut self) -> Vec<SecretRecoveryReference> {
        let references = mem::take(&mut self.secret_recovery);
        if !references.is_empty() {
            self.schema = Self::SCHEMA_V2.into();
        }
        references
    }
}
pub(super) fn new_id() -> Result<String, InterviewError> {
    Ok(SubmissionId::new()
        .map_err(|e| invalid(e.to_string()))?
        .to_string())
}
pub(super) fn valid_id(value: &str) -> bool {
    value
        .parse::<SubmissionId>()
        .is_ok_and(|id| id.to_string() == value)
}
