use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use yo_core::{BackendFailure, BackendFailureKind, QuestionChoice};

use super::super::state::{InputQuestion, InputQuestions};
use crate::protocol;

impl InputQuestions {
    pub(in crate::runtime) fn parse(params: &Value) -> Result<Self, BackendFailure> {
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .filter(|questions| !questions.is_empty())
            .ok_or_else(|| protocol::protocol_failure("user-input request has no questions"))?;
        let mut ids = HashSet::new();
        let mut parsed = Vec::with_capacity(questions.len());
        for question in questions {
            if question
                .get("isSecret")
                .is_some_and(|secret| secret != &Value::Bool(false))
            {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "secret input requires a secure editor; yo will not echo it into the transcript",
                ));
            }
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
            });
        }
        Ok(Self {
            captured_answers: vec![None; parsed.len()],
            questions: parsed,
            current: 0,
            answers: Map::new(),
            drafts: HashMap::new(),
            capture: None,
        })
    }
}
