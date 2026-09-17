use super::model::{Measurement, Owners};

pub(super) fn validate_owners(owners: &Owners) -> Result<(), String> {
    let values = [
        ("packet rendered bytes", &owners.packet.rendered_bytes),
        ("packet managed tokens", &owners.packet.managed_tokens),
        ("provider input", &owners.provider.usage.input_tokens),
        ("provider output", &owners.provider.usage.output_tokens),
        ("provider total", &owners.provider.usage.total_tokens),
        (
            "provider reasoning",
            &owners.provider.usage.reasoning_tokens,
        ),
        (
            "provider cache read",
            &owners.provider.usage.cache_read_input_tokens,
        ),
        (
            "provider cache write",
            &owners.provider.usage.cache_write_input_tokens,
        ),
        (
            "coordinator input",
            &owners.coordinator_context.usage.input_tokens,
        ),
        (
            "coordinator output",
            &owners.coordinator_context.usage.output_tokens,
        ),
        (
            "coordinator total",
            &owners.coordinator_context.usage.total_tokens,
        ),
        (
            "coordinator reasoning",
            &owners.coordinator_context.usage.reasoning_tokens,
        ),
        (
            "coordinator cache read",
            &owners.coordinator_context.usage.cache_read_input_tokens,
        ),
        (
            "coordinator cache write",
            &owners.coordinator_context.usage.cache_write_input_tokens,
        ),
        (
            "command complete log",
            &owners.command_output.complete_log_bytes,
        ),
        ("command returned", &owners.command_output.returned_bytes),
        ("elapsed total", &owners.elapsed.total_milliseconds),
    ];
    for (label, value) in values {
        value.validate(label)?;
    }

    if let (Some(complete), Some(returned)) = (
        owners.command_output.complete_log_bytes.value(),
        owners.command_output.returned_bytes.value(),
    ) && returned > complete
    {
        return Err("returned command bytes cannot exceed complete log bytes".to_owned());
    }
    require_text(
        &owners.elapsed.critical_bottleneck.name,
        "elapsed bottleneck name",
    )?;
    if owners.elapsed.critical_bottleneck.elapsed_milliseconds == 0 {
        return Err("elapsed bottleneck must be nonzero".to_owned());
    }
    if let Some(total) = owners.elapsed.total_milliseconds.value()
        && total < owners.elapsed.critical_bottleneck.elapsed_milliseconds
    {
        return Err("elapsed total cannot be smaller than its bottleneck".to_owned());
    }
    Ok(())
}

impl Measurement {
    const fn value(&self) -> Option<u64> {
        match self {
            Self::Reported { value } | Self::Partial { value, .. } => Some(*value),
            Self::Unavailable { .. } => None,
        }
    }

    fn validate(&self, label: &str) -> Result<(), String> {
        match self {
            Self::Reported { .. } => Ok(()),
            Self::Partial { reason, .. } | Self::Unavailable { reason } => {
                require_text(reason, &format!("{label} reason"))
            },
        }
    }
}

pub(super) fn require_text(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!("{label} must be nonblank and bounded"))
    } else {
        Ok(())
    }
}
