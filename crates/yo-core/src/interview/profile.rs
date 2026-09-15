use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CAPTURE_LIMIT, InterviewError, invalid, refs};
use crate::{ActivityQuestion, ActivityRef, ActivityRequestRef, ActivityResponse, QuestionChoice};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterviewOption {
    pub id: String,
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterviewQuestion {
    pub id: String,
    pub prompt: String,
    pub question: String,
    pub options: Vec<InterviewOption>,
    pub allow_free_text: bool,
    pub allow_notes: bool,
    pub is_secret: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub question_id: String,
    pub option_id: Option<String>,
    pub text: String,
    pub notes: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerResponse {
    pub question_id: String,
    #[serde(with = "refs::request")]
    pub request: ActivityRequestRef,
    #[serde(with = "refs::activity")]
    pub response_activity: ActivityRef,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Capture {
    Batch {
        #[serde(with = "refs::request")]
        interview: ActivityRequestRef,
        revision: String,
        questions: Vec<InterviewQuestion>,
        current_question_id: String,
    },
    Question {
        #[serde(with = "refs::request")]
        interview: ActivityRequestRef,
        revision: String,
        question: InterviewQuestion,
    },
    AcceptedAnswers {
        #[serde(with = "refs::request")]
        interview: ActivityRequestRef,
        revision: String,
        answers: Vec<Answer>,
        answer_responses: Vec<AnswerResponse>,
        #[serde(with = "refs::request")]
        final_request: ActivityRequestRef,
        #[serde(with = "refs::activity")]
        response_activity: ActivityRef,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    capture: Capture,
}
#[derive(Serialize)]
struct Revision<'a> {
    #[serde(with = "refs::request")]
    interview: ActivityRequestRef,
    questions: &'a [InterviewQuestion],
}

impl Capture {
    pub const SCHEMA: &'static str = "yo.interview-capture/v1";
    pub fn batch(
        interview: ActivityRequestRef,
        questions: Vec<InterviewQuestion>,
    ) -> Result<Self, InterviewError> {
        let current_question_id = questions
            .first()
            .ok_or_else(|| invalid("empty interview"))?
            .id
            .clone();
        let revision = revision(interview, &questions)?;
        let value = Self::Batch {
            interview,
            revision,
            questions,
            current_question_id,
        };
        value.to_snapshot()?;
        Ok(value)
    }
    pub fn source(&self) -> (ActivityRequestRef, &str) {
        match self {
            Self::Batch {
                interview,
                revision,
                ..
            }
            | Self::Question {
                interview,
                revision,
                ..
            }
            | Self::AcceptedAnswers {
                interview,
                revision,
                ..
            } => (*interview, revision),
        }
    }
    pub fn to_snapshot(&self) -> Result<String, InterviewError> {
        self.validate()?;
        let text = serde_json::to_string(&Envelope {
            schema: Self::SCHEMA.into(),
            capture: self.clone(),
        })
        .map_err(|e| invalid(e.to_string()))?;
        if text.len() > CAPTURE_LIMIT {
            return Err(invalid(
                "complete nonsecret interview recovery is unavailable: capture exceeds 1 MiB",
            ));
        }
        Ok(text)
    }
    pub fn from_snapshot(text: &str) -> Result<Self, InterviewError> {
        if text.len() > CAPTURE_LIMIT {
            return Err(invalid("interview capture exceeds 1 MiB"));
        }
        let value: Envelope = serde_json::from_str(text).map_err(|e| invalid(e.to_string()))?;
        if value.schema != Self::SCHEMA || value.capture.to_snapshot()? != text {
            return Err(invalid("unsupported or noncanonical interview capture"));
        }
        Ok(value.capture)
    }
    fn validate(&self) -> Result<(), InterviewError> {
        let (interview, rev) = self.source();
        if !valid_revision(rev) {
            return Err(invalid("invalid interview revision"));
        }
        match self {
            Self::Batch {
                questions,
                current_question_id,
                ..
            } => {
                let mut ids = HashSet::new();
                for q in questions {
                    q.validate()?;
                    if !ids.insert(&q.id) {
                        return Err(invalid("duplicate question id"));
                    }
                }
                if questions
                    .first()
                    .is_none_or(|q| &q.id != current_question_id)
                    || revision(interview, questions)? != rev
                {
                    return Err(invalid("batch identity does not match complete questions"));
                }
            },
            Self::Question { question, .. } => question.validate()?,
            Self::AcceptedAnswers {
                answers,
                answer_responses,
                final_request,
                response_activity,
                ..
            } => {
                if answers.is_empty()
                    || answers.len() != answer_responses.len()
                    || final_request.activity().turn() != interview.activity().turn()
                    || response_activity.turn() != interview.activity().turn()
                {
                    return Err(invalid("invalid answer correlation"));
                }
                for (answer, response) in answers.iter().zip(answer_responses) {
                    if answer.question_id != response.question_id
                        || response.request.activity().turn() != interview.activity().turn()
                        || response.response_activity.turn() != interview.activity().turn()
                    {
                        return Err(invalid("invalid answer reference"));
                    }
                }
                if answer_responses.last().is_none_or(|r| {
                    r.request != *final_request || r.response_activity != *response_activity
                }) {
                    return Err(invalid("final answer correlation does not match"));
                }
            },
        }
        Ok(())
    }
}
pub(super) fn valid_revision(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn revision(
    interview: ActivityRequestRef,
    questions: &[InterviewQuestion],
) -> Result<String, InterviewError> {
    let bytes = serde_json::to_vec(&Revision {
        interview,
        questions,
    })
    .map_err(|e| invalid(e.to_string()))?;
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{hex}"))
}
impl InterviewQuestion {
    pub(super) fn validate(&self) -> Result<(), InterviewError> {
        if self.id.is_empty()
            || self.is_secret
            || self.question.is_empty()
            || (!self.allow_free_text && self.options.is_empty())
        {
            return Err(invalid("unsupported or secret interview question"));
        }
        for (i, o) in self.options.iter().enumerate() {
            if o.id != (i + 1).to_string() || o.label.is_empty() {
                return Err(invalid("invalid ordered option"));
            }
        }
        Ok(())
    }
    pub fn empty_answer(&self) -> Answer {
        Answer {
            question_id: self.id.clone(),
            option_id: None,
            text: String::new(),
            notes: String::new(),
        }
    }
    /// Projects exact committed frontend values; backend wire trimming is not recovery authority.
    pub fn project_response(&self, response: &ActivityResponse) -> Result<Answer, InterviewError> {
        let mut answer = self.empty_answer();
        match response {
            ActivityResponse::QuestionAnswer { choice, notes } => {
                answer.option_id = Some(choice.to_string());
                answer.notes = notes.as_str().to_owned();
            },
            ActivityResponse::UserInput(input) => {
                if let Some(option) = input
                    .as_str()
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|v| v.checked_sub(1))
                    .and_then(|i| self.options.get(i))
                {
                    answer.option_id = Some(option.id.clone());
                } else {
                    answer.text = input.as_str().to_owned();
                }
            },
            _ => return Err(invalid("not an answering response")),
        }
        self.validate_answer(&answer, false)?;
        Ok(answer)
    }
    pub fn validate_answer(&self, answer: &Answer, editable: bool) -> Result<(), InterviewError> {
        if answer.question_id != self.id
            || answer
                .option_id
                .as_ref()
                .is_some_and(|id| !self.options.iter().any(|o| &o.id == id))
            || (answer.option_id.is_some() && !answer.text.is_empty())
            || (!self.allow_free_text && !answer.text.is_empty())
            || (!self.allow_notes && !answer.notes.is_empty())
            || (!editable && answer.option_id.is_none() && !self.allow_free_text)
        {
            return Err(invalid("answer does not match admitted question"));
        }
        Ok(())
    }
    pub fn presentation(&self, index: usize, total: usize) -> ActivityQuestion {
        let hint = if self.options.is_empty() {
            "Enter your answer."
        } else {
            "Enter a number or write your answer."
        };
        let submission = if total == 1 {
            "Submitting this answer sends your response.".into()
        } else if index + 1 == total {
            format!("Submitting this answer sends all {total} answers.")
        } else {
            "This answer is recorded locally. All answers are sent after the final question.".into()
        };
        ActivityQuestion {
            plain_text: format!(
                "Question {} of {} · {}\n\n{hint}\n{submission}\nEsc interrupts the turn.",
                index + 1,
                total,
                self.prompt,
            ),
            choices: if self.options.len() > 64 {
                Vec::new()
            } else {
                self.options
                    .iter()
                    .map(|o| QuestionChoice {
                        label: o.label.clone(),
                        description: o.description.clone(),
                    })
                    .collect()
            },
            allow_notes: self.allow_notes,
            previous_question: index > 0,
            draft: None,
            draft_choice: None,
        }
    }
}
