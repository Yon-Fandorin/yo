use std::num::NonZeroU16;

use yo_core::ApiCredential;

use crate::{
    AppError,
    connection::{
        input::TtyConnectionInput,
        presentation::{
            PlanAction, PresentationError, default_width, push_change, push_title, wrap,
        },
    },
    presentation::{PresentationStyle, TextStyle},
};

/// Structured presentation for the account command's no-echo secret capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HiddenSecretAction {
    Save,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HiddenSecretPrompt {
    title: String,
    target: String,
    action: HiddenSecretAction,
    subject: String,
    detail: String,
    instruction: String,
    note: String,
    label: String,
}

impl HiddenSecretPrompt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        title: impl Into<String>,
        target: impl Into<String>,
        action: HiddenSecretAction,
        subject: impl Into<String>,
        detail: impl Into<String>,
        instruction: impl Into<String>,
        note: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            target: target.into(),
            action,
            subject: subject.into(),
            detail: detail.into(),
            instruction: instruction.into(),
            note: note.into(),
            label: label.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn render(&self, width: NonZeroU16) -> String {
        self.render_styled(width, PresentationStyle::Plain)
            .expect("fixture prompt must satisfy the presentation grammar")
    }

    pub(crate) fn render_styled(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        let width = usize::from(width.get());
        let mut output = String::new();
        push_title(&mut output, &self.title, &self.target, width, style)?;
        output.push('\n');
        let action = match self.action {
            HiddenSecretAction::Save => PlanAction::Add,
            HiddenSecretAction::Replace => PlanAction::Change,
        };
        push_change(
            &mut output,
            action,
            &self.subject,
            &self.detail,
            width,
            style,
        )?;
        output.push('\n');
        for line in wrap(&self.instruction, width)? {
            output.push_str(&line);
            output.push('\n');
        }
        for line in wrap(&self.note, width)? {
            style.push(&mut output, TextStyle::Muted, &line);
            output.push('\n');
        }
        output.push('\n');
        style.push(&mut output, TextStyle::Bold, &self.label);
        Ok(output)
    }
}

pub(crate) fn read_hidden_secret(
    prompt: &HiddenSecretPrompt,
    validation_context: &'static str,
) -> Result<ApiCredential, AppError> {
    let mut input = TtyConnectionInput::new();
    let terminal_width = {
        let terminal = input.terminal()?;
        rustix::termios::tcgetwinsize(&*terminal)
            .ok()
            .and_then(|size| NonZeroU16::new(size.ws_col))
            .unwrap_or_else(default_width)
    };
    let rendered = prompt
        .render_styled(terminal_width, input.style())
        .map_err(|error| AppError::single("formatting the hidden-secret prompt", error))?;
    input.read_secret_with(&rendered, validation_context, TtyConnectionInput::read_line)
}

#[cfg(test)]
mod tests {
    use yo_tui::surface::cell_width;

    use super::*;

    fn width(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).unwrap()
    }

    // account refresh의 hidden secret prompt는 account 전용 의미를 유지하면서 좁은 터미널에서도
    // 모든 줄을 실제 cell width 안에 렌더링합니다.
    #[test]
    fn hidden_secret_prompt_uses_the_shared_cli_hierarchy_at_every_width() {
        let prompt = HiddenSecretPrompt::new(
            "ACCOUNT ACCESS",
            "qwencloud:default",
            HiddenSecretAction::Save,
            "Browser session",
            "Not saved · save for future account refreshes",
            "Paste the Cookie header from Billing > Subscription.",
            "It stays separate from the model API key.",
            "Cookie (hidden): ",
        );

        let output = prompt.render(width(80));
        assert!(output.starts_with("ACCOUNT ACCESS  qwencloud:default\n\n+ Browser session\n"));
        assert!(output.contains("Not saved · save for future account refreshes"));
        assert!(output.contains("It stays separate from the model API key."));
        assert!(output.ends_with("Cookie (hidden): "));

        let narrow = prompt.render(width(24));
        assert!(
            narrow
                .lines()
                .all(|line| cell_width(line).is_ok_and(|width| width <= 24)),
            "narrow prompt: {narrow:?}"
        );
    }
}
