use serde::{Deserialize, Serialize};

use super::{
    Answer, COPY_LIMIT, CapturedInterview, InterviewCatalog, InterviewError, PREVIEW_LIMIT,
    invalid, refs,
};
use crate::{ActivityRef, ActivityRequestRef, InputSubmission, SubmissionId, TurnRef, UserInput};

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
}
/// Immutable explicit first-Turn intent; callers retain it across backpressure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewConversation {
    pub copy_id: String,
    pub preview: String,
    pub submission: InputSubmission,
}
impl WorkingCopy {
    pub const SCHEMA: &'static str = "yo.interview-working-copy/v1";
    pub fn new(capture: &CapturedInterview) -> Result<Self, InterviewError> {
        Ok(Self {
            schema: Self::SCHEMA.into(),
            copy_id: new_id()?,
            generation: 1,
            source: Source {
                interview: capture.interview,
                revision: capture.revision.clone(),
            },
            answers: capture.answers.clone(),
            current_question_id: capture.current_question_id.clone(),
            context: String::new(),
            submission: capture.submitted.map(|(final_request, response_activity)| {
                Submission::ActivityResponse {
                    final_request,
                    response_activity,
                }
            }),
        })
    }
    pub fn reopen(&self) -> Result<Self, InterviewError> {
        let mut copy = self.clone();
        copy.copy_id = new_id()?;
        copy.generation = 1;
        copy.submission = None;
        Ok(copy)
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
        if self.schema != Self::SCHEMA
            || !valid_id(&self.copy_id)
            || self.generation == 0
            || !super::profile::valid_revision(&self.source.revision)
        {
            return Err(invalid("unsupported or invalid interview working copy"));
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
    pub fn preview(&self, catalog: &InterviewCatalog) -> Result<String, InterviewError> {
        let capture = self.validate(catalog)?;
        let mut text = String::from("Interview questions and editable answers\n\n");
        for (q, a) in capture.questions.iter().zip(&self.answers) {
            text.push_str(&q.prompt);
            text.push('\n');
            if let Some(id) = &a.option_id {
                let option = q
                    .options
                    .iter()
                    .find(|o| &o.id == id)
                    .expect("validated option");
                text.push_str(&format!("Answer: {}\n", option.label));
            } else {
                text.push_str(&format!("Answer: {}\n", a.text));
            }
            if !a.notes.is_empty() {
                text.push_str(&format!("Notes: {}\n", a.notes));
            }
            text.push('\n');
            if text.len() > PREVIEW_LIMIT {
                return Err(invalid(
                    "interview preview exceeds 64 KiB; no text was truncated",
                ));
            }
        }
        if !self.context.is_empty() {
            text.push_str("Additional context:\n");
            text.push_str(&self.context);
        }
        if text.len() > PREVIEW_LIMIT {
            return Err(invalid(
                "interview preview exceeds 64 KiB; no text was truncated",
            ));
        }
        Ok(text)
    }
    pub fn new_conversation(
        &self,
        catalog: &InterviewCatalog,
    ) -> Result<NewConversation, InterviewError> {
        if self.submission.is_some() {
            return Err(invalid(
                "reopen the submitted interview as a separate editable copy first",
            ));
        }
        let preview = self.preview(catalog)?;
        let id = SubmissionId::new().map_err(|e| invalid(e.to_string()))?;
        Ok(NewConversation {
            copy_id: self.copy_id.clone(),
            submission: InputSubmission::new(id, UserInput::new(preview.clone())),
            preview,
        })
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
