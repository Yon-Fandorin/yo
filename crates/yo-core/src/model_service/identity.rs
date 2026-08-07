use std::{fmt, str::FromStr};

const MAX_ID_BYTES: usize = 256;

macro_rules! model_service_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
                let value = value.into();
                validate_id($label, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = ModelServiceError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }
    };
}

model_service_id!(ProviderId, "ProviderId");
model_service_id!(AccountId, "AccountId");
model_service_id!(ModelId, "ModelId");

fn validate_id(label: &'static str, value: &str) -> Result<(), ModelServiceError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(ModelServiceError::new(format!(
            "{label} must contain 1 to {MAX_ID_BYTES} bytes"
        )));
    }
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(ModelServiceError::new(format!(
            "{label} must not contain surrounding whitespace or control characters"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelServiceError {
    message: String,
}

impl ModelServiceError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelServiceError {}
