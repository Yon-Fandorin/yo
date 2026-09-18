use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretAnswerState {
    Submitted,
    ReentryRequired,
}

impl SecretAnswerState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::ReentryRequired => "reentry_required",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "submitted" => Some(Self::Submitted),
            "reentry_required" => Some(Self::ReentryRequired),
            _ => None,
        }
    }
}

/// Public answer data or a closed, payload-free secret marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Answer {
    pub question_id: String,
    pub option_id: Option<String>,
    pub text: String,
    pub notes: String,
    secret: Option<SecretAnswerState>,
}

impl Answer {
    pub(crate) fn public(
        question_id: String,
        option_id: Option<String>,
        text: String,
        notes: String,
    ) -> Self {
        Self {
            question_id,
            option_id,
            text,
            notes,
            secret: None,
        }
    }

    pub(crate) fn submitted_secret(question_id: String) -> Self {
        Self {
            question_id,
            option_id: None,
            text: String::new(),
            notes: String::new(),
            secret: Some(SecretAnswerState::Submitted),
        }
    }

    pub(crate) fn reentry_required_secret(question_id: String) -> Self {
        Self {
            question_id,
            option_id: None,
            text: String::new(),
            notes: String::new(),
            secret: Some(SecretAnswerState::ReentryRequired),
        }
    }

    pub(crate) fn secret_state(&self) -> Option<SecretAnswerState> {
        self.secret
    }

    pub fn is_secret(&self) -> bool {
        self.secret.is_some()
    }
}

#[derive(Serialize)]
struct PublicAnswerRef<'a> {
    question_id: &'a str,
    option_id: &'a Option<String>,
    text: &'a str,
    notes: &'a str,
}

#[derive(Serialize)]
struct SecretAnswerRef<'a> {
    question_id: &'a str,
    secret: &'static str,
}

impl Serialize for Answer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.secret {
            Some(state) => SecretAnswerRef {
                question_id: &self.question_id,
                secret: state.as_str(),
            }
            .serialize(serializer),
            None => PublicAnswerRef {
                question_id: &self.question_id,
                option_id: &self.option_id,
                text: &self.text,
                notes: &self.notes,
            }
            .serialize(serializer),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicAnswerOwned {
    question_id: String,
    option_id: Option<String>,
    text: String,
    notes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretAnswerOwned {
    question_id: String,
    secret: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AnswerWire {
    Public(PublicAnswerOwned),
    Secret(SecretAnswerOwned),
}

impl<'de> Deserialize<'de> for Answer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match AnswerWire::deserialize(deserializer)? {
            AnswerWire::Public(answer) => Ok(Self::public(
                answer.question_id,
                answer.option_id,
                answer.text,
                answer.notes,
            )),
            AnswerWire::Secret(answer) => {
                let state = SecretAnswerState::from_str(&answer.secret).ok_or_else(|| {
                    D::Error::custom("unsupported secret interview answer marker")
                })?;
                Ok(Self {
                    question_id: answer.question_id,
                    option_id: None,
                    text: String::new(),
                    notes: String::new(),
                    secret: Some(state),
                })
            },
        }
    }
}
