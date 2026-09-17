use std::fmt::{Display, Formatter, Result as FmtResult};

/// 하나의 complete public connection snapshot을 위한 불투명 compare-and-swap token입니다.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ConnectionRevision {
    Absent,
    Token(String),
}

impl ConnectionRevision {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub(crate) fn from_operation_journal(value: &str) -> Option<Self> {
        if value == "absent" {
            return Some(Self::Absent);
        }
        parse_revision_token(value).map(Self::Token)
    }
}

impl Display for ConnectionRevision {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Token(token) => formatter.write_str(token),
        }
    }
}

fn parse_revision_token(revision: &str) -> Option<String> {
    let valid = revision.len() == 36
        && revision.starts_with("rev-")
        && revision[4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    valid.then(|| revision.to_owned())
}
