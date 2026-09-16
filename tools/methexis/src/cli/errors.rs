use serde::Serialize;

pub(super) const UNSUPPORTED_COMMAND: &str = "\
{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,\"error\":{\"code\":\"unsupported_command\",\"affected_ids\":[],\"next_actions\":[\"methexis --help\"]}}
";

#[derive(Serialize)]
pub(super) struct ArgumentFailure {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) error: ArgumentError,
}

#[derive(Serialize)]
pub(super) struct ArgumentError {
    pub(super) code: &'static str,
    pub(super) affected_ids: Vec<String>,
    pub(super) next_actions: Vec<&'static str>,
}

pub(super) fn argument_failure(code: &'static str, affected_ids: Vec<String>) -> ArgumentFailure {
    ArgumentFailure {
        schema: "methexis.error/v1alpha1",
        ok: false,
        error: ArgumentError {
            code,
            affected_ids,
            next_actions: vec!["methexis --help"],
        },
    }
}
