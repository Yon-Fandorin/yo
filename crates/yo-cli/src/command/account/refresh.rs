use yo_backend_delegated_codex::{
    CodexBackendConfig, read_account_capacity as read_codex_account_capacity,
};
use yo_backend_delegated_grok::{
    GrokBackendConfig, read_account_capacity as read_grok_account_capacity,
};
use yo_core::{
    AccountCapacitySnapshot, AccountId, ApiCredential, CredentialSnapshot, KimiCatalogSeed,
    LocalConnectionOperationRepositories, LocalConnectionOperationSession,
    LocalConnectionRepository, LocalCredentialRepository, ProviderId, read_kimi_account_capacity,
};

use super::{
    domain::{
        AccountCapacityReport, AccountProviderData, AccountQuery, AccountTarget,
        has_qwencloud_token_plan_binding,
    },
    input::{HiddenSecretAction, HiddenSecretPrompt, read_hidden_secret},
    qwencloud,
};
use crate::{AppError, interaction::diagnostic::CliDiagnostic, state::config};

#[cfg(test)]
mod tests;

pub(super) struct AccountRefreshResult {
    pub(super) report: AccountCapacityReport,
    pub(super) diagnostics: Vec<CliDiagnostic>,
}

pub(super) fn refresh_target(target: &AccountTarget) -> Result<AccountRefreshResult, AppError> {
    let (report, diagnostics) = match target.provider().as_str() {
        "codex" => read_codex_capacity()?,
        "grok" => (read_grok_capacity()?, Vec::new()),
        "kimi" => (
            AccountCapacityReport::plain(read_kimi_capacity(
                target
                    .exact_coordinate()
                    .ok_or_else(|| AppError::message("Kimi requires an exact account target"))?
                    .account
                    .as_str(),
            )?),
            Vec::new(),
        ),
        "qwencloud" => (
            read_qwencloud_capacity(
                target
                    .exact_coordinate()
                    .ok_or_else(|| AppError::message("QwenCloud requires an exact account target"))?
                    .account
                    .as_str(),
            )?,
            Vec::new(),
        ),
        _ => {
            return Err(AppError::message(format!(
                "unsupported account-capacity source `{}`",
                target.reference()
            )));
        },
    };
    Ok(AccountRefreshResult {
        report: report.with_observed_at(current_observed_at()),
        diagnostics,
    })
}

pub(super) fn query_exact_account(query: &AccountQuery) -> Option<&str> {
    match query {
        AccountQuery::Exact(target) => Some(target.account.as_str()),
        _ => None,
    }
}

pub(super) fn current_observed_at() -> String {
    let now = jiff::Timestamp::now();
    jiff::Timestamp::from_second(now.as_second())
        .expect("the current timestamp rounded to seconds remains in range")
        .to_string()
}

pub(super) fn read_qwencloud_capacity(account: &str) -> Result<AccountCapacityReport, AppError> {
    let provider = ProviderId::new("qwencloud")
        .map_err(|error| AppError::single("resolving the QwenCloud Provider", error))?;
    let account = AccountId::new(account)
        .map_err(|error| AppError::single("resolving the QwenCloud Account", error))?;
    let config = config::load().map_err(|error| AppError::single("loading config", error))?;
    let state_directory = config.state_directory();
    let repositories = LocalConnectionOperationRepositories::in_directory(state_directory)
        .map_err(|error| AppError::single("opening connection repositories", error))?;
    let mut operation = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    operation
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;
    let connections = operation
        .capture_connections()
        .map_err(|error| AppError::single("reading stored model connections", error))?;
    if !has_qwencloud_token_plan_binding(&connections, &provider, &account) {
        return Err(AppError::message(format!(
            "no stored QwenCloud Token Plan account `{account}`"
        )));
    }
    let credentials = operation
        .capture_credentials()
        .map_err(|error| AppError::single("reading stored account credentials", error))?;
    let credential_repository = LocalCredentialRepository::new(config.credential_path());
    let (snapshot, provider_data) = refresh_qwencloud_capacity_with(
        &mut operation,
        &credential_repository,
        credentials,
        &provider,
        &account,
        |expired| capture_qwencloud_account_session(&provider, &account, expired),
        |session| qwencloud::read_account_capacity(&provider, &account, session),
    )?;
    Ok(AccountCapacityReport {
        snapshot,
        provider_data: Some(AccountProviderData::QwenCloud(provider_data)),
        observed_at: None,
    })
}

