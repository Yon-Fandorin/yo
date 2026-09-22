use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use yo_core::{BackendFailure, BackendFailureKind, QuestionChoice, SecretInput};

use super::super::state::{InputQuestion, InputQuestions};
use crate::protocol;

impl InputQuestions {
    const MAX_RETAINED_SECRET_BYTES: usize = 256 * 1024;
    const RETAINED_SECRET_OVER_BUDGET: &'static str =
        "retained secret input exceeds the 256 KiB batch limit";

    pub(in crate::runtime) fn parse(params: &Value) -> Result<Self, BackendFailure> {
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .filter(|questions| !questions.is_empty())
            .ok_or_else(|| protocol::protocol_failure("user-input request has no questions"))?;
        let mut ids = HashSet::new();
        let mut parsed = Vec::with_capacity(questions.len());
        for question in questions {
            let is_secret = match question.get("isSecret") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(protocol::protocol_failure(
                        "question isSecret must be boolean",
                    ));
                },
            };
            let other = match question.get("isOther") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(protocol::protocol_failure(
                        "question isOther must be boolean",
                    ));
                },
            };
            let id = protocol::string_at(question, &["id"])?;
            if id.is_empty() || !ids.insert(id) {
                return Err(protocol::protocol_failure(
                    "user-input question IDs must be nonempty and unique",
                ));
            }
            let header = protocol::string_at(question, &["header"])?;
            let body = protocol::string_at(question, &["question"])?;
            let mut prompt = format!("{header}\n\n{body}");
            let mut options = Vec::new();
            let mut choices = Vec::new();
            if let Some(values) = question.get("options").filter(|value| !value.is_null()) {
                let values = values.as_array().ok_or_else(|| {
                    protocol::protocol_failure("question options must be an array")
                })?;
                for value in values {
                    let label = protocol::string_at(value, &["label"])?;
                    let description = protocol::string_at(value, &["description"])?;
                    options.push(label.to_owned());
                    choices.push(QuestionChoice {
                        label: label.to_owned(),
                        description: description.to_owned(),
                    });
                    prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
                }
            }
            if is_secret && (other || !options.is_empty()) {
                return Err(protocol::protocol_failure(
                    "secret questions cannot offer choices or isOther",
                ));
            }
            if other && !options.is_empty() {
                let label = "None of the above";
                let description = "Choose an answer outside this list.";
                options.push(label.to_owned());
                choices.push(QuestionChoice {
                    label: label.to_owned(),
                    description: description.to_owned(),
                });
                prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
            }
            parsed.push(InputQuestion {
                id: id.to_owned(),
                prompt,
                question: body.to_owned(),
                options,
                choices,
                is_secret,
            });
        }
        Ok(Self {
            captured_answers: vec![None; parsed.len()],
            questions: parsed,
            current: 0,
            answers: Map::new(),
            drafts: HashMap::new(),
            capture: None,
            secret_delivery_blocked: false,
            secret_tool: None,
        })
    }

    pub(in crate::runtime) fn has_secret(&self) -> bool {
        self.questions.iter().any(|question| question.is_secret)
    }

    pub(in crate::runtime) fn discard_secret_values(&mut self, block_delivery: bool) {
        for index in 0..self.questions.len() {
            self.discard_secret_value(index);
        }
        if block_delivery {
            self.secret_delivery_blocked = true;
        }
    }

    pub(in crate::runtime) fn discard_secret_value(&mut self, index: usize) {
        let Some((is_secret, id)) = self
            .questions
            .get(index)
            .map(|question| (question.is_secret, question.id.clone()))
        else {
            return;
        };
        if !is_secret {
            return;
        }
        self.answers.remove(&id);
        self.drafts.remove(&id);
        if let Some(answer) = self.captured_answers.get_mut(index) {
            *answer = None;
        }
    }

    pub(in crate::runtime) fn retain_secret(
        &mut self,
        index: usize,
        input: &SecretInput,
    ) -> Result<(), BackendFailure> {
        let Some((is_secret, id)) = self
            .questions
            .get(index)
            .map(|question| (question.is_secret, question.id.clone()))
        else {
            return Err(protocol::protocol_failure(
                "secret question index is invalid",
            ));
        };
        if !is_secret {
            return Err(protocol::protocol_failure(
                "secret input response targets a non-secret question",
            ));
        }
        let retained = self
            .answers
            .iter()
            .filter(|(question_id, _)| question_id.as_str() != id.as_str())
            .filter_map(|(question_id, value)| {
                self.questions
                    .iter()
                    .find(|question| {
                        question.is_secret && question.id.as_str() == question_id.as_str()
                    })
                    .and_then(|_| value.get("answers"))
                    .and_then(Value::as_array)
                    .and_then(|answers| answers.first())
                    .and_then(Value::as_str)
                    .map(str::len)
            })
            .try_fold(input.expose().len(), |total, length| {
                total.checked_add(length)
            })
            .ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::InputOverBudget,
                    Self::RETAINED_SECRET_OVER_BUDGET,
                )
            })?;
        if retained > Self::MAX_RETAINED_SECRET_BYTES {
            return Err(BackendFailure::new(
                BackendFailureKind::InputOverBudget,
                Self::RETAINED_SECRET_OVER_BUDGET,
            ));
        }
        self.answers
            .insert(id, serde_json::json!({"answers": [input.expose()]}));
        Ok(())
    }
}
