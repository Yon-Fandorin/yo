use std::{
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use serde::Deserialize;
use yo_core::{
    AccountId, ConnectionAccount, ConnectionCatalogSeed, EffectiveModelBinding,
    EffectiveModelProfile, ModelId, ModelProfileLayer, ModelProfileParameters, NormalizedEndpoint,
    ProviderId, StoredModelBinding, VersionedProfileId,
};

use crate::AppError;

pub(super) const MAX_DEFINITION_BYTES: usize = 1_048_576;
const MAX_MODELS: usize = 4_096;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

#[derive(Debug)]
pub(super) struct ImportedDefinition {
    pub(super) account: ConnectionAccount,
    pub(super) bindings: Vec<StoredModelBinding>,
    pub(super) catalog_seed: Option<ConnectionCatalogSeed>,
}

impl ImportedDefinition {
    pub(super) fn provider(&self) -> &ProviderId {
        self.account.provider_id()
    }

    pub(super) fn account_id(&self) -> &AccountId {
        self.account.account_id()
    }
}

pub(super) fn read(source: &Path) -> Result<ImportedDefinition, AppError> {
    let bytes = if source.as_os_str() == "-" {
        read_bounded(std::io::stdin().lock(), "standard input")?
    } else {
        if !source.is_absolute() {
            return Err(AppError::message(format!(
                "--from requires an absolute path or exact '-': {}",
                source.display()
            )));
        }
        read_file(source)?
    };
    let contents = std::str::from_utf8(&bytes)
        .map_err(|_| AppError::message("connection definition must contain valid UTF-8"))?;
    parse(contents)
}

fn read_file(path: &Path) -> Result<Vec<u8>, AppError> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| AppError::single("opening the connection definition", error))?;
    let before = FileMetadata::capture(&file)?;
    if before.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
        return Err(AppError::message(format!(
            "connection definition must be a regular file: {}",
            path.display()
        )));
    }
    if before.len > MAX_DEFINITION_BYTES as u64 {
        return Err(AppError::message(format!(
            "connection definition exceeds the {MAX_DEFINITION_BYTES}-byte limit"
        )));
    }
    let bytes = read_bounded(&mut file, "connection definition")?;
    let after = FileMetadata::capture(&file)?;
    if before != after || after.len != bytes.len() as u64 {
        return Err(AppError::message(
            "connection definition changed while it was being read; retry with a stable file",
        ));
    }
    Ok(bytes)
}

fn read_bounded(mut input: impl Read, label: &str) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(MAX_DEFINITION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::single("reading the connection definition", error))?;
    if bytes.len() > MAX_DEFINITION_BYTES {
        return Err(AppError::message(format!(
            "{label} exceeds the {MAX_DEFINITION_BYTES}-byte connection definition limit"
        )));
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileMetadata {
    device: u64,
    inode: u64,
    mode: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl FileMetadata {
    fn capture(file: &fs::File) -> Result<Self, AppError> {
        let metadata = file
            .metadata()
            .map_err(|error| AppError::single("inspecting the connection definition", error))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            len: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    provider: String,
    #[serde(default)]
    provider_display_name: Authored<String>,
    account: String,
    #[serde(default)]
    account_display_name: Authored<String>,
    #[serde(default)]
    catalog: Authored<String>,
    #[serde(default)]
    base_url: Authored<String>,
    #[serde(default)]
    profile: Authored<Profile>,
    #[serde(default)]
    models: Authored<Vec<Model>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    model: String,
    #[serde(default)]
    model_display_name: Authored<String>,
    #[serde(default)]
    profile: Authored<Profile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    #[serde(default)]
    api_dialect: Authored<String>,
    #[serde(default)]
    tokenizer_profile: Authored<String>,
    #[serde(default)]
    input_token_limit: Authored<u64>,
    #[serde(default)]
    max_output_tokens: Authored<u64>,
    #[serde(default)]
    reasoning_parameters: Authored<ModelProfileParameters>,
    #[serde(default)]
    optional_request_parameters: Authored<ModelProfileParameters>,
    #[serde(default)]
    tool_capability_policy: Authored<String>,
    #[serde(default)]
    replay_profile: Authored<String>,
}

#[derive(Debug, Default)]
enum Authored<T> {
    #[default]
    Missing,
    Present(T),
}

impl<T> Authored<T> {
    fn into_option(self) -> Option<T> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(value),
        }
    }
}

impl<'de, T> Deserialize<'de> for Authored<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer)?
            .map(Self::Present)
            .ok_or_else(|| serde::de::Error::custom("authored fields cannot be null"))
    }
}

