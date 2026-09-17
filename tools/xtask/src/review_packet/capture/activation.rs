use super::ActivationRequest;

pub(in crate::review_packet) fn parse_activation_request(
    bytes: &[u8],
) -> Result<ActivationRequest, String> {
    let request = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid activation request: {error}"))?;
    validate_activation_request(&request)?;
    Ok(request)
}

fn validate_activation_request(request: &ActivationRequest) -> Result<(), String> {
    if request.schema == "methexis.activation-request/v1alpha1"
        && valid_hash(&request.checkpoint_id)
        && valid_hash(&request.checkpoint_hash)
        && request
            .replace_active_hash
            .as_ref()
            .is_none_or(|hash| valid_hash(hash))
    {
        Ok(())
    } else {
        Err("activation request schema or hashes are invalid".to_owned())
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
