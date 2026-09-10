use serde::{Deserialize, Deserializer, Serialize};

use super::super::JournalCodecError;
use crate::{
    InputImage, InputReference, ResolvedSkill, SkillReference, SkillReferenceScope, UserInput,
    WorkspaceReference, WorkspaceReferenceKind,
};

const PROFILE: &str = "yo.structured-input/v1";
const RESOLVED_PROFILE: &str = "yo.structured-input/v2";
const IMAGE_PROFILE: &str = "yo.structured-input/v3";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireUserInput {
    profile: String,
    text: String,
    references: Vec<WireInputReference>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "resolved_skill_field"
    )]
    resolved_skill: Option<WireResolvedSkill>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "images_field"
    )]
    images: Option<Vec<InputImage>>,
}

// Reject explicit null and stop before retaining a seventeenth occurrence.
fn images_field<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Vec<InputImage>>, D::Error> {
    struct ImagesVisitor;
    impl<'de> serde::de::Visitor<'de> for ImagesVisitor {
        type Value = Vec<InputImage>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("one to sixteen input image occurrences")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;
            let mut images = Vec::new();
            let mut source_bytes = 0_u64;
            let mut png_bytes = 0_usize;
            while images.len() < InputImage::MAX_OCCURRENCES {
                let Some(image) = sequence.next_element::<InputImage>()? else {
                    return Ok(images);
                };
                source_bytes = source_bytes
                    .checked_add(image.source_byte_length())
                    .ok_or_else(|| A::Error::custom("image source byte overflow"))?;
                png_bytes = png_bytes
                    .checked_add(image.snapshot().png().len())
                    .ok_or_else(|| A::Error::custom("image PNG byte overflow"))?;
                if source_bytes > InputImage::MAX_INPUT_SOURCE_BYTES
                    || png_bytes > crate::InputImageSnapshot::MAX_BYTES
                {
                    return Err(A::Error::custom(
                        "aggregate input image byte limit exceeded",
                    ));
                }
                images.push(image);
            }
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(A::Error::custom("input image occurrence limit exceeded"));
            }
            Ok(images)
        }
    }
    decoder.deserialize_seq(ImagesVisitor).map(Some)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireResolvedSkill {
    instructions: String,
}

// Missing is meaningful for v1; explicit null must not silently erase a snapshot.
fn resolved_skill_field<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<WireResolvedSkill>, D::Error> {
    WireResolvedSkill::deserialize(decoder).map(Some)
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
            profile: if !input.images().is_empty() {
                IMAGE_PROFILE
            } else if input.resolved_skill().is_some() {
                RESOLVED_PROFILE
            } else {
                PROFILE
            }
            .to_owned(),
            resolved_skill: input.resolved_skill().map(|skill| WireResolvedSkill {
                instructions: skill.instructions().to_owned(),
            }),
            images: (!input.images().is_empty()).then(|| input.images().to_vec()),
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
        if input.profile != PROFILE
            && input.profile != RESOLVED_PROFILE
            && input.profile != IMAGE_PROFILE
        {
            return Err(JournalCodecError::new(format!(
                "unsupported structured input profile {:?}",
                input.profile
            )));
        }
        let image_profile = input.profile == IMAGE_PROFILE;
        if image_profile
            != input
                .images
                .as_ref()
                .is_some_and(|images| !images.is_empty())
            || (!image_profile && input.images.is_some())
        {
            return Err(JournalCodecError::new(
                "image occurrences do not match structured input profile",
            ));
        }
        if image_profile {
            validate_wire_image_encoding(&input)?;
        }
        if !image_profile && (input.profile == RESOLVED_PROFILE) != input.resolved_skill.is_some() {
            return Err(JournalCodecError::new(
                "resolved skill snapshot does not match structured input profile",
            ));
        }
        validate_persisted_v1(&input.text, &input.references)?;
        let references = input
            .references
            .into_iter()
            .map(InputReference::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let mut decoded = UserInput::from_validated_persisted_v1(input.text, references);
        if let Some(images) = input.images {
            decoded = decoded
                .with_images(images)
                .map_err(|error| JournalCodecError::new(error.to_string()))?;
        }
        if let Some(snapshot) = input.resolved_skill {
            let reference = decoded
                .references()
                .iter()
                .find_map(InputReference::skill_reference)
                .ok_or_else(|| {
                    JournalCodecError::new("resolved skill snapshot has no selected skill")
                })?
                .clone();
            let skill = ResolvedSkill::new(reference, snapshot.instructions)
                .map_err(|error| JournalCodecError::new(error.to_string()))?;
            decoded
                .with_resolved_skill(skill)
                .map_err(|error| JournalCodecError::new(error.to_string()))
        } else {
            Ok(decoded)
        }
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

/// Uses the persistence owner's canonical writer so live admission and recovery charge
/// exactly the same escaping, base64, reference, snapshot and framing bytes.
pub(crate) fn validate_image_input_encoding(input: &UserInput) -> Result<(), JournalCodecError> {
    validate_wire_image_encoding(&WireUserInput::try_from(input)?)
}

fn validate_wire_image_encoding(input: &WireUserInput) -> Result<(), JournalCodecError> {
    struct EncodedBudget(usize);
    impl std::io::Write for EncodedBudget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let total = self
                .0
                .checked_add(bytes.len())
                .filter(|total| *total <= InputImage::MAX_ENCODED_INPUT_BYTES)
                .ok_or_else(|| {
                    std::io::Error::other("complete encoded image input exceeds 16 MiB")
                })?;
            self.0 = total;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(EncodedBudget(0), input)
        .map_err(|error| JournalCodecError::new(error.to_string()))
}

#[cfg(test)]
mod tests;