pub(super) fn parse(contents: &str) -> Result<ImportedDefinition, AppError> {
    let decoded: Definition = yo_yaml::from_str_with_limits(
        contents,
        yo_yaml::ParseLimits::with_max_total_scalar_bytes(MAX_DEFINITION_BYTES),
    )
    .map_err(|error| AppError::single("decoding the connection definition", error))?;
    resolve(decoded)
}

fn resolve(decoded: Definition) -> Result<ImportedDefinition, AppError> {
    let invalid = |error| AppError::single("validating the connection definition", error);
    let provider = ProviderId::new(decoded.provider).map_err(&invalid)?;
    let account = AccountId::new(decoded.account).map_err(&invalid)?;
    let provider_display_name = decoded.provider_display_name.into_option();
    let account_display_name = decoded.account_display_name.into_option();
    let stored_account = ConnectionAccount::new(
        provider.clone(),
        account.clone(),
        provider_display_name.clone(),
        account_display_name.clone(),
    )
    .map_err(&invalid)?;

    if let Some(catalog) = decoded.catalog.into_option() {
        if decoded.base_url.into_option().is_some()
            || decoded.profile.into_option().is_some()
            || decoded.models.into_option().is_some()
        {
            return Err(AppError::message(
                "a catalog definition cannot also author base_url, profile, or models",
            ));
        }
        let catalog_seed = ConnectionCatalogSeed::built_in(
            VersionedProfileId::new(catalog).map_err(&invalid)?,
            provider,
            account,
            provider_display_name,
            account_display_name,
        )
        .map_err(&invalid)?;
        return Ok(ImportedDefinition {
            account: stored_account,
            bindings: Vec::new(),
            catalog_seed: Some(catalog_seed),
        });
    }

    let base_url = decoded
        .base_url
        .into_option()
        .ok_or_else(|| AppError::message("an explicit definition requires base_url"))?;
    let base_profile = decoded
        .profile
        .into_option()
        .ok_or_else(|| AppError::message("an explicit definition requires profile"))?;
    let endpoint = NormalizedEndpoint::parse(&base_url).map_err(&invalid)?;
    let base_profile = profile_layer(base_profile)?;
    let models = decoded.models.into_option();
    if models.is_none() {
        if provider.as_str() != "openrouter" {
            return Err(AppError::message(
                "only an OpenRouter discovery seed may omit models",
            ));
        }
        let profile =
            EffectiveModelProfile::resolve(Some(&base_profile), &ModelProfileLayer::default())
                .map_err(&invalid)?;
        let seed = ConnectionCatalogSeed::openrouter(
            provider,
            account,
            provider_display_name,
            account_display_name,
            endpoint,
            profile,
        )
        .map_err(&invalid)?;
        return Ok(ImportedDefinition {
            account: stored_account,
            bindings: Vec::new(),
            catalog_seed: Some(seed),
        });
    }
    let models = models.expect("the missing case returned above");
    if models.is_empty() || models.len() > MAX_MODELS {
        return Err(AppError::message(format!(
            "an explicit definition requires 1 to {MAX_MODELS} models"
        )));
    }
    let mut seen = HashSet::new();
    let mut bindings = Vec::with_capacity(models.len());
    for model in models {
        let model_id = ModelId::new(model.model).map_err(&invalid)?;
        if !seen.insert(model_id.clone()) {
            return Err(AppError::message(format!(
                "connection definition repeats ModelId {model_id}"
            )));
        }
        let layer = model
            .profile
            .into_option()
            .map(profile_layer)
            .transpose()?
            .unwrap_or_default();
        let profile =
            EffectiveModelProfile::resolve(Some(&base_profile), &layer).map_err(&invalid)?;
        let effective = EffectiveModelBinding::new(
            provider.clone(),
            account.clone(),
            model_id,
            profile.api_dialect(),
            endpoint.clone(),
        );
        let complete = yo_core::CompleteModelBinding::new(effective, profile).map_err(&invalid)?;
        bindings.push(
            StoredModelBinding::new(complete, model.model_display_name.into_option())
                .map_err(&invalid)?,
        );
    }
    Ok(ImportedDefinition {
        account: stored_account,
        bindings,
        catalog_seed: None,
    })
}

