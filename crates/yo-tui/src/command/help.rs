use yo_core::ActivityDocument;

use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Help,
    "command.help",
    "/help",
    "show commands and keyboard shortcuts",
    CommandEffect::ShowHelp,
);

pub(super) fn document(commands: String) -> ActivityDocument {
    ActivityDocument {
        title: "Available commands and keyboard help".into(),
        markdown: format!(
            "## Commands\n\n{commands}\n\n## Read and inspect\n\n- `Up/Down`, `PageUp/PageDown`: scroll output. `End`: follow latest.\n- `Alt+Up/Down`: move to item starts.\n- `Alt+O`: fold the current activity. `Ctrl+O`: reset item choices and toggle all.\n- `/changes`: inspect file diffs. `/output`: read retained tool output.\n- `Left/Right`: select files in Changes or tools in Output. `F1`: return to Chat.\n\n## Write and respond\n\n- `Ctrl+A/E`: start/end of the current input line.\n- `Ctrl+U/K`: remove to the start/end; `Ctrl+Y`: restore the last removal.\n- `Ctrl+Left/Right`: move by whitespace-separated words; `Ctrl+W`: remove the previous word.\n- For approvals, select a choice, then `Enter`. `Esc` declines or interrupts.\n- When offered, `Tab` adds answer notes; `Shift+Tab` returns to the previous question.\n\nCommands depend on the active connection's capabilities. Use `Alt+Up` to reach this guide's start."
        ),
    }
}
