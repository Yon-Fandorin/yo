use serde::{Deserialize, Serialize};

use super::super::JournalCodecError;
use crate::{
    InputReference, SkillReference, SkillReferenceScope, UserInput, WorkspaceReference,
    WorkspaceReferenceKind,
};

const PROFILE: &str = "yo.structured-input/v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireUserInput {
    profile: String,
    text: String,
    references: Vec<WireInputReference>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireInputReference {
    Workspace {
        start: u64,
        end: u64,
        projection: String,
        identity: String,
        execution_environment_identity: String,
        workspace_identity: String,
        root_identity: String,
        relative_path: String,
        kind: WireWorkspaceReferenceKind,
    },
    Skill {
        start: u64,
        end: u64,
        projection: String,
        identity: String,
        execution_environment_identity: String,
        locator: String,
        name: String,
        scope: WireSkillReferenceScope,
        catalog_generation: u64,
        entry_revision: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireWorkspaceReferenceKind {
    File,
    Directory,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireSkillReferenceScope {
    Workspace,
    User,
    System,
    Admin,
}

impl TryFrom<&UserInput> for WireUserInput {
    type Error = JournalCodecError;

    fn try_from(input: &UserInput) -> Result<Self, Self::Error> {
        Ok(Self {
            profile: PROFILE.to_owned(),
            text: input.as_str().to_owned(),
            references: input
                .references()
                .iter()
                .map(WireInputReference::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl TryFrom<WireUserInput> for UserInput {
    type Error = JournalCodecError;

    fn try_from(input: WireUserInput) -> Result<Self, Self::Error> {
        if input.profile != PROFILE {
            return Err(JournalCodecError::new(format!(
                "unsupported structured input profile {:?}",
                input.profile
            )));
        }
        validate_persisted_v1(&input.text, &input.references)?;
        let references = input
            .references
            .into_iter()
            .map(InputReference::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(UserInput::from_validated_persisted_v1(
            input.text, references,
        ))
    }
}

impl TryFrom<&InputReference> for WireInputReference {
    type Error = JournalCodecError;

    fn try_from(reference: &InputReference) -> Result<Self, Self::Error> {
        let span = reference.span();
        let start = u64::try_from(span.start)
            .map_err(|_| JournalCodecError::new("input reference start exceeds u64"))?;
        let end = u64::try_from(span.end)
            .map_err(|_| JournalCodecError::new("input reference end exceeds u64"))?;
        match reference {
            InputReference::Workspace {
                projection,
                reference,
                ..
            } => Ok(Self::Workspace {
                start,
                end,
                projection: projection.clone(),
                identity: reference.identity().to_owned(),
                execution_environment_identity: reference
                    .execution_environment_identity()
                    .to_owned(),
                workspace_identity: reference.workspace_identity().to_owned(),
                root_identity: reference.root_identity().to_owned(),
                relative_path: reference.relative_path().to_owned(),
                kind: match reference.kind() {
                    WorkspaceReferenceKind::File => WireWorkspaceReferenceKind::File,
                    WorkspaceReferenceKind::Directory => WireWorkspaceReferenceKind::Directory,
                },
            }),
            InputReference::Skill {
                projection,
                reference,
                ..
            } => Ok(Self::Skill {
                start,
                end,
                projection: projection.clone(),
                identity: reference.identity().to_owned(),
                execution_environment_identity: reference
                    .execution_environment_identity()
                    .to_owned(),
                locator: reference.locator().to_owned(),
                name: reference.name().to_owned(),
                scope: match reference.scope() {
                    SkillReferenceScope::Workspace => WireSkillReferenceScope::Workspace,
                    SkillReferenceScope::User => WireSkillReferenceScope::User,
                    SkillReferenceScope::System => WireSkillReferenceScope::System,
                    SkillReferenceScope::Admin => WireSkillReferenceScope::Admin,
                },
                catalog_generation: reference.catalog_generation(),
                entry_revision: reference.entry_revision().to_owned(),
            }),
        }
    }
}

impl TryFrom<WireInputReference> for InputReference {
    type Error = JournalCodecError;

    fn try_from(reference: WireInputReference) -> Result<Self, Self::Error> {
        match reference {
            WireInputReference::Workspace {
                start,
                end,
                projection,
                identity,
                execution_environment_identity,
                workspace_identity,
                root_identity,
                relative_path,
                kind,
            } => {
                let span = decoded_span(start, end)?;
                let reference = WorkspaceReference::from_validated_persisted_v1(
                    identity,
                    execution_environment_identity,
                    workspace_identity,
                    root_identity,
                    relative_path,
                    match kind {
                        WireWorkspaceReferenceKind::File => WorkspaceReferenceKind::File,
                        WireWorkspaceReferenceKind::Directory => WorkspaceReferenceKind::Directory,
                    },
                );
                Ok(Self::persisted_workspace(span, projection, reference))
            },
            WireInputReference::Skill {
                start,
                end,
                projection,
                identity,
                execution_environment_identity,
                locator,
                name,
                scope,
                catalog_generation,
                entry_revision,
            } => Ok(Self::persisted_skill(
                decoded_span(start, end)?,
                projection,
                SkillReference::from_validated_persisted_v1(
                    identity,
                    execution_environment_identity,
                    locator,
                    name,
                    match scope {
                        WireSkillReferenceScope::Workspace => SkillReferenceScope::Workspace,
                        WireSkillReferenceScope::User => SkillReferenceScope::User,
                        WireSkillReferenceScope::System => SkillReferenceScope::System,
                        WireSkillReferenceScope::Admin => SkillReferenceScope::Admin,
                    },
                    catalog_generation,
                    entry_revision,
                ),
            )),
        }
    }
}

fn decoded_span(start: u64, end: u64) -> Result<std::ops::Range<usize>, JournalCodecError> {
    Ok(usize::try_from(start)
        .map_err(|_| JournalCodecError::new("input reference start cannot be addressed"))?
        ..usize::try_from(end)
            .map_err(|_| JournalCodecError::new("input reference end cannot be addressed"))?)
}

fn validate_persisted_v1(
    text: &str,
    references: &[WireInputReference],
) -> Result<(), JournalCodecError> {
    let mut previous_end = 0_u64;
    let mut skill_count = 0_u8;
    for (index, reference) in references.iter().enumerate() {
        let (start, end, projection) = reference.location();
        let span = decoded_span(start, end)?;
        if start >= end {
            return Err(invalid_input(index, "reference span is empty"));
        }
        if span.end > text.len()
            || !text.is_char_boundary(span.start)
            || !text.is_char_boundary(span.end)
        {
            return Err(invalid_input(
                index,
                "reference span is not a UTF-8 boundary",
            ));
        }
        if index > 0 && start < previous_end {
            return Err(invalid_input(
                index,
                "references overlap or are out of order",
            ));
        }
        if projection.is_empty() || text.get(span) != Some(projection) {
            return Err(invalid_input(
                index,
                "projection does not match its text span",
            ));
        }
        match reference {
            WireInputReference::Workspace {
                identity,
                execution_environment_identity,
                workspace_identity,
                root_identity,
                relative_path,
                ..
            } => {
                if [
                    identity.as_str(),
                    execution_environment_identity.as_str(),
                    workspace_identity.as_str(),
                    root_identity.as_str(),
                ]
                .contains(&"")
                {
                    return Err(invalid_input(index, "workspace identity metadata is empty"));
                }
                if !is_persisted_v1_relative_path(relative_path) {
                    return Err(JournalCodecError::new(
                        "invalid workspace reference: path must be canonical and root-relative",
                    ));
                }
            },
            WireInputReference::Skill {
                identity,
                execution_environment_identity,
                locator,
                name,
                catalog_generation,
                entry_revision,
                ..
            } => {
                skill_count = skill_count.saturating_add(1);
                if [
                    identity.as_str(),
                    execution_environment_identity.as_str(),
                    locator.as_str(),
                    name.as_str(),
                    entry_revision.as_str(),
                ]
                .contains(&"")
                    || *catalog_generation == 0
                {
                    return Err(invalid_input(index, "skill metadata is invalid"));
                }
            },
        }
        previous_end = end;
    }
    if skill_count > 1 {
        return Err(JournalCodecError::new(
            "invalid structured input: at most one skill occurrence is supported",
        ));
    }
    Ok(())
}

impl WireInputReference {
    fn location(&self) -> (u64, u64, &str) {
        match self {
            Self::Workspace {
                start,
                end,
                projection,
                ..
            }
            | Self::Skill {
                start,
                end,
                projection,
                ..
            } => (*start, *end, projection),
        }
    }
}

fn is_persisted_v1_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
}

fn invalid_input(index: usize, detail: &str) -> JournalCodecError {
    JournalCodecError::new(format!(
        "invalid structured input reference {index}: {detail}"
    ))
}
