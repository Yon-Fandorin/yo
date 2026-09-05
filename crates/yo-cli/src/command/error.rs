pub(super) fn raw_command_error(
    kind: clap::error::ErrorKind,
    message: impl Into<String>,
) -> clap::Error {
    let mut message = message.into();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    clap::Error::raw(kind, message)
}
