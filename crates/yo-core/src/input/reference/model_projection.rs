use serde_json::json;

use super::{
    error::UserInputError,
    model::{InputReference, ResolvedSkill, UserInput},
};
use crate::{ModelInputPart, journal::codec};

impl UserInput {
    /// 표시 입력과 reference span을 유지하면서 host가 검증한 skill snapshot을 추가합니다.
    /// live runtime admission은 caller가 제공한 snapshot을 authorization으로 사용하지 않습니다.
    pub fn with_resolved_skill(mut self, skill: ResolvedSkill) -> Result<Self, UserInputError> {
        let mut selected = self
            .references
            .iter()
            .filter_map(InputReference::skill_reference);
        if self.resolved_skill.is_some()
            || selected.next() != Some(skill.reference())
            || selected.next().is_some()
        {
            return Err(UserInputError::SkillSnapshotMismatch);
        }
        // 고정된 v2 input framing은 recovery에서도 model-visible request를 byte 단위로 재현합니다.
        let snapshot = json!({
            "name": skill.reference().name(),
            "source": skill.reference().locator(),
            "instructions": skill.instructions(),
        });
        self.model_text = Some(format!(
            "{}\n\nExplicit skill instructions (yo.skill-instructions/v1):\n{}",
            self.text, snapshot
        ));
        self.resolved_skill = Some(Box::new(skill));
        self.validate_image_encoding()?;
        Ok(self)
    }

    /// provider request, context accounting, replay가 사용하는 정확한 user-role text입니다.
    /// unresolved 및 과거 v1 input은 원래 text byte를 그대로 유지합니다.
    #[must_use]
    pub fn model_input(&self) -> &str {
        assert!(
            self.images.is_empty(),
            "image input requires ordered model_parts"
        );
        self.model_text.as_deref().unwrap_or(&self.text)
    }

    #[must_use]
    pub fn into_model_input(self) -> String {
        assert!(
            self.images.is_empty(),
            "image input requires ordered model_parts"
        );
        self.model_text.unwrap_or(self.text)
    }

    /// 표시 attachment marker를 제외한 정확한 순서의 model content입니다.
    #[must_use]
    pub fn model_parts(&self) -> Vec<ModelInputPart> {
        if self.images.is_empty() {
            return vec![ModelInputPart::Text {
                text: self.model_input().to_owned(),
            }];
        }
        let mut parts = Vec::with_capacity(self.images.len() * 2 + 1);
        let mut cursor = 0;
        for image in &self.images {
            if image.span().start > cursor {
                parts.push(ModelInputPart::Text {
                    text: self.text[cursor..image.span().start].to_owned(),
                });
            }
            parts.push(ModelInputPart::Image {
                snapshot: image.snapshot().clone(),
            });
            cursor = image.span().end;
        }
        if cursor < self.text.len() {
            parts.push(ModelInputPart::Text {
                text: self.text[cursor..].to_owned(),
            });
        }
        if let Some(model_text) = &self.model_text {
            let trailer = &model_text[self.text.len()..];
            if let Some(ModelInputPart::Text { text }) = parts.last_mut() {
                text.push_str(trailer);
            } else {
                parts.push(ModelInputPart::Text {
                    text: trailer.to_owned(),
                });
            }
        }
        parts
    }

    /// text-only input은 기존 string message로, image input은 multimodal item으로 replay합니다.
    #[must_use]
    pub fn model_replay_item(&self) -> crate::ModelReplayItem {
        if self.images.is_empty() {
            crate::ModelReplayItem::Message {
                role: crate::ModelReplayRole::User,
                content: self.model_input().to_owned(),
                refusal: None,
            }
        } else {
            crate::ModelReplayItem::MultimodalUser {
                parts: self.model_parts(),
            }
        }
    }

    pub(super) fn validate_image_encoding(&self) -> Result<(), UserInputError> {
        if !self.images.is_empty() {
            codec::validate_image_input_encoding(self)
                .map_err(|_| UserInputError::EncodedImageInputTooLarge)?;
        }
        Ok(())
    }
}
