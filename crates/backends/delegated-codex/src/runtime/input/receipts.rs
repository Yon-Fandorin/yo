use yo_core::{ActivityNotice, ActivityQuestion, NoticeLevel, ToolOutput, interview::Capture};

use super::super::state::InputQuestions;

impl InputQuestions {
    pub(in crate::runtime) fn incomplete_notice(&self, reason: &str) -> String {
        let recorded = self.answers.len();
        let remaining = self.questions.len().saturating_sub(recorded);
        let base = format!(
            "{recorded}/{} answers recorded · {remaining} unanswered\nSubmission incomplete.\n{reason}",
            self.questions.len()
        );
        let fallback = || {
            ActivityNotice {
                level: NoticeLevel::Warning,
                title: "Interview incomplete".to_owned(),
                message: format!(
                    "{base}\nQuestion list omitted: summary exceeds the output limit."
                ),
            }
            .to_snapshot()
            .expect("bounded summary counts and static reason")
        };
        let mut message = base.clone();
        for (index, question) in self.questions.iter().enumerate() {
            let state = if question.is_secret {
                if self.answers.contains_key(&question.id) {
                    "Entered"
                } else {
                    "Not entered"
                }
            } else if self.answers.contains_key(&question.id) {
                "Recorded"
            } else {
                "Unanswered"
            };
            let prefix = format!("\n{}. {state}: ", index + 1);
            if message
                .len()
                .checked_add(prefix.len())
                .and_then(|size| size.checked_add(question.question.len()))
                .is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES)
            {
                return fallback();
            }
            message.push_str(&prefix);
            message.push_str(&question.question);
        }
        ActivityNotice {
            level: NoticeLevel::Warning,
            title: "Interview incomplete".to_owned(),
            message,
        }
        .to_snapshot()
        .unwrap_or_else(fallback)
    }

    pub(in crate::runtime) fn receipt(&self, answer: &str, notes: Option<&str>) -> String {
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
        let question = &self.questions[self.current].question;
        if self.questions[self.current].is_secret {
            return format!("{progress}Secret answer entered.\n\n{delivery}");
        }
        let parts = [
            progress.as_str(),
            question,
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

    pub(in crate::runtime) fn question_profile(&self, index: usize) -> ActivityQuestion {
        let question = &self.questions[index];
        let hint = if question.is_secret {
            "Enter your secret answer."
        } else if question.options.is_empty() {
            "Enter your answer."
        } else {
            "Enter a number or write your answer."
        };
        let submission = if self.probe_only {
            "Yo discards this sample value and sends only a fixed completion status to Codex."
                .to_owned()
        } else if self.questions.len() == 1 {
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
        let plain_text = format!(
            "Question {} of {} · {}\n\n{hint}\n{submission}\nEsc interrupts the turn.",
            index + 1,
            self.questions.len(),
            question.prompt
        );
        ActivityQuestion {
            allow_notes: !question.is_secret,
            is_secret: question.is_secret,
            storage_offer: None,
            previous_question: index > 0,
            draft: (!question.is_secret)
                .then(|| self.drafts.get(&question.id).map(|(_, text)| text.clone()))
                .flatten(),
            draft_choice: (!question.is_secret)
                .then(|| {
                    self.drafts
                        .get(&question.id)
                        .and_then(|(choice, _)| *choice)
                })
                .flatten(),
            plain_text,
            choices: if question.is_secret {
                Vec::new()
            } else {
                question.choices.clone()
            },
        }
    }

    pub(in crate::runtime) fn prompt(&self) -> String {
        if self.probe_only {
            let profile = self.question_profile(self.current);
            return profile.to_snapshot().unwrap_or(profile.plain_text);
        }
        if let Some(Capture::Batch {
            interview,
            revision,
            questions,
            ..
        }) = self.capture.as_deref()
        {
            let capture = if self.current == 0 && self.answers.is_empty() && self.drafts.is_empty()
            {
                self.capture.as_deref().cloned().expect("present capture")
            } else {
                Capture::Question {
                    interview: *interview,
                    revision: revision.clone(),
                    question: questions[self.current].clone(),
                    secret_batch: self.has_secret(),
                }
            };
            if let Ok(snapshot) = capture.to_snapshot() {
                return snapshot;
            }
        }
        let mut profile = self.question_profile(self.current);
        profile.previous_question = self.current > 0
            && self
                .question_profile(self.current - 1)
                .to_snapshot()
                .is_some();
        let mut text = profile.to_snapshot().unwrap_or(profile.plain_text);
        if self.capture.is_none() {
            // 레거시/과대 배치는 실시간으로 계속 동작하지만, 보이지 않는 질문은 재구성할 수
            // 없습니다.
            text = text.replace(
                "Esc interrupts the turn.",
                "Complete interview recovery is unavailable. Esc interrupts the turn.",
            );
        }
        text
    }
}
