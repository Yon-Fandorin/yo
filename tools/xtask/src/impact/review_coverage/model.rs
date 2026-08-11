use super::super::slice_review::Lens;

pub(super) struct Coverage {
    pub(super) lens: Lens,
    pub(super) reviewer: Reviewer,
    pub(super) diff_hash: String,
}

impl Coverage {
    pub(super) fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(" - ");
        let lens = parts.next().and_then(Lens::parse)?;
        if parts.next()? != "exact" {
            return None;
        }
        let reviewer = Reviewer::parse(parts.next()?)?;
        let diff_hash = parts.next()?;
        if !valid_sha256(diff_hash) || parts.next().is_some() {
            return None;
        }
        Some(Self {
            lens,
            reviewer,
            diff_hash: diff_hash.to_owned(),
        })
    }
}

pub(super) enum Reviewer {
    Human {
        identity: String,
    },
    Model {
        high: bool,
        provider: String,
        session: String,
    },
}

impl Reviewer {
    fn parse(value: &str) -> Option<Self> {
        let segments = value.split('/').collect::<Vec<_>>();
        match segments.as_slice() {
            ["human", identity] if valid_segment(identity) => Some(Self::Human {
                identity: (*identity).to_owned(),
            }),
            [class @ ("model" | "model-high"), provider, model, session]
                if [provider, model, session]
                    .into_iter()
                    .all(|segment| valid_segment(segment)) =>
            {
                Some(Self::Model {
                    high: *class == "model-high",
                    provider: (*provider).to_owned(),
                    session: (*session).to_owned(),
                })
            },
            _ => None,
        }
    }

    pub(super) fn compact_id(&self) -> String {
        match self {
            Self::Human { identity } => format!("human/{identity}"),
            Self::Model {
                provider, session, ..
            } => format!("{provider}/{session}"),
        }
    }

    pub(super) fn is_high_or_human(&self) -> bool {
        matches!(self, Self::Human { .. } | Self::Model { high: true, .. })
    }
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:+-".contains(character))
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}
