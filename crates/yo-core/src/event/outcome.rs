#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    code: Option<String>,
    message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Result<Self, &'static str> {
        let code = code.into();
        if code.is_empty()
            || code.len() > 128
            || !code.is_ascii()
            || code
                .chars()
                .any(|character| character.is_ascii_whitespace() || character.is_ascii_control())
        {
            return Err("failure code must be a non-empty bounded ASCII identifier");
        }
        self.code = Some(code);
        Ok(self)
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
