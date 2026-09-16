//! Wire 레코드 문법과 책임별 의미 변환.

mod conversion;
mod fork;
mod replay;
mod validation;

pub(super) use conversion::WireRecord;
pub(super) use fork::{WireForkRecord, prepare_seed, validate_child};
pub(super) use replay::{
    WireContextArtifactReceipt, WireContextLoss, WireContextRetainedGroup, WireModelReplayContract,
    WireModelReplayDelta, WireModelReplayItem, decode_context_loss, decode_model_replay,
    decode_model_replay_contract, decode_model_replay_items, encode_context_loss,
    encode_model_replay, encode_model_replay_contract, encode_model_replay_item,
};
pub(super) use validation::{
    deserialize_context_losses, non_null_accounting_field, required_journal_sequence,
    with_context_epoch,
};
