use yo_core::{ModelSelection, ModelSelectionController};

use crate::overlay::{PanelSnapshot, SelectionEntry};

#[derive(Clone, Debug)]
pub(super) struct ModelSelectionState {
    controller: ModelSelectionController,
}

impl ModelSelectionState {
    pub(super) const fn new(controller: ModelSelectionController) -> Self {
        Self { controller }
    }

    pub(super) fn panel(&self) -> Result<PanelSnapshot, String> {
        let entries = self
            .controller
            .choices()
            .iter()
            .map(|choice| {
                let selection = choice.selection();
                let detail = Some(format!(
                    "{} / {} / {}",
                    selection.provider(),
                    selection.account(),
                    selection.model()
                ));
                if let Some(reason) = choice.disabled_reason() {
                    return SelectionEntry::disabled(
                        selection.row_identity(),
                        choice.model_label(),
                        detail,
                        reason,
                    );
                }
                let context = choice.last_failure().map_or_else(
                    || format!("{} › {}", choice.provider_label(), choice.account_label()),
                    |failure| {
                        format!(
                            "{} › {} · warning: {} at {}",
                            choice.provider_label(),
                            choice.account_label(),
                            failure.kind(),
                            failure.observed_at()
                        )
                    },
                );
                SelectionEntry::enabled_with_context(
                    selection.row_identity(),
                    choice.model_label(),
                    Some(context),
                    detail,
                )
            })
            .collect();
        PanelSnapshot::new("Select model", entries)
            .map_err(|error| format!("the configured model catalog cannot be displayed: {error:?}"))
    }

    pub(super) fn resolve_direct(&self, value: &str) -> Result<ModelSelection, String> {
        self.controller
            .resolve_reference(value)
            .map_err(|error| error.to_string())
    }

    pub(super) fn accept_identity(&self, identity: &str) -> Result<ModelSelection, String> {
        self.controller
            .accept_row_identity(identity)
            .map_err(|error| error.to_string())
    }

    pub(super) fn is_current(&self, selection: &ModelSelection) -> bool {
        self.controller.current() == Some(selection)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::SystemTime};

    use yo_core::{
        AccountId, CompleteModelBinding, ConnectionAccount, LocalConnectionRepository,
        ModelLastFailure, ModelRequestFailureKind, ModelSelectionController, ProviderId,
        StoredModelBinding,
    };

    use super::ModelSelectionState;
    use crate::{
        appearance::AppearanceState,
        overlay::{OverlayBindings, SelectionPanel},
        surface::{CellContent, Point, Rect, Size, Surface},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "yo-tui-model-observation-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn rendered_panel(state: &ModelSelectionState) -> String {
        let panel = SelectionPanel::new(state.panel().unwrap());
        let appearance = AppearanceState::default();
        let pin = appearance.pin();
        let prepared = panel
            .prepare(
                Size::new(180, 6),
                pin.snapshot().styles().overlay,
                &OverlayBindings::default(),
                false,
            )
            .unwrap();
        let prepared_size = prepared.size();
        let mut surface = Surface::new(prepared_size).unwrap();
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), prepared_size))
            .unwrap();
        prepared.paint(&mut view).unwrap();
        (0..prepared_size.height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..prepared_size.width {
                    match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank => row.push(' '),
                        CellContent::Continuation { .. } => {},
                        CellContent::Grapheme { text, .. } => row.push_str(text),
                    }
                }
                row
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // A remembered request failure is visible context, not an availability gate: the exact
    // Provider/Account/Model row remains selectable while carrying the durable warning.
    #[test]
    fn model_failure_is_a_visible_warning_without_disabling_selection() {
        let directory = TestDirectory::new();
        let repository = LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let complete = CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"qwen3.8max","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap();
        let account = ConnectionAccount::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            Some("Qwen Cloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding =
            StoredModelBinding::new(complete.clone(), Some("Qwen 3.8 Max".to_owned())).unwrap();
        let selection = binding.selection();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let failure =
            ModelLastFailure::new(ModelRequestFailureKind::RateLimited, "2026-08-17T09:10:11Z")
                .unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_observation(&selection, &complete, Some(failure))
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();

        let controller = ModelSelectionController::new(
            repository.capture().unwrap().model_catalog().unwrap(),
            None,
        );
        let state = ModelSelectionState::new(controller);
        let rendered = rendered_panel(&state);

        assert!(
            rendered
                .contains("Qwen Cloud › Default · warning: rate_limited at 2026-08-17T09:10:11Z"),
            "{rendered}"
        );
        assert_eq!(
            state.accept_identity(&selection.row_identity()).unwrap(),
            selection
        );
    }

    // operator가 비활성화한 stored model은 picker에서 사라지지 않고 한 가지 이유로
    // 표시되지만 row 수락은 같은 binding을 새 작업으로 시작하지 못하게 거절합니다.
    #[test]
    fn disabled_model_is_visible_with_reason_and_cannot_be_accepted() {
        let directory = TestDirectory::new();
        let repository = LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let complete = CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"qwen3.8max","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap();
        let account = ConnectionAccount::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            Some("Qwen Cloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding = StoredModelBinding::new(complete, Some("Qwen 3.8 Max".to_owned())).unwrap();
        let selection = binding.selection();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_activation(&selection, false)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();

        let controller = ModelSelectionController::new(
            repository.capture().unwrap().model_catalog().unwrap(),
            None,
        );
        let state = ModelSelectionState::new(controller);
        let rendered = rendered_panel(&state);

        assert!(rendered.contains("Qwen 3.8 Max"), "{rendered}");
        assert!(rendered.contains("disabled by operator"), "{rendered}");
        assert!(
            state
                .accept_identity(&selection.row_identity())
                .unwrap_err()
                .contains("disabled by operator")
        );
    }
}
