use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Secrets,
    "command.secrets",
    "/secrets",
    "list locally saved secrets or delete one by its public ID",
    CommandEffect::ShowSecrets,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/secrets").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}

#[cfg(test)]
mod tests {
    use super::argument;

    // 명령 접두어와 삭제 ID 사이의 경계를 유지해 비슷한 일반 텍스트를 가로채지 않는다.
    #[test]
    fn secrets_argument_accepts_only_the_command_boundary() {
        assert_eq!(argument("/secrets"), Some(""));
        assert_eq!(argument("/secrets delete abc"), Some("delete abc"));
        assert_eq!(argument("/secretsdelete abc"), None);
        assert_eq!(argument(" /secrets"), None);
    }
}
