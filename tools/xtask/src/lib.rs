mod activation_slice;
mod bounded_file;
mod cli;
mod cost_report;
mod docs_translation;
mod git;
mod grok_outer_sandbox;
mod impact;
mod review_continuation_preflight;
mod review_delivery;
mod review_delta;
mod review_egress;
mod review_packet;
mod review_prepare;
mod review_protocol;
mod review_result;
mod review_session;
mod review_target_admission;
mod slice_accept;
mod slice_close;
mod slice_contract;
mod slice_create;
mod slice_gate;
mod slice_status;
mod slice_worktree;
mod test_explanations;
mod validation_stage;
mod validation_summary;

#[cfg(test)]
mod test_support;

pub use cli::run;
