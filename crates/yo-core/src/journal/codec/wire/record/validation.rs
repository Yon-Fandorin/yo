use std::fmt::{Formatter, Result as FmtResult};

use serde::{
    Deserialize,
    de::{SeqAccess, Visitor},
};

use super::{super::JournalCodecError, WireContextLoss};
use crate::{
    JournalSequence,
    journal::codec::{self, SequencedJournalRecord},
};

pub(in super::super) fn required_journal_sequence(
    entry: &SequencedJournalRecord,
) -> Result<JournalSequence, JournalCodecError> {
    entry.journal_sequence().ok_or_else(|| {
        JournalCodecError::new("semantic Journal record is missing journal_sequence")
    })
}

pub(in super::super) fn non_null_accounting_field<
    'de,
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
>(
    decoder: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(decoder).map(Some)
}

pub(in super::super) fn deserialize_context_losses<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Vec<WireContextLoss>, D::Error> {
    struct LossVisitor;
    impl<'de> Visitor<'de> for LossVisitor {
        type Value = Vec<WireContextLoss>;
        fn expecting(&self, f: &mut Formatter<'_>) -> FmtResult {
            f.write_str("bounded checkpoint losses with at most 64 image entries")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;
            let mut losses = Vec::new();
            let mut images = Vec::new();
            while let Some(loss) = sequence.next_element::<WireContextLoss>()? {
                if losses.len() == 4096 {
                    return Err(A::Error::custom("checkpoint loss count exceeds its bound"));
                }
                if let WireContextLoss::ImageInputSummarized(image) = &loss {
                    if images.len() == 64 {
                        return Err(A::Error::custom("checkpoint image loss count exceeds 64"));
                    }
                    images.push(image.clone());
                    codec::validate_image_losses(&images).map_err(A::Error::custom)?;
                }
                losses.push(loss);
            }
            Ok(losses)
        }
    }
    decoder.deserialize_seq(LossVisitor)
}

pub(in super::super) fn with_context_epoch<T>(
    value: T,
    context_epoch: Option<u64>,
    apply: impl FnOnce(T, u64) -> T,
) -> Result<T, JournalCodecError> {
    match context_epoch {
        Some(0) => Err(JournalCodecError::new("context_epoch must be positive")),
        Some(context_epoch) => Ok(apply(value, context_epoch)),
        None => Ok(value),
    }
}
