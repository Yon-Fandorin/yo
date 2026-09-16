use crate::review_protocol;

const ARGV_DOMAIN: &[u8] = b"yo.validation-run-argv/v1alpha1\0";

pub(super) fn verify_recorded_command_and_log(
    command_argv_count: usize,
    command_argv_hash: &str,
    log_hash: &str,
) -> Result<(), String> {
    if command_argv_count == 0 {
        return Err("command_argv_count must be greater than zero".to_owned());
    }
    canonical_sha256(command_argv_hash, "validation command argv hash")?;
    canonical_sha256(log_hash, "validation log hash")
}

pub(super) fn verify_command_and_log(
    expected_argv: &[String],
    command_argv_count: usize,
    command_argv_hash: &str,
    log_hash: &str,
) -> Result<(), String> {
    if command_argv_count != expected_argv.len() {
        return Err("command_argv_count does not match the gate request".to_owned());
    }
    canonical_sha256(command_argv_hash, "validation command argv hash")?;
    let expected_hash = argv_hash(expected_argv);
    if command_argv_hash != expected_hash {
        return Err(format!(
            "command_argv_hash does not match the gate request; expected {expected_hash}"
        ));
    }
    canonical_sha256(log_hash, "validation log hash")
}

pub(super) fn canonical_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}

pub(super) fn argv_hash(argv: &[String]) -> String {
    let mut framed = Vec::with_capacity(
        ARGV_DOMAIN.len() + argv.iter().map(|value| value.len() + 24).sum::<usize>(),
    );
    framed.extend_from_slice(ARGV_DOMAIN);
    for value in argv {
        framed.extend_from_slice(value.len().to_string().as_bytes());
        framed.push(b':');
        framed.extend_from_slice(value.as_bytes());
        framed.push(0);
    }
    review_protocol::digest(&framed)
}
