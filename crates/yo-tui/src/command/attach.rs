use std::{ops::Range, path::PathBuf};

use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Attach,
    "command.attach",
    "/attach",
    "attach a local PNG or JPEG: /attach PATH",
    CommandEffect::AttachImage,
);

pub(crate) fn attachment_argument(text: &str) -> Option<(Range<usize>, PathBuf)> {
    // Bracketed paste may preserve CR or CRLF line breaks (for example tmux's
    // paste-buffer conversion). Recognize the final command without rewriting
    // the surrounding draft or its immutable image/reference byte spans. Keep
    // trailing terminators outside the command's replacement range.
    let command_text = text.trim_end_matches(['\n', '\r']);
    let start = command_text
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    let line = &command_text[start..];
    let argument = line.strip_prefix("/attach")?;
    if !argument.is_empty() && !argument.starts_with(char::is_whitespace) {
        return None;
    }
    let path = argument.trim();
    let path = path
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .or_else(|| {
            path.strip_prefix('\'')
                .and_then(|path| path.strip_suffix('\''))
        })
        .unwrap_or(path);
    Some((start..command_text.len(), PathBuf::from(path)))
}
