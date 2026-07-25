//! Closed on-disk record shapes used by discovery.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub(crate) const KNOWLEDGE_SCHEMA: &str = "methexis.knowledge/v1alpha1";
pub(crate) const OWNER_SCHEMA: &str = "methexis.owner/v1alpha1";
pub(crate) const SOURCE_SCHEMA: &str = "methexis.source/v1alpha1";
pub(crate) const PROJECTION_SCHEMA: &str = "methexis.review-projection/v1alpha1";
pub(crate) const PROJECTION_PROFILE: &str = "ko-review/v1alpha1";
pub(crate) const PROJECTION_COMPILER: &str = "methexis/0.0.0";
pub(crate) const PROJECTION_REQUEST_SCHEMA: &str = "methexis.review-projection-request/v1alpha1";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRef {
    pub(crate) id: String,
    pub(crate) revision: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KnowledgeKind {
    Definition,
    Rule,
    Decision,
    Procedure,
}

impl KnowledgeKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Rule => "rule",
            Self::Decision => "decision",
            Self::Procedure => "procedure",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relations {
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) constrained_by: Vec<String>,
    #[serde(default)]
    pub(crate) validated_by: Vec<String>,
    #[serde(default)]
    pub(crate) applies_to: Vec<String>,
    #[serde(default)]
    pub(crate) supersedes: Vec<String>,
}

impl Relations {
    pub(crate) fn typed(&self) -> [(&'static str, &[String]); 5] {
        [
            ("applies_to", &self.applies_to),
            ("constrained_by", &self.constrained_by),
            ("depends_on", &self.depends_on),
            ("supersedes", &self.supersedes),
            ("validated_by", &self.validated_by),
        ]
    }

    pub(crate) fn knowledge_targets(&self) -> impl Iterator<Item = &String> {
        self.depends_on
            .iter()
            .chain(&self.constrained_by)
            .chain(&self.supersedes)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeMetadata {
    pub(crate) schema: String,
    pub(crate) id: String,
    pub(crate) kind: KnowledgeKind,
    pub(crate) owner: String,
    pub(crate) sources: Vec<SourceRef>,
    #[serde(default)]
    pub(crate) relations: Relations,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionMetadata {
    pub(crate) schema: String,
    pub(crate) knowledge_id: String,
    pub(crate) revision: String,
    pub(crate) profile: String,
    pub(crate) compiler: String,
    pub(crate) request_hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogUnit {
    pub(crate) id: String,
    pub(crate) revision: String,
    pub(crate) owner: String,
    pub(crate) sources: Vec<SourceRef>,
    pub(crate) path: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) projection: Option<String>,
    pub(crate) relations: Relations,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerRecord {
    pub(crate) schema: String,
    pub(crate) id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRecord {
    pub(crate) schema: String,
    pub(crate) id: String,
    pub(crate) revision: String,
    pub(crate) payload: SourcePayload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SourcePayload {
    Decision {
        content: String,
    },
    Code {
        path: String,
        symbol: String,
        content_hash: String,
        #[serde(default)]
        line_hint: Option<u64>,
    },
    Conversation {
        material: ConversationMaterial,
    },
    External {
        freshness: ExternalFreshness,
    },
}

impl SourcePayload {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Decision { .. } => "decision",
            Self::Code { .. } => "code",
            Self::Conversation { .. } => "conversation",
            Self::External { .. } => "external",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConversationMaterial {
    Excerpt {
        content: String,
    },
    Opaque {
        reference: String,
        content_hash: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "freshness", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ExternalFreshness {
    Immutable {
        locator: String,
        version: String,
        content_hash: String,
    },
    Mutable {
        locator: String,
        content_hash: String,
    },
    Attested {
        reference: String,
        content_hash: String,
        expires_at: String,
    },
}

impl CatalogUnit {
    pub(crate) fn searchable_fields(&self) -> BTreeMap<&'static str, &str> {
        let mut fields = BTreeMap::from([
            ("body", self.body.as_str()),
            ("id", self.id.as_str()),
            ("path", self.path.as_str()),
            ("title", self.title.as_str()),
        ]);
        if let Some(projection) = &self.projection {
            fields.insert("projection", projection);
        }
        fields
    }
}
