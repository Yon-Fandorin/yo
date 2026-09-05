use crate::{
    AppError,
    connection::{input::TtyConnectionInput, presentation::ConfirmationView},
};

pub(crate) trait ExternalDisconnectInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError>;
    fn confirm(&mut self, preview: &dyn ConfirmationView) -> Result<bool, AppError>;
}

impl ExternalDisconnectInput for TtyConnectionInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError> {
        let terminal = self.terminal()?;
        use std::io::Write;
        write!(
            terminal,
            "Select one stored target by entering its exact reference:\n  - {}\nTarget: ",
            choices.join("\n  - ")
        )
        .and_then(|()| terminal.flush())
        .map_err(|error| AppError::single("writing the disconnect target prompt", error))?;
        TtyConnectionInput::read_line(terminal)
    }

    fn confirm(&mut self, preview: &dyn ConfirmationView) -> Result<bool, AppError> {
        TtyConnectionInput::confirm(self, preview)
    }
}
