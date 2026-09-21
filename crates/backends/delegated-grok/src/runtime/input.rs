mod response;

use serde_json::{Map, Value, json};
use yo_core::{
    ActivityQuestion, BackendFailure, ToolOutput,
    interview::{Capture, RECOVERY_UNAVAILABLE_RECEIPT_PREFIX},
};

use super::state::InputQuestions;
use crate::protocol;

impl InputQuestions {
    pub(super) fn question_profile(&self, index: usize) -> ActivityQuestion {
        let question = &self.questions[index];
        let hint = if question.choices.is_empty() {
            "Type your answer."
        } else {
            "Choose an option or type your own answer. Tab adds notes."
        };
        let submission = if self.questions.len() == 1 {
            "Submitting this answer sends your response.".to_owned()
        } else if index + 1 == self.questions.len() {
            format!(
                "Submitting this answer sends all {} answers.",
                self.questions.len()
            )
        } else {
            "This answer is recorded locally. All answers are sent after the final question."
                .to_owned()
        };
        let (draft_choice, draft) = &self.drafts[index];
        ActivityQuestion {
            plain_text: format!(
                "Question {} of {} · {}\n\n{hint}\n{submission}\nEsc interrupts the turn.",
                index + 1,
                self.questions.len(),
                question.text,
            ),
            choices: question.choices.clone(),
            allow_notes: true,
            previous_question: index > 0,
            draft: (!draft.is_empty()).then(|| draft.clone()),
            draft_choice: *draft_choice,
            is_secret: false,
            storage_offer: None,
        }
    }

    pub(super) fn prompt(&self) -> String {
        if let Some(Capture::Batch {
            interview,
            revision,
            questions,
            ..
        }) = &self.capture
        {
            let capture = if self.current == 0
                && self.answers.iter().all(Option::is_none)
                && self
                    .drafts
                    .iter()
                    .all(|(choice, draft)| choice.is_none() && draft.is_empty())
            {
                self.capture.clone().expect("present capture")
            } else {
                Capture::Question {
                    interview: *interview,
                    revision: revision.clone(),
                    question: questions[self.current].clone(),
                    secret_batch: false,
                }
            };
            if let Ok(snapshot) = capture.to_snapshot() {
                return snapshot;
            }
        }
        let profile = self.question_profile(self.current);
        let mut text = profile.to_snapshot().unwrap_or(profile.plain_text);
        if self.capture.is_none() {
            text = text.replace(
                "Esc interrupts the turn.",
                "Complete interview recovery is unavailable. Esc interrupts the turn.",
            );
        }
        text
    }

    pub(super) fn receipt(&self, answer: &str, notes: Option<&str>) -> String {
        let progress = format!(
            "Question {} of {}\n",
            self.current + 1,
            self.questions.len()
        );
        let delivery = if self.current + 1 == self.questions.len() {
            "All question responses sent."
        } else {
            "Recorded; waiting for the remaining questions."
        };
        let parts = [
            progress.as_str(),
            self.questions[self.current].text.as_str(),
            "\n\nAnswer: ",
            answer,
            if notes.is_some() { "\nNote: " } else { "" },
            notes.unwrap_or(""),
            "\n\n",
            delivery,
        ];
        let size = parts
            .iter()
            .try_fold(0_usize, |size, part| size.checked_add(part.len()));
        if size.is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES) {
            return format!(
                "{progress}{delivery}\nAnswer display omitted: receipt exceeds the output limit. The original response remains in the session journal."
            );
        }
        let mut text = String::with_capacity(size.expect("bounded receipt size"));
        for part in parts {
            text.push_str(part);
        }
        text
    }

    pub(super) fn accepted_payload(&self) -> Result<Value, BackendFailure> {
        let mut answers = Map::new();
        let mut annotations = Map::new();
        for (question, answer) in self.questions.iter().zip(&self.answers) {
            let answer = answer.as_ref().ok_or_else(|| {
                protocol::protocol_failure("Grok user-question answer batch is incomplete")
            })?;
            answers.insert(question.text.clone(), json!([answer.label]));
            let mut annotation = Map::new();
            if let Some(preview) = &answer.preview {
                annotation.insert("preview".to_owned(), Value::String(preview.clone()));
            }
            if let Some(notes) = answer.notes.as_deref().filter(|notes| !notes.is_empty()) {
                annotation.insert("notes".to_owned(), Value::String(notes.to_owned()));
            }
            if !annotation.is_empty() {
                annotations.insert(question.text.clone(), Value::Object(annotation));
            }
        }
        let mut result = Map::new();
        result.insert("outcome".to_owned(), Value::String("accepted".to_owned()));
        result.insert("answers".to_owned(), Value::Object(answers));
        if !annotations.is_empty() {
            result.insert("annotations".to_owned(), Value::Object(annotations));
        }
        Ok(Value::Object(result))
    }

    pub(super) fn recovery_failure_receipt(&self, reason: &str, receipt: &str) -> String {
        format!("{RECOVERY_UNAVAILABLE_RECEIPT_PREFIX} {reason}\n{receipt}")
    }
}