pub(super) fn refresh_qwencloud_capacity_with<T>(
    operation: &mut LocalConnectionOperationSession<'_>,
    credential_repository: &LocalCredentialRepository,
    mut credentials: CredentialSnapshot,
    provider: &ProviderId,
    account: &AccountId,
    mut capture_session: impl FnMut(bool) -> Result<ApiCredential, AppError>,
    mut refresh: impl FnMut(&ApiCredential) -> Result<T, qwencloud::QwenCloudCapacityError>,
) -> Result<T, AppError> {
    if credentials.resolve(provider, account).is_none() {
        return Err(AppError::message(format!(
            "credentials.yaml has no API credential for Provider {provider} and Account {account}"
        )));
    }
    let session = match credentials
        .resolve_account_session(provider, account)
        .cloned()
    {
        Some(session) => session,
        None => {
            let mutation = credentials
                .prepare_set_account_session(provider, account)
                .map_err(|error| {
                    AppError::single("preparing the account-session credential", error)
                })?;
            let session = qwencloud::validate_account_session(capture_session(false)?)?;
            persist_account_session(credential_repository, &mutation, &session)?;
            credentials = operation.capture_credentials().map_err(|error| {
                AppError::single("checking the persisted account-session credential", error)
            })?;
            if credentials.revision() != mutation.planned_revision() {
                return Err(AppError::message(
                    "credentials.yaml changed immediately after the account session was saved; retry with the current account state",
                ));
            }
            session
        },
    };

    match refresh(&session) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if error.is_expired_session() => {
            let mutation = credentials
                .prepare_set_account_session(provider, account)
                .map_err(|error| {
                    AppError::single("preparing the account-session replacement", error)
                })?;
            let replacement = qwencloud::validate_account_session(capture_session(true)?)?;
            persist_account_session(credential_repository, &mutation, &replacement)?;
            refresh(&replacement).map_err(qwencloud::QwenCloudCapacityError::into_app_error)
        },
        Err(error) => Err(error.into_app_error()),
    }
}

pub(super) fn persist_account_session(
    credentials: &LocalCredentialRepository,
    mutation: &yo_core::PreparedAccountSessionMutation,
    session: &ApiCredential,
) -> Result<(), AppError> {
    credentials
        .commit_account_session(mutation, session)
        .map(drop)
        .map_err(|error| AppError::single("persisting the account-session credential", error))
}

pub(super) fn capture_qwencloud_account_session(
    provider: &ProviderId,
    account: &AccountId,
    expired: bool,
) -> Result<ApiCredential, AppError> {
    let prompt = qwencloud_account_session_prompt(provider, account, expired);
    read_hidden_secret(&prompt, "reading the QwenCloud browser Cookie")
        .map_err(|error| AppError::single("capturing the QwenCloud browser session", error))
}

pub(super) fn qwencloud_account_session_prompt(
    provider: &ProviderId,
    account: &AccountId,
    expired: bool,
) -> HiddenSecretPrompt {
    let action = if expired {
        HiddenSecretAction::Replace
    } else {
        HiddenSecretAction::Save
    };
    let detail = if expired {
        "Expired · replace the saved session"
    } else {
        "Not saved · save for future account refreshes"
    };
    HiddenSecretPrompt::new(
        "ACCOUNT ACCESS",
        format!("{provider}:{account}"),
        action,
        "Browser session",
        detail,
        "Paste the Cookie header from Billing > Subscription.",
        "It is stored locally and stays separate from the model API key.",
        "Cookie (hidden): ",
    )
}

pub(super) fn read_codex_capacity() -> Result<(AccountCapacityReport, Vec<CliDiagnostic>), AppError>
{
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let read = read_codex_account_capacity(CodexBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Codex account capacity", error))?;
    let (snapshot, warning) = read.into_parts();
    let diagnostics = warning
        .into_iter()
        .map(|warning| CliDiagnostic::warning(warning.to_string()))
        .collect();
    Ok((AccountCapacityReport::plain(snapshot), diagnostics))
}

pub(super) fn read_grok_capacity() -> Result<AccountCapacityReport, AppError> {
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let snapshot = read_grok_account_capacity(GrokBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Grok account capacity", error))?;
    Ok(AccountCapacityReport::plain(snapshot))
}

pub(super) fn read_kimi_capacity(account: &str) -> Result<AccountCapacitySnapshot, AppError> {
    let provider = ProviderId::new("kimi")
        .map_err(|error| AppError::single("resolving the Kimi Provider", error))?;
    let account = AccountId::new(account)
        .map_err(|error| AppError::single("resolving the Kimi Account", error))?;
    let config = config::load().map_err(|error| AppError::single("loading config", error))?;
    let connections = LocalConnectionRepository::new(config.connection_path())
        .capture()
        .map_err(|error| AppError::single("reading stored model connections", error))?;
    let seed = connections
        .kimi_catalog_seed(&provider, &account)
        .map_err(|error| AppError::single("resolving the stored Kimi account", error))?;
    let seed = match seed {
        Some(seed) => Some(seed),
        None => derive_kimi_code_seed(&connections, &provider, &account)?,
    }
    .ok_or_else(|| {
        AppError::message(format!(
            "no stored Kimi Code Membership account `{account}`"
        ))
    })?;
    let credentials = LocalCredentialRepository::new(config.credential_path())
        .capture()
        .map_err(|error| AppError::single("reading stored model credentials", error))?;
    let credential = credentials.resolve(&provider, &account).ok_or_else(|| {
        AppError::message(format!("no stored credential for Kimi account `{account}`"))
    })?;
    read_kimi_account_capacity(&seed, credential)
        .map_err(|error| AppError::single("refreshing Kimi account capacity", error))
}

pub(super) fn derive_kimi_code_seed(
    connections: &yo_core::ConnectionSnapshot,
    provider: &ProviderId,
    account: &AccountId,
) -> Result<Option<KimiCatalogSeed>, AppError> {
    for binding in connections
        .models()
        .iter()
        .map(|stored| stored.complete().binding())
        .filter(|binding| binding.provider_id() == provider && binding.account_id() == account)
    {
        if let Some(seed) = KimiCatalogSeed::from_code_membership_binding(binding)
            .map_err(|error| AppError::single("resolving the stored Kimi product", error))?
        {
            return Ok(Some(seed));
        }
    }
    Ok(None)
}