fn profile_layer(profile: Profile) -> Result<ModelProfileLayer, AppError> {
    let invalid = |error| AppError::single("validating the connection profile", error);
    Ok(ModelProfileLayer::new(
        profile
            .api_dialect
            .into_option()
            .map(|value| value.parse())
            .transpose()
            .map_err(&invalid)?,
        profile
            .tokenizer_profile
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(&invalid)?,
        profile.input_token_limit.into_option(),
        profile.max_output_tokens.into_option(),
        profile.reasoning_parameters.into_option(),
        profile.optional_request_parameters.into_option(),
        profile
            .tool_capability_policy
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(&invalid)?,
    )
    .with_replay_profile(
        profile
            .replay_profile
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(invalid)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "api_dialect: openai-chat-completions\ntokenizer_profile: utf8-bytes/v1\ninput_token_limit: 1000\nmax_output_tokens: 100\nreasoning_parameters: {}\noptional_request_parameters: {}\ntool_capability_policy: local-tools/v1";

    // Base profile 하나를 상속한 여러 모델이 같은 Provider·Account definition으로 해석됩니다.
    #[test]
    fn resolves_multiple_models_from_one_base_profile() {
        let definition = parse(&format!(
            "provider: vendor\naccount: team\nbase_url: https://example.test/v1\nprofile:\n  {}\nmodels:\n  - model: alpha\n  - model: beta\n",
            PROFILE.replace('\n', "\n  ")
        ))
        .unwrap();
        assert_eq!(definition.bindings.len(), 2);
        assert_eq!(definition.bindings[0].selection().model().as_str(), "alpha");
        assert_eq!(definition.bindings[1].selection().model().as_str(), "beta");
    }

    // 중복 ModelId와 명시적 whole-field null은 생략으로 축약하지 않고 문서 전체를 거절합니다.
    #[test]
    fn rejects_duplicate_models_and_whole_field_null() {
        for suffix in [
            "models:\n  - model: alpha\n  - model: alpha\n",
            "models: null\n",
            "provider_display_name: null\nmodels:\n  - model: alpha\n",
            "models:\n  - model: alpha\n    model_display_name: null\n",
        ] {
            let error = parse(&format!(
                "provider: vendor\naccount: team\nbase_url: https://example.test/v1\nprofile:\n  {}\n{suffix}",
                PROFILE.replace('\n', "\n  ")
            ))
            .unwrap_err()
            .to_string();
            assert!(error.contains("repeats ModelId") || error.contains("cannot be null"));
        }
    }

    // Model 목록 생략은 bounded discovery seed를 소유한 OpenRouter에만 허용합니다.
    #[test]
    fn openrouter_seed_is_the_only_model_omission() {
        let seed = parse(&format!(
            "provider: openrouter\naccount: team\nbase_url: https://openrouter.ai/api/v1\nprofile:\n  {}\n",
            PROFILE.replace('\n', "\n  ")
        ))
        .unwrap();
        assert!(seed.bindings.is_empty());
        assert!(seed.catalog_seed.is_some());
    }
}
