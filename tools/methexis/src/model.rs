use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const KNOWLEDGE_SCHEMA: &str = "methexis.knowledge/v1alpha1";
pub(crate) const OWNER_SCHEMA: &str = "methexis.owner/v1alpha1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
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

    pub(crate) fn required_targets(&self) -> impl Iterator<Item = &String> {
        self.depends_on.iter().chain(&self.constrained_by)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeMetadata {
    pub(crate) schema: String,
    pub(crate) id: String,
    pub(crate) kind: KnowledgeKind,
    pub(crate) owner: String,
    pub(crate) sources: Vec<String>,
    #[serde(default)]
    pub(crate) relations: Relations,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerRecord {
    pub(crate) schema: String,
    pub(crate) id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct KnowledgeUnit {
    pub(crate) metadata: KnowledgeMetadata,
    pub(crate) body: String,
    pub(crate) path: PathBuf,
    pub(crate) revision: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Owner {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
}

pub(crate) type UnitsById = BTreeMap<String, Vec<KnowledgeUnit>>;
