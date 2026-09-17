use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) slice: String,
    pub(super) candidate_commit: String,
    pub(super) owners: Owners,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owners {
    pub(super) packet: PacketOwner,
    pub(super) provider: ProviderOwner,
    pub(super) coordinator_context: ContextOwner,
    pub(super) command_output: CommandOwner,
    pub(super) elapsed: ElapsedOwner,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PacketOwner {
    pub(super) basis: String,
    pub(super) sources: Vec<Source>,
    pub(super) publication_count: u64,
    pub(super) rendered_bytes: Measurement,
    pub(super) managed_tokens: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderOwner {
    pub(super) basis: String,
    pub(super) sources: Vec<Source>,
    pub(super) request_count: u64,
    pub(super) usage: Usage,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextOwner {
    pub(super) basis: String,
    pub(super) sources: Vec<Source>,
    pub(super) usage: Usage,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Usage {
    pub(super) input_tokens: Measurement,
    pub(super) output_tokens: Measurement,
    pub(super) total_tokens: Measurement,
    pub(super) reasoning_tokens: Measurement,
    pub(super) cache_read_input_tokens: Measurement,
    pub(super) cache_write_input_tokens: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandOwner {
    pub(super) basis: String,
    pub(super) sources: Vec<Source>,
    pub(super) command_count: u64,
    pub(super) complete_log_bytes: Measurement,
    pub(super) returned_bytes: Measurement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ElapsedOwner {
    pub(super) basis: String,
    pub(super) sources: Vec<Source>,
    pub(super) total_milliseconds: Measurement,
    pub(super) critical_bottleneck: Bottleneck,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bottleneck {
    pub(super) name: String,
    pub(super) elapsed_milliseconds: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) schema: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Measurement {
    Reported { value: u64 },
    Partial { value: u64, reason: String },
    Unavailable { reason: String },
}

#[derive(Serialize)]
pub(super) struct Report<'a> {
    pub(super) schema: &'static str,
    pub(super) aggregation_policy: &'static str,
    pub(super) slice: &'a str,
    pub(super) candidate_commit: &'a str,
    pub(super) request: CapturedInput<'a>,
    pub(super) source_artifacts: Vec<CapturedSource>,
    pub(super) owners: &'a Owners,
}

#[derive(Serialize)]
pub(super) struct CapturedInput<'a> {
    pub(super) path: &'a Path,
    pub(super) hash: String,
    pub(super) bytes: usize,
}

#[derive(Serialize)]
pub(super) struct CapturedSource {
    pub(super) owner: &'static str,
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) schema: String,
    pub(super) bytes: usize,
}
