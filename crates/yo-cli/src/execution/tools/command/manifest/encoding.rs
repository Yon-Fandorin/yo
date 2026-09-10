//! The manifest-specific framed digest; never used as a general JSON wire format.

use serde_json::Value;
use sha2::{Digest, Sha256};
use yo_core::ToolExecutionError;

use super::{MANIFEST_PROFILE, invalid};

const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn digest(value: &Value) -> Result<String, ToolExecutionError> {
    let mut encoder = Encoder {
        hash: Sha256::new(),
        remaining: MAX_ENCODED_BYTES,
    };
    encoder.bytes(MANIFEST_PROFILE.as_bytes())?;
    encoder.bytes(&[0])?;
    encoder.value(value)?;
    Ok(tagged_hash(encoder.hash))
}

pub(super) fn tagged_hash(hash: Sha256) -> String {
    format!(
        "sha256:{}",
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

struct Encoder {
    hash: Sha256,
    remaining: usize,
}

impl Encoder {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ToolExecutionError> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| invalid("command manifest exceeds its encoded bound"))?;
        self.hash.update(bytes);
        Ok(())
    }

    fn length(&mut self, length: usize) -> Result<(), ToolExecutionError> {
        self.bytes(&(length as u64).to_be_bytes())
    }

    fn string(&mut self, value: &str) -> Result<(), ToolExecutionError> {
        self.bytes(&[5])?;
        self.length(value.len())?;
        self.bytes(value.as_bytes())
    }

    fn value(&mut self, value: &Value) -> Result<(), ToolExecutionError> {
        match value {
            Value::Null => self.bytes(&[0]),
            Value::Bool(false) => self.bytes(&[1]),
            Value::Bool(true) => self.bytes(&[2]),
            Value::Number(number) if number.is_f64() => {
                let number = number
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| invalid("command manifest contains an invalid number"))?;
                self.bytes(&[4])?;
                self.bytes(
                    &(if number == 0.0 { 0.0 } else { number })
                        .to_bits()
                        .to_be_bytes(),
                )
            },
            Value::Number(number) => {
                let text = number.to_string();
                self.bytes(&[3])?;
                self.length(text.len())?;
                self.bytes(text.as_bytes())
            },
            Value::String(value) => self.string(value),
            Value::Array(values) => {
                self.bytes(&[6])?;
                self.length(values.len())?;
                for value in values {
                    self.value(value)?;
                }
                Ok(())
            },
            Value::Object(values) => {
                self.bytes(&[7])?;
                self.length(values.len())?;
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
                for (key, value) in entries {
                    self.string(key)?;
                    self.value(value)?;
                }
                Ok(())
            },
        }
    }
}
