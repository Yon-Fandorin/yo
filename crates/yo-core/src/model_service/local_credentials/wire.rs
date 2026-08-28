use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize, de};

use super::{
    super::{AccountId, ApiCredential, ProviderId},
    LocalCredentialStoreError,
    repository::{CredentialEntry, CredentialRevision},
    storage::MAX_CREDENTIAL_FILE_BYTES,
};

pub(super) struct DecodedCredentials {
    pub(super) revision: CredentialRevision,
    pub(super) entries: Vec<CredentialEntry>,
}

pub(super) fn decode(
    path: &Path,
    encoded: &[u8],
    revisionless_snapshot_revision: CredentialRevision,
) -> Result<DecodedCredentials, LocalCredentialStoreError> {
    let decoded: CredentialFile = yo_yaml::from_slice(encoded)
        .map_err(|_| LocalCredentialStoreError::InvalidContents(path.to_owned()))?;
    let revision = decoded
        .revision
        .map(|revision| parse_revision(path, &revision))
        .transpose()?
        .unwrap_or(revisionless_snapshot_revision);
    Ok(DecodedCredentials {
        revision,
        entries: decoded
            .providers
            .into_iter()
            .map(|((provider, account), credential)| CredentialEntry {
                provider,
                account,
                credential: credential.api_key,
                account_session: credential.account_session,
            })
            .collect(),
    })
}

pub(super) fn encode(
    revision: &str,
    entries: &[CredentialEntry],
) -> Result<Vec<u8>, LocalCredentialStoreError> {
    let mut providers: BTreeMap<&str, BTreeMap<&str, AccountCredentialRef<'_>>> = BTreeMap::new();
    for entry in entries {
        let previous = providers
            .entry(entry.provider.as_str())
            .or_default()
            .insert(
                entry.account.as_str(),
                AccountCredentialRef {
                    api_key: entry.credential.expose_secret(),
                    account_session: entry
                        .account_session
                        .as_ref()
                        .map(ApiCredential::expose_secret),
                },
            );
        if previous.is_some() {
            return Err(LocalCredentialStoreError::InvalidContents(PathBuf::new()));
        }
    }
    let encoded = yo_yaml::to_string(&CredentialFileRef {
        revision,
        providers,
    })
    .map(String::into_bytes)
    .map_err(|_| LocalCredentialStoreError::InvalidContents(PathBuf::new()))?;
    if encoded.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(LocalCredentialStoreError::PreparedTooLarge);
    }
    Ok(encoded)
}

pub(super) fn new_revision() -> Result<CredentialRevision, LocalCredentialStoreError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| LocalCredentialStoreError::Randomness(error.to_string()))?;
    let mut token = String::with_capacity(37);
    token.push_str("crev-");
    for byte in bytes {
        use fmt::Write as _;
        write!(token, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    Ok(CredentialRevision::managed(token))
}

fn parse_revision(
    path: &Path,
    revision: &str,
) -> Result<CredentialRevision, LocalCredentialStoreError> {
    parse_managed_revision_token(revision)
        .map(CredentialRevision::managed)
        .ok_or_else(|| LocalCredentialStoreError::InvalidContents(path.to_owned()))
}

pub(super) fn parse_managed_revision_token(revision: &str) -> Option<String> {
    let valid = revision.len() == 37
        && revision.starts_with("crev-")
        && revision[5..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    valid.then(|| revision.to_owned())
}

pub(super) fn parse_derived_revision_token(revision: &str) -> Option<String> {
    let valid = revision.len() == 120
        && revision.starts_with("derived-")
        && revision[8..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    valid.then(|| revision.to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFile {
    #[serde(default)]
    revision: Option<String>,
    #[serde(deserialize_with = "deserialize_providers")]
    providers: Vec<((ProviderId, AccountId), DecodedAccountCredential)>,
}

#[derive(Serialize)]
struct CredentialFileRef<'a> {
    revision: &'a str,
    providers: BTreeMap<&'a str, BTreeMap<&'a str, AccountCredentialRef<'a>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountCredential {
    api_key: String,
    #[serde(default)]
    account_session: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
struct AccountCredentialRef<'a> {
    api_key: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_session: Option<&'a str>,
}

struct DecodedAccountCredential {
    api_key: ApiCredential,
    account_session: Option<ApiCredential>,
}

struct ProviderCredentials(Vec<(AccountId, DecodedAccountCredential)>);

impl<'de> Deserialize<'de> for ProviderCredentials {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AccountsVisitor;

        impl<'de> de::Visitor<'de> for AccountsVisitor {
            type Value = ProviderCredentials;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an AccountId-keyed credential mapping")
            }

            fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut accounts = Vec::with_capacity(mapping.size_hint().unwrap_or(0));
                let mut seen = HashSet::new();
                while let Some(account) = mapping.next_key::<String>()? {
                    let credential = mapping.next_value::<AccountCredential>()?;
                    let account = AccountId::new(account).map_err(de::Error::custom)?;
                    if !seen.insert(account.clone()) {
                        return Err(de::Error::custom("duplicate AccountId inside Provider"));
                    }
                    let credential = DecodedAccountCredential {
                        api_key: ApiCredential::new(credential.api_key)
                            .map_err(de::Error::custom)?,
                        account_session: credential
                            .account_session
                            .map(ApiCredential::new)
                            .transpose()
                            .map_err(de::Error::custom)?,
                    };
                    accounts.push((account, credential));
                }
                Ok(ProviderCredentials(accounts))
            }
        }

        deserializer.deserialize_map(AccountsVisitor)
    }
}

type ScopedCredential = ((ProviderId, AccountId), DecodedAccountCredential);

fn deserialize_providers<'de, D>(deserializer: D) -> Result<Vec<ScopedCredential>, D::Error>
where
    D: Deserializer<'de>,
{
    struct ProvidersVisitor;

    impl<'de> de::Visitor<'de> for ProvidersVisitor {
        type Value = Vec<ScopedCredential>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a ProviderId-keyed credential mapping")
        }

        fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut credentials = Vec::new();
            let mut seen = HashSet::new();
            while let Some(provider) = mapping.next_key::<String>()? {
                let provider_accounts = mapping.next_value::<ProviderCredentials>()?;
                let provider = ProviderId::new(provider).map_err(de::Error::custom)?;
                if !seen.insert(provider.clone()) {
                    return Err(de::Error::custom("duplicate ProviderId"));
                }
                credentials.extend(
                    provider_accounts
                        .0
                        .into_iter()
                        .map(|(account, credential)| ((provider.clone(), account), credential)),
                );
            }
            Ok(credentials)
        }
    }

    deserializer.deserialize_map(ProvidersVisitor)
}
