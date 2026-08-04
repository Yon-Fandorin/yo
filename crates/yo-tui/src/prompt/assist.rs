//! One prompt-assist controller for every cursor-local trigger kind.

use yo_core::{
    SkillReferenceSearchRequest, SkillReferenceSearchUpdate, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchUpdate,
};

use super::{
    skill_reference::SkillReferenceAssist,
    workspace_reference::{
        AssistTriggerKind, WorkspaceEdit, WorkspaceReferenceAssist, scan_prompt_trigger,
    },
};
use crate::{
    input::editor::PromptEditor,
    overlay::{AcceptanceReceipt, PromptOverlaySlot},
};

#[derive(Debug)]
pub(crate) enum PromptAssistRequest {
    Workspace(WorkspaceReferenceSearchRequest),
    Skill(SkillReferenceSearchRequest),
}

#[derive(Debug, Default)]
pub(crate) struct PromptAssistController {
    workspace_enabled: bool,
    skill_enabled: bool,
    editor_revision: u64,
    next_request_id: u64,
    workspace: WorkspaceReferenceAssist,
    skill: SkillReferenceAssist,
}

impl PromptAssistController {
    pub(crate) fn enable_workspace(&mut self) {
        self.workspace_enabled = true;
    }

    pub(crate) fn enable_skill(&mut self) {
        self.skill_enabled = true;
    }

    pub(crate) fn prompt_changed(
        &mut self,
        editor: &PromptEditor,
        overlay: &mut PromptOverlaySlot,
        edit: Option<&WorkspaceEdit>,
        eligible: bool,
    ) -> Option<PromptAssistRequest> {
        self.workspace.update_annotations(editor.text(), edit);
        self.skill.update_annotation(editor.text(), edit);
        self.editor_revision = self.editor_revision.saturating_add(1);
        if !eligible {
            self.close(overlay);
            return None;
        }
        let Some(trigger) = scan_prompt_trigger(editor.text(), editor.cursor_byte_index()) else {
            self.close(overlay);
            return None;
        };
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request_id = self.next_request_id;
        match trigger.kind {
            AssistTriggerKind::Workspace if self.workspace_enabled => {
                self.skill.close(overlay);
                self.workspace
                    .begin(editor, overlay, trigger, request_id, self.editor_revision)
                    .map(|(request, _)| PromptAssistRequest::Workspace(request))
            },
            AssistTriggerKind::Skill if self.skill_enabled => {
                self.workspace.close(overlay);
                if self.skill.trigger_is_accepted(&trigger) {
                    self.skill.close(overlay);
                    return None;
                }
                self.skill
                    .begin(editor, overlay, trigger, request_id, self.editor_revision)
                    .map(|(request, _)| PromptAssistRequest::Skill(request))
            },
            AssistTriggerKind::Workspace | AssistTriggerKind::Skill => {
                self.close(overlay);
                None
            },
        }
    }

    pub(crate) fn observe_workspace(
        &mut self,
        update: WorkspaceReferenceSearchUpdate,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        update.editor_revision() == self.editor_revision && self.workspace.observe(update, overlay)
    }

    pub(crate) fn observe_skill(
        &mut self,
        update: SkillReferenceSearchUpdate,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        update.editor_revision() == self.editor_revision && self.skill.observe(update, overlay)
    }

    pub(crate) fn workspace_failed(
        &mut self,
        reason: String,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        self.workspace.provider_failed(reason, overlay)
    }

    pub(crate) fn skill_failed(&mut self, reason: String, overlay: &mut PromptOverlaySlot) -> bool {
        self.skill.provider_failed(reason, overlay)
    }

    pub(crate) fn filter_changed(
        &mut self,
        selected: usize,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        self.skill.filter_changed(selected, overlay)
    }

    pub(crate) fn accept(
        &mut self,
        receipt: &AcceptanceReceipt,
        editor: &mut PromptEditor,
    ) -> bool {
        self.workspace.accept(receipt, editor) || self.skill.accept(receipt, editor)
    }

    pub(crate) fn has_accepted_references(&self) -> bool {
        self.workspace.has_accepted_references() || self.skill.has_accepted_reference()
    }

    pub(crate) fn cancel(&mut self) {
        self.workspace.cancel();
        self.skill.cancel();
    }

    fn close(&mut self, overlay: &mut PromptOverlaySlot) {
        self.workspace.close(overlay);
        self.skill.close(overlay);
    }
}

#[cfg(test)]
mod tests;
