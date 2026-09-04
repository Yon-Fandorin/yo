use std::{collections::BTreeMap, fmt::Write as _, io::IsTerminal as _, num::NonZeroU16};

use yo_backend_delegated_codex::{
    CodexBackendConfig, read_account_capacity as read_codex_account_capacity,
};
use yo_backend_delegated_grok::{
    GrokBackendConfig, read_account_capacity as read_grok_account_capacity,
};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId,
    ApiCredential, CredentialSnapshot, KimiCatalogSeed, LocalConnectionOperationRepositories,
    LocalConnectionOperationSession, LocalConnectionRepository, LocalCredentialRepository,
    ProviderId, read_kimi_account_capacity,
};

mod json;
mod qwencloud;
mod storage;

use unicode_segmentation::UnicodeSegmentation;
use yo_tui::{
    GlyphProfile,
    meter::{MeterGlyphs, MeterShape, MeterSpec, MeterTemplate},
    surface::cell_width,
};

use super::{
    AppError,
    command::{AccountCommand, OutputFormat},
    config,
    connection::{HiddenSecretAction, HiddenSecretPrompt, read_hidden_secret},
    diagnostic::CliDiagnostic,
    presentation::{PresentationStyle, TextStyle, remaining_bar},
};

pub(crate) struct AccountRunOutput {
    pub(crate) output: String,
    pub(crate) diagnostics: Vec<CliDiagnostic>,
    pub(crate) completion: AccountCompletion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountCompletion {
    Success,
    RefreshFailures,
}

const ACCOUNT_TABLE_METER_TEMPLATE: MeterTemplate<'static> =
    MeterTemplate::new("{label} {meter} {percent}%");

fn account_table_meter(glyph_profile: GlyphProfile) -> MeterSpec<'static> {
    MeterSpec::new(
        MeterShape::VerticalLevel,
        MeterGlyphs::for_profile(glyph_profile),
        ACCOUNT_TABLE_METER_TEMPLATE,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccountOutputWidth {
    Unbounded,
    Bounded(NonZeroU16),
    Unknown,
}

fn account_output_width() -> AccountOutputWidth {
    if !std::io::stdout().is_terminal() {
        return AccountOutputWidth::Unbounded;
    }
    yo_tui::terminal::current_width()
        .map_or(AccountOutputWidth::Unknown, AccountOutputWidth::Bounded)
}

pub(crate) fn run(command: AccountCommand) -> Result<AccountRunOutput, AppError> {
    let config = config::load().map_err(|error| AppError::single("loading config", error))?;
    let cache_path = config.account_capacity_path();
    let cached = storage::load(&cache_path)
        .map_err(|error| AppError::single("loading account capacity cache", error))?;
    let connection_repository = LocalConnectionRepository::new(config.connection_path());
    connection_repository
        .recover_pending_operation()
        .map_err(|error| AppError::single("checking pending connection operations", error))?;
    let connections = connection_repository
        .capture()
        .map_err(|error| AppError::single("reading stored model connections", error))?;
    let query = parse_query(command.source.as_deref())?;
    let targets = resolve_targets(&query, &connections, &cached, command.refresh)?;

    let mut refreshed = BTreeMap::new();
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    if command.refresh {
        for target in &targets {
            let display_target = target_display_reference(target, &cached);
            match refresh_target(target) {
                Ok(refresh) => {
                    for warning in refresh.diagnostics {
                        if !warnings.contains(&warning) {
                            warnings.push(warning);
                        }
                    }
                    let report = refresh.report;
                    let matches_exact = exact_refresh_matches(&query, target, &report);
                    if matches_exact == Some(false) {
                        failures.push(AccountRefreshFailure {
                            target: display_target.clone(),
                            message: format!(
                                "the authenticated account is `{}` (requested `{}`)",
                                report.account_label(),
                                query_exact_account(&query).unwrap_or_default()
                            ),
                        });
                    } else {
                        refreshed.insert(target.clone(), report);
                    }
                },
                Err(error) => failures.push(AccountRefreshFailure {
                    target: display_target,
                    message: error.to_string(),
                }),
            }
        }
        let updates = refreshed.values().cloned().collect::<Vec<_>>();
        if !updates.is_empty()
            && let Err(error) = storage::upsert(&cache_path, &updates)
        {
            failures.push(AccountRefreshFailure {
                target: "cache".to_owned(),
                message: format!("saving account capacity cache: {error}"),
            });
        }
    }

    let cached = cached
        .into_iter()
        .map(|report| (report.coordinate(), report))
        .collect::<BTreeMap<_, _>>();
    let records = targets
        .into_iter()
        .map(|target| {
            let report = refreshed.get(&target).cloned().or_else(|| {
                target
                    .exact_coordinate()
                    .and_then(|coordinate| cached.get(coordinate).cloned())
            });
            AccountCapacityRecord { target, report }
        })
        .collect::<Vec<_>>();
    let output = match command.output.format {
        OutputFormat::Text => Ok(render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &query,
                refreshed: command.refresh,
                detail: command.detail,
                glyph_profile: command.output.glyph_profile,
                warnings: &warnings,
                failures: &failures,
                output_width: account_output_width(),
                style: PresentationStyle::for_stdout(),
            },
        )),
        OutputFormat::Json => json::render_for_query(&records, &query, &failures)
            .map_err(|error| AppError::single("serializing account capacity JSON", error)),
    }?;
    Ok(AccountRunOutput {
        output,
        diagnostics: match command.output.format {
            OutputFormat::Text => Vec::new(),
            OutputFormat::Json => warnings,
        },
        completion: if failures.is_empty() {
            AccountCompletion::Success
        } else {
            AccountCompletion::RefreshFailures
        },
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AccountRefreshFailure {
    target: String,
    message: String,
}

#[derive(Clone)]
struct AccountCapacityReport {
    snapshot: AccountCapacitySnapshot,
    provider_data: Option<AccountProviderData>,
    observed_at: Option<String>,
}

impl AccountCapacityReport {
    fn plain(snapshot: AccountCapacitySnapshot) -> Self {
        Self {
            snapshot,
            provider_data: None,
            observed_at: None,
        }
    }

    fn from_cached(
        snapshot: AccountCapacitySnapshot,
        account_label: Option<String>,
        provider_data: Option<AccountProviderData>,
        observed_at: String,
    ) -> Self {
        let snapshot = match account_label {
            Some(label) => snapshot.with_account_label(label),
            None => snapshot,
        };
        Self {
            snapshot,
            provider_data,
            observed_at: Some(observed_at),
        }
    }

    fn with_observed_at(mut self, observed_at: String) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    #[cfg(test)]
    fn with_account_label(mut self, account_label: impl Into<String>) -> Self {
        self.snapshot = self.snapshot.with_account_label(account_label);
        self
    }

    fn coordinate(&self) -> AccountCoordinate {
        AccountCoordinate {
            provider: self.snapshot.provider().clone(),
            account: self.snapshot.account().clone(),
        }
    }

    fn snapshot(&self) -> &AccountCapacitySnapshot {
        &self.snapshot
    }

    fn account_label(&self) -> &str {
        self.snapshot.account_label()
    }

    fn provider_data(&self) -> Option<&AccountProviderData> {
        self.provider_data.as_ref()
    }

    fn observed_at(&self) -> Option<&str> {
        self.observed_at.as_deref()
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum AccountProviderData {
    QwenCloud(qwencloud::QwenCloudProviderData),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct AccountCoordinate {
    provider: ProviderId,
    account: AccountId,
}

impl AccountCoordinate {
    fn new(provider: &str, account: &str) -> Result<Self, AppError> {
        Ok(Self {
            provider: ProviderId::new(provider)
                .map_err(|error| AppError::single("resolving account Provider", error))?,
            account: AccountId::new(account)
                .map_err(|error| AppError::single("resolving account Account", error))?,
        })
    }

    fn reference(&self) -> String {
        format!("{}:{}", self.provider, self.account)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum AccountTarget {
    Exact(AccountCoordinate),
    LocalHost { provider: ProviderId },
}

impl AccountTarget {
    fn provider(&self) -> &ProviderId {
        match self {
            Self::Exact(coordinate) => &coordinate.provider,
            Self::LocalHost { provider } => provider,
        }
    }

    fn exact_coordinate(&self) -> Option<&AccountCoordinate> {
        match self {
            Self::Exact(coordinate) => Some(coordinate),
            Self::LocalHost { .. } => None,
        }
    }

    fn reference(&self) -> String {
        match self {
            Self::Exact(coordinate) => coordinate.reference(),
            Self::LocalHost { provider } => local_host_label(provider),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AccountQuery {
    All,
    Provider(ProviderId),
    Exact(AccountCoordinate),
}

#[derive(Clone)]
struct AccountCapacityRecord {
    target: AccountTarget,
    report: Option<AccountCapacityReport>,
}

fn parse_query(source: Option<&str>) -> Result<AccountQuery, AppError> {
    let Some(source) = source else {
        return Ok(AccountQuery::All);
    };
    if let Some((provider, account)) = source.split_once(':') {
        if provider.is_empty() || account.is_empty() || account.contains(':') {
            return Err(AppError::message(format!(
                "invalid account source `{source}`; use PROVIDER or PROVIDER:ACCOUNT"
            )));
        }
        return Ok(AccountQuery::Exact(AccountCoordinate::new(
            provider, account,
        )?));
    }
    Ok(AccountQuery::Provider(ProviderId::new(source).map_err(
        |error| AppError::single("resolving account Provider", error),
    )?))
}

fn resolve_targets(
    query: &AccountQuery,
    connections: &yo_core::ConnectionSnapshot,
    cached: &[AccountCapacityReport],
    refresh: bool,
) -> Result<Vec<AccountTarget>, AppError> {
    let mut known = Vec::new();
    for provider in ["codex", "grok"] {
        if let Some(report) = latest_host_cache(cached, provider) {
            known.push(AccountTarget::Exact(report.coordinate()));
        } else if host_command_available(provider) {
            known.push(AccountTarget::LocalHost {
                provider: ProviderId::new(provider)
                    .map_err(|error| AppError::single("resolving host Provider", error))?,
            });
        }
    }
    for account in connections.accounts() {
        let target = AccountCoordinate {
            provider: account.provider_id().clone(),
            account: account.account_id().clone(),
        };
        if is_stored_capacity_account(connections, &target)? {
            known.push(AccountTarget::Exact(target));
        }
    }
    known.sort();
    known.dedup();

    let targets = match query {
        AccountQuery::All => known,
        AccountQuery::Provider(provider) => known
            .into_iter()
            .filter(|target| target.provider() == provider)
            .collect(),
        AccountQuery::Exact(target) => {
            if let Some(candidate) = known.iter().find(|candidate| {
                candidate
                    .exact_coordinate()
                    .is_some_and(|coordinate| coordinate == target)
            }) {
                vec![candidate.clone()]
            } else if let Some(candidate) = cached.iter().find(|report| {
                report.snapshot().provider() == &target.provider
                    && account_selector_matches(target, report)
                    && cached_report_is_selectable(connections, report)
            }) {
                vec![AccountTarget::Exact(candidate.coordinate())]
            } else if refresh
                && matches!(target.provider.as_str(), "codex" | "grok")
                && host_command_available(target.provider.as_str())
            {
                vec![AccountTarget::Exact(target.clone())]
            } else {
                return Err(AppError::message(format!(
                    "no stored account-capacity source matches `{}`",
                    target.reference()
                )));
            }
        },
    };
    if targets.is_empty() {
        let description = match query {
            AccountQuery::All => "any supported Provider".to_owned(),
            AccountQuery::Provider(provider) => format!("Provider `{provider}`"),
            AccountQuery::Exact(target) => format!("`{}`", target.reference()),
        };
        return Err(AppError::message(format!(
            "no account-capacity sources are available for {description}"
        )));
    }
    Ok(targets)
}

struct AccountRefreshResult {
    report: AccountCapacityReport,
    diagnostics: Vec<CliDiagnostic>,
}

fn refresh_target(target: &AccountTarget) -> Result<AccountRefreshResult, AppError> {
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

fn query_exact_account(query: &AccountQuery) -> Option<&str> {
    match query {
        AccountQuery::Exact(target) => Some(target.account.as_str()),
        _ => None,
    }
}

fn account_selector_matches(target: &AccountCoordinate, report: &AccountCapacityReport) -> bool {
    target.account == *report.snapshot().account()
        || target
            .account
            .as_str()
            .strip_prefix("account-")
            .is_some_and(|legacy| legacy == report.snapshot().account().as_str())
        || report
            .snapshot()
            .account()
            .as_str()
            .strip_prefix("account-")
            .is_some_and(|legacy| legacy == target.account.as_str())
        || target.account.as_str() == report.account_label()
        || target
            .account
            .as_str()
            .eq_ignore_ascii_case(report.account_label())
}

fn exact_refresh_matches(
    query: &AccountQuery,
    target: &AccountTarget,
    report: &AccountCapacityReport,
) -> Option<bool> {
    match query {
        AccountQuery::Exact(expected)
            if expected.provider.as_str() == target.provider().as_str()
                && matches!(target.provider().as_str(), "codex" | "grok") =>
        {
            target
                .exact_coordinate()
                .map(|_| account_selector_matches(expected, report))
        },
        _ => None,
    }
}

fn cached_report_is_selectable(
    connections: &yo_core::ConnectionSnapshot,
    report: &AccountCapacityReport,
) -> bool {
    match report.snapshot().provider().as_str() {
        "codex" | "grok" => true,
        "kimi" | "qwencloud" => {
            is_stored_capacity_account(connections, &report.coordinate()).unwrap_or(false)
        },
        _ => false,
    }
}

fn target_display_reference(target: &AccountTarget, cached: &[AccountCapacityReport]) -> String {
    let Some(coordinate) = target.exact_coordinate() else {
        return target.reference();
    };
    cached
        .iter()
        .find(|report| report.coordinate() == *coordinate)
        .map(|report| format!("{}:{}", coordinate.provider, report.account_label()))
        .unwrap_or_else(|| coordinate.reference())
}

fn latest_host_cache<'a>(
    cached: &'a [AccountCapacityReport],
    provider: &str,
) -> Option<&'a AccountCapacityReport> {
    cached
        .iter()
        .filter(|report| report.snapshot().provider().as_str() == provider)
        .max_by_key(|report| report.observed_at().unwrap_or_default())
}

fn is_stored_capacity_account(
    connections: &yo_core::ConnectionSnapshot,
    target: &AccountCoordinate,
) -> Result<bool, AppError> {
    match target.provider.as_str() {
        "kimi" => Ok(has_kimi_code_membership_binding(connections, target)),
        "qwencloud" => Ok(has_qwencloud_token_plan_binding(
            connections,
            &target.provider,
            &target.account,
        )),
        _ => Ok(false),
    }
}

fn has_kimi_code_membership_binding(
    connections: &yo_core::ConnectionSnapshot,
    target: &AccountCoordinate,
) -> bool {
    connections
        .kimi_catalog_seed(&target.provider, &target.account)
        .ok()
        .flatten()
        .is_some_and(|seed| seed.profile().as_str() == "kimi-code-membership/v1")
        || connections.models().iter().any(|stored| {
            let binding = stored.complete().binding();
            binding.provider_id() == &target.provider
                && binding.account_id() == &target.account
                && KimiCatalogSeed::from_code_membership_binding(binding)
                    .ok()
                    .flatten()
                    .is_some()
        })
}

fn host_command_available(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(command);
        let Ok(metadata) = std::fs::metadata(candidate) else {
            return false;
        };
        metadata.is_file() && {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    })
}

fn current_observed_at() -> String {
    let now = jiff::Timestamp::now();
    jiff::Timestamp::from_second(now.as_second())
        .expect("the current timestamp rounded to seconds remains in range")
        .to_string()
}

fn read_qwencloud_capacity(account: &str) -> Result<AccountCapacityReport, AppError> {
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

fn refresh_qwencloud_capacity_with<T>(
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

fn persist_account_session(
    credentials: &LocalCredentialRepository,
    mutation: &yo_core::PreparedAccountSessionMutation,
    session: &ApiCredential,
) -> Result<(), AppError> {
    credentials
        .commit_account_session(mutation, session)
        .map(drop)
        .map_err(|error| AppError::single("persisting the account-session credential", error))
}

fn capture_qwencloud_account_session(
    provider: &ProviderId,
    account: &AccountId,
    expired: bool,
) -> Result<ApiCredential, AppError> {
    let prompt = qwencloud_account_session_prompt(provider, account, expired);
    read_hidden_secret(&prompt, "reading the QwenCloud browser Cookie")
        .map_err(|error| AppError::single("capturing the QwenCloud browser session", error))
}

fn qwencloud_account_session_prompt(
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

fn has_qwencloud_token_plan_binding(
    connections: &yo_core::ConnectionSnapshot,
    provider: &ProviderId,
    account: &AccountId,
) -> bool {
    connections
        .models()
        .iter()
        .map(|stored| stored.complete().binding())
        .any(|binding| {
            binding.provider_id() == provider
                && binding.account_id() == account
                && binding.endpoint().as_str().trim_end_matches('/')
                    == "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
        })
}

fn read_codex_capacity() -> Result<(AccountCapacityReport, Vec<CliDiagnostic>), AppError> {
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

fn read_grok_capacity() -> Result<AccountCapacityReport, AppError> {
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let snapshot = read_grok_account_capacity(GrokBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Grok account capacity", error))?;
    Ok(AccountCapacityReport::plain(snapshot))
}

fn read_kimi_capacity(account: &str) -> Result<AccountCapacitySnapshot, AppError> {
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

fn derive_kimi_code_seed(
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

#[cfg(test)]
fn render_records(
    records: &[AccountCapacityRecord],
    query: &AccountQuery,
    refreshed: bool,
    detail: bool,
    warnings: &[CliDiagnostic],
    failures: &[AccountRefreshFailure],
    style: PresentationStyle,
) -> String {
    render_records_with_width(
        records,
        AccountRenderOptions {
            query,
            refreshed,
            detail,
            glyph_profile: GlyphProfile::Rich,
            warnings,
            failures,
            output_width: AccountOutputWidth::Unbounded,
            style,
        },
    )
}

struct AccountRenderOptions<'a> {
    query: &'a AccountQuery,
    refreshed: bool,
    detail: bool,
    glyph_profile: GlyphProfile,
    warnings: &'a [CliDiagnostic],
    failures: &'a [AccountRefreshFailure],
    output_width: AccountOutputWidth,
    style: PresentationStyle,
}

fn render_records_with_width(
    records: &[AccountCapacityRecord],
    options: AccountRenderOptions<'_>,
) -> String {
    let detailed = options.detail || records.len() == 1 || !table_fits(records, &options);
    let AccountRenderOptions {
        query,
        refreshed,
        detail: _,
        glyph_profile,
        warnings,
        failures,
        output_width,
        style,
    } = options;
    let mut output = if detailed {
        render_detailed_records(records, style, glyph_profile, detail_width(output_width))
    } else {
        render_table_records(records, query, style, glyph_profile)
    };
    let mut commands = Vec::new();
    if !detailed {
        commands.push(("Detail", account_command(query, "--detail")));
    }
    if !refreshed && records.iter().any(|record| record.report.is_none()) {
        commands.push(("Refresh", account_command(query, "--refresh")));
    }
    if !failures.is_empty() {
        commands.push(("Retry", account_command(query, "--refresh")));
    }
    if !warnings.is_empty() {
        output.push('\n');
        push_heading(
            &mut output,
            "Refresh warnings",
            style,
            detail_width(output_width),
            TextStyle::Warning,
        );
        for warning in warnings {
            push_tail_plain_line(
                &mut output,
                &format!("  {}", terminal_safe(warning.message())),
                detail_width(output_width),
            );
        }
    }
    if !failures.is_empty() {
        output.push('\n');
        push_heading(
            &mut output,
            "Refresh failures",
            style,
            detail_width(output_width),
            TextStyle::Error,
        );
        for failure in failures {
            push_tail_plain_line(
                &mut output,
                &format!(
                    "  {} · {}",
                    terminal_safe(&failure.target),
                    terminal_safe(&failure.message)
                ),
                detail_width(output_width),
            );
        }
    }
    if !commands.is_empty() {
        output.push('\n');
        for (label, command) in commands {
            push_tail_styled_line(
                &mut output,
                &format!("{label}: {command}"),
                style,
                TextStyle::Muted,
                detail_width(output_width),
            );
        }
    }
    output
}

fn table_fits(records: &[AccountCapacityRecord], options: &AccountRenderOptions<'_>) -> bool {
    let output_width = options.output_width;
    let AccountOutputWidth::Bounded(terminal_width) = output_width else {
        return !matches!(output_width, AccountOutputWidth::Unknown);
    };
    let table = render_table_records(
        records,
        options.query,
        PresentationStyle::Plain,
        options.glyph_profile,
    );
    let tail = table_tail_lines(records, options);
    table
        .lines()
        .chain(tail.iter().map(String::as_str))
        .all(|line| {
            cell_width(line).is_ok_and(|line_width| line_width <= usize::from(terminal_width.get()))
        })
}

fn table_tail_lines(
    records: &[AccountCapacityRecord],
    options: &AccountRenderOptions<'_>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if !options.warnings.is_empty() {
        lines.push("Refresh warnings".to_owned());
        lines.extend(
            options
                .warnings
                .iter()
                .map(|warning| format!("  {}", terminal_safe(warning.message()))),
        );
    }
    if !options.failures.is_empty() {
        lines.push("Refresh failures".to_owned());
        lines.extend(options.failures.iter().map(|failure| {
            format!(
                "  {} · {}",
                terminal_safe(&failure.target),
                terminal_safe(&failure.message)
            )
        }));
    }
    lines.push(format!(
        "Detail: {}",
        account_command(options.query, "--detail")
    ));
    if !options.refreshed && records.iter().any(|record| record.report.is_none()) {
        lines.push(format!(
            "Refresh: {}",
            account_command(options.query, "--refresh")
        ));
    }
    if !options.failures.is_empty() {
        lines.push(format!(
            "Retry: {}",
            account_command(options.query, "--refresh")
        ));
    }
    lines
}

fn render_detailed_records(
    records: &[AccountCapacityRecord],
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    if records.len() > 1 {
        let heading = format!("Account capacity · {} accounts", records.len());
        if let Some(width) = width {
            push_wrapped_value(&mut output, "", &heading, width, style, TextStyle::Bold);
        } else {
            style.push(&mut output, TextStyle::Bold, &heading);
            output.push('\n');
        }
        output.push('\n');
    }
    for (index, record) in records.iter().enumerate() {
        if index != 0 {
            output.push('\n');
        }
        match &record.report {
            Some(report) => output.push_str(&render_report(report, style, glyph_profile, width)),
            None => output.push_str(&render_missing(&record.target, style, width)),
        }
    }
    output
}

fn detail_width(output_width: AccountOutputWidth) -> Option<usize> {
    match output_width {
        AccountOutputWidth::Bounded(width) => Some(usize::from(width.get())),
        AccountOutputWidth::Unbounded | AccountOutputWidth::Unknown => None,
    }
}

const TABLE_ACCOUNT_HEADINGS: [&str; 5] = ["PROVIDER", "ACCOUNT", "PLAN", "LIMITS", "UPDATED"];

#[derive(Clone, Debug)]
struct TableCell {
    plain: String,
    parts: Vec<TableCellPart>,
}

#[derive(Clone, Debug)]
struct TableCellPart {
    value: String,
    tone: Option<TextStyle>,
}

impl TableCell {
    fn from_parts(parts: Vec<TableCellPart>) -> Self {
        let mut plain = String::new();
        for part in &parts {
            plain.push_str(&part.value);
        }
        Self { plain, parts }
    }

    fn plain(value: impl Into<String>) -> Self {
        Self::from_parts(vec![TableCellPart {
            value: value.into(),
            tone: None,
        }])
    }

    fn styled(value: impl Into<String>, tone: TextStyle) -> Self {
        Self::from_parts(vec![TableCellPart {
            value: value.into(),
            tone: Some(tone),
        }])
    }

    fn width(&self) -> usize {
        cell_width(&self.plain).expect("account table cells must be terminal-safe")
    }

    fn render(
        &self,
        output: &mut String,
        style: PresentationStyle,
        default_tone: Option<TextStyle>,
    ) {
        for part in &self.parts {
            match part.tone.or(default_tone) {
                Some(tone) => style.push(output, tone, &part.value),
                None => output.push_str(&part.value),
            }
        }
    }
}

fn table_account_row(
    record: &AccountCapacityRecord,
    glyph_profile: GlyphProfile,
) -> Vec<TableCell> {
    let Some(report) = record.report.as_ref() else {
        return match &record.target {
            AccountTarget::LocalHost { provider } => vec![
                TableCell::plain(provider.as_str()),
                TableCell::styled("Local host", TextStyle::Muted),
                TableCell::styled("Unknown", TextStyle::Muted),
                TableCell::styled("Not refreshed", TextStyle::Muted),
                TableCell::styled("Never", TextStyle::Muted),
            ],
            AccountTarget::Exact(coordinate) => vec![
                TableCell::plain(coordinate.provider.as_str()),
                TableCell::plain(terminal_safe(coordinate.account.as_str())),
                TableCell::styled("Unknown", TextStyle::Muted),
                TableCell::styled("Not refreshed", TextStyle::Muted),
                TableCell::styled("Never", TextStyle::Muted),
            ],
        };
    };

    let snapshot = report.snapshot();
    let (status, status_tone) = display_status(snapshot);
    let mut limits = table_limit_cell(snapshot, glyph_profile);
    if status != "Available" {
        let mut parts = vec![TableCellPart {
            value: status,
            tone: Some(status_tone),
        }];
        if !limits.plain.is_empty() {
            parts.push(TableCellPart {
                value: " · ".to_owned(),
                tone: None,
            });
        }
        parts.extend(limits.parts);
        limits = TableCell::from_parts(parts);
    }

    let updated = report.observed_at().map_or_else(
        || TableCell::styled("Never", TextStyle::Muted),
        table_observed_at_cell,
    );
    vec![
        TableCell::plain(snapshot.provider().as_str()),
        TableCell::plain(terminal_safe(report.account_label())),
        TableCell::plain(display_plan(snapshot)),
        limits,
        updated,
    ]
}

fn render_table_records(
    records: &[AccountCapacityRecord],
    query: &AccountQuery,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
) -> String {
    let mut output = String::new();
    style.push(
        &mut output,
        TextStyle::Bold,
        &format!(
            "{} · {} {}",
            account_scope_label(query),
            records.len(),
            if records.len() == 1 {
                "account"
            } else {
                "accounts"
            }
        ),
    );
    output.push('\n');

    let headings = TABLE_ACCOUNT_HEADINGS
        .iter()
        .map(|heading| TableCell::plain(*heading))
        .collect::<Vec<_>>();
    let rows = records
        .iter()
        .map(|record| table_account_row(record, glyph_profile))
        .collect::<Vec<_>>();
    let widths = table_column_widths(&headings, &rows);
    push_table_row(
        &mut output,
        &headings,
        &widths,
        style,
        Some(TextStyle::Muted),
    );
    for row in &rows {
        push_table_row(&mut output, row, &widths, style, None);
    }
    output
}

fn table_column_widths(headings: &[TableCell], rows: &[Vec<TableCell>]) -> Vec<usize> {
    let mut widths = headings.iter().map(TableCell::width).collect::<Vec<_>>();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    widths
}

fn push_table_row(
    output: &mut String,
    cells: &[TableCell],
    widths: &[usize],
    style: PresentationStyle,
    default_tone: Option<TextStyle>,
) {
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        cell.render(output, style, default_tone);
        if index + 1 < cells.len() {
            output.push_str(&" ".repeat(width.saturating_sub(cell.width()) + 2));
        }
    }
    output.push('\n');
}

fn account_scope_label(query: &AccountQuery) -> String {
    match query {
        AccountQuery::All => "Account capacity".to_owned(),
        AccountQuery::Provider(provider) => {
            format!("{} account capacity", display_identifier(provider.as_str()))
        },
        AccountQuery::Exact(target) => target.reference(),
    }
}

fn account_command(query: &AccountQuery, flag: &str) -> String {
    let source = match query {
        AccountQuery::All => String::new(),
        AccountQuery::Provider(provider) => format!(" {provider}"),
        AccountQuery::Exact(target) => format!(" {}", shell_quote(&target.reference())),
    };
    format!("yo account{source} {flag}")
}

fn shell_quote(value: &str) -> String {
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+')
    }) {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn table_limit_cell(snapshot: &AccountCapacitySnapshot, glyph_profile: GlyphProfile) -> TableCell {
    let mut parts = Vec::new();
    for (index, (value, tone)) in table_limit_items(snapshot, glyph_profile)
        .into_iter()
        .enumerate()
    {
        if index != 0 {
            parts.push(TableCellPart {
                value: " · ".to_owned(),
                tone: None,
            });
        }
        parts.push(TableCellPart {
            value,
            tone: Some(tone),
        });
    }
    TableCell::from_parts(parts)
}

fn table_limit_items(
    snapshot: &AccountCapacitySnapshot,
    glyph_profile: GlyphProfile,
) -> Vec<(String, TextStyle)> {
    let mut limits = Vec::new();
    for (index, bucket) in snapshot.buckets().iter().enumerate() {
        let nested = is_additional_bucket(snapshot, bucket, index);
        let bucket_label = nested.then(|| table_bucket_label(bucket));
        for window in [bucket.primary(), bucket.secondary()].into_iter().flatten() {
            let mut label = table_window_label(window.window_duration_minutes());
            if let Some(bucket_label) = &bucket_label {
                label = format!("{bucket_label} {label}");
            }
            let remaining = window.remaining_percent_basis_points();
            let rendered = account_table_meter(glyph_profile)
                .render(&label, remaining)
                .expect("built-in account limit meter must remain renderable");
            limits.push((rendered, capacity_tone(remaining)));
        }
    }
    if limits.is_empty()
        && let Some(credits) = snapshot
            .buckets()
            .iter()
            .find_map(|bucket| bucket.credits())
    {
        return vec![(
            format!("credits {}", display_credits(credits)),
            TextStyle::Muted,
        )];
    }
    if limits.is_empty() {
        return vec![("No capacity".to_owned(), TextStyle::Muted)];
    }
    limits
}

fn table_bucket_label(bucket: &AccountCapacityBucket) -> String {
    let value = bucket
        .name()
        .or_else(|| bucket.id())
        .unwrap_or("Additional");
    if value.to_ascii_lowercase().contains("spark") {
        return "Spark".to_owned();
    }
    let value = display_identifier(value);
    let mut characters = value.chars();
    let compact = characters.by_ref().take(15).collect::<String>();
    if characters.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

fn table_window_label(minutes: Option<u64>) -> String {
    let Some(minutes) = minutes else {
        return "window".to_owned();
    };
    const MINUTES_PER_DAY: u64 = 24 * 60;
    if minutes == 7 * MINUTES_PER_DAY {
        return "7d".to_owned();
    }
    if minutes != 0 && minutes.is_multiple_of(60) {
        return format!("{}h", minutes / 60);
    }
    format!("{minutes}m")
}

fn display_observed_at_with_age(value: &str) -> String {
    let displayed = display_observed_at(value);
    let Ok(timestamp) = value.parse::<jiff::Timestamp>() else {
        return displayed;
    };
    let age = jiff::Timestamp::now()
        .as_second()
        .saturating_sub(timestamp.as_second());
    if age < 0 {
        return displayed;
    }
    format!("{displayed} ({})", display_age(age as u64))
}

fn table_observed_at_cell(value: &str) -> TableCell {
    let Ok(timestamp) = value.parse::<jiff::Timestamp>() else {
        return TableCell::plain(display_observed_at_with_age(value));
    };
    let displayed = timestamp
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M %Z")
        .to_string();
    let age = jiff::Timestamp::now()
        .as_second()
        .saturating_sub(timestamp.as_second());
    if age < 0 {
        return TableCell::plain(displayed);
    }
    TableCell::from_parts(vec![
        TableCellPart {
            value: displayed,
            tone: None,
        },
        TableCellPart {
            value: format!(" ({})", display_age(age as u64)),
            tone: Some(TextStyle::Muted),
        },
    ])
}

fn display_age(seconds: u64) -> String {
    if seconds < 60 {
        return "just now".to_owned();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h {}m ago", minutes % 60);
    }
    let days = hours / 24;
    format!("{days}d {}h ago", hours % 24)
}

fn render_missing(
    target: &AccountTarget,
    style: PresentationStyle,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    let (heading, account, heading_account) = match target {
        AccountTarget::LocalHost { provider } => {
            (local_host_label(provider), "Not resolved".to_owned(), None)
        },
        AccountTarget::Exact(coordinate) => (
            display_identifier(coordinate.provider.as_str()),
            terminal_safe(coordinate.account.as_str()),
            Some(terminal_safe(coordinate.account.as_str())),
        ),
    };
    push_account_heading(
        &mut output,
        &heading,
        heading_account.as_deref(),
        style,
        width,
    );
    if heading_account.is_none() {
        push_field(&mut output, "Account", &account, style, None, width);
    }
    push_field(&mut output, "Plan", "Unknown", style, None, width);
    push_field(
        &mut output,
        "Status",
        "Not refreshed",
        style,
        Some(TextStyle::Muted),
        width,
    );
    push_field(
        &mut output,
        "Updated",
        "Never",
        style,
        Some(TextStyle::Muted),
        width,
    );
    output.push('\n');
    push_heading(&mut output, "Usage limits", style, width, TextStyle::Accent);
    if let Some(width) = width {
        push_wrapped_value(
            &mut output,
            "  ",
            "No cached capacity information.",
            width,
            style,
            TextStyle::Muted,
        );
    } else {
        style.push(
            &mut output,
            TextStyle::Muted,
            "  No cached capacity information.",
        );
        output.push('\n');
    }
    output
}

fn local_host_label(provider: &ProviderId) -> String {
    format!("Local {}", display_identifier(provider.as_str()))
}

#[cfg(test)]
fn render(snapshot: &AccountCapacitySnapshot, style: PresentationStyle) -> String {
    render_snapshot(snapshot, None, style, GlyphProfile::Rich, None)
}

fn render_report(
    report: &AccountCapacityReport,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    render_snapshot(
        report.snapshot(),
        report.observed_at(),
        style,
        glyph_profile,
        width,
    )
}

fn push_account_heading(
    output: &mut String,
    provider: &str,
    account: Option<&str>,
    style: PresentationStyle,
    width: Option<usize>,
) {
    if let Some(width) = width {
        let provider_overflows = line_width(provider) > width;
        let heading_overflows =
            account.is_some_and(|account| line_width(&format!("{provider} · {account}")) > width);
        if provider_overflows || heading_overflows {
            push_wrapped_value(output, "", provider, width, style, TextStyle::Bold);
            if let Some(account) = account {
                push_wrapped_value(output, "  ", account, width, style, TextStyle::Accent);
            }
            return;
        }
    }
    style.push(output, TextStyle::Bold, provider);
    if let Some(account) = account {
        output.push_str(" · ");
        style.push(output, TextStyle::Accent, account);
    }
    output.push('\n');
}

fn render_snapshot(
    snapshot: &AccountCapacitySnapshot,
    observed_at: Option<&str>,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    let provider = display_identifier(snapshot.provider().as_str());
    let account = terminal_safe(snapshot.account_label());
    push_account_heading(&mut output, &provider, Some(&account), style, width);
    push_field(
        &mut output,
        "Plan",
        &display_plan(snapshot),
        style,
        None,
        width,
    );
    let (status, status_tone) = display_status(snapshot);
    push_field(
        &mut output,
        "Status",
        &status,
        style,
        Some(status_tone),
        width,
    );
    if let Some(credits) = snapshot
        .buckets()
        .iter()
        .find_map(|bucket| bucket.credits())
    {
        push_field(
            &mut output,
            "Credits",
            &display_credits(credits),
            style,
            None,
            width,
        );
    }
    if let Some(observed_at) = observed_at {
        push_field(
            &mut output,
            "Updated",
            &display_observed_at_with_age(observed_at),
            style,
            None,
            width,
        );
    }

    output.push('\n');
    push_heading(&mut output, "Usage limits", style, width, TextStyle::Accent);
    if !snapshot.buckets().iter().any(|bucket| {
        bucket.primary().is_some() || bucket.secondary().is_some() || bucket.credits().is_some()
    }) {
        if let Some(width) = width {
            push_wrapped_value(
                &mut output,
                "  ",
                "No capacity information available.",
                width,
                style,
                TextStyle::Muted,
            );
        } else {
            style.push(
                &mut output,
                TextStyle::Muted,
                "  No capacity information available.",
            );
            output.push('\n');
        }
        return output;
    }

    for (index, bucket) in snapshot.buckets().iter().enumerate() {
        let nested = is_additional_bucket(snapshot, bucket, index);
        if nested {
            let heading = format!("{} limit:", additional_bucket_heading(bucket));
            if let Some(width) = width {
                push_wrapped_value(&mut output, "  ", &heading, width, style, TextStyle::Bold);
            } else {
                output.push_str("  ");
                style.push(&mut output, TextStyle::Bold, &heading);
                output.push('\n');
            }
        }
        render_bucket(&mut output, bucket, nested, style, glyph_profile, width);
    }
    output
}

fn display_observed_at(value: &str) -> String {
    value.parse::<jiff::Timestamp>().map_or_else(
        |_| terminal_safe(value),
        |timestamp| {
            timestamp
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M:%S %Z")
                .to_string()
        },
    )
}

fn push_field(
    output: &mut String,
    label: &str,
    value: &str,
    style: PresentationStyle,
    tone: Option<TextStyle>,
    width: Option<usize>,
) {
    let Some(width) = width else {
        output.push_str("  ");
        style.push(output, TextStyle::Muted, &format!("{label:<9}"));
        match tone {
            Some(tone) => style.push(output, tone, value),
            None => output.push_str(value),
        }
        output.push('\n');
        return;
    };

    let field_indent = "  ";
    let inline_indent_width = cell_width("  ").expect("account field indent must be terminal-safe");
    let inline_prefix_width = inline_indent_width + 9;
    if width > inline_prefix_width {
        let value_width = width - inline_prefix_width;
        let lines = wrap_terminal_text(value, value_width);
        for (index, line) in lines.iter().enumerate() {
            if index == 0 {
                output.push_str(field_indent);
                style.push(output, TextStyle::Muted, &format!("{label:<9}"));
            } else {
                output.push_str(&" ".repeat(inline_prefix_width));
            }
            push_styled_value(output, line, style, tone);
            output.push('\n');
        }
        return;
    }

    let label_indent_width = inline_indent_width.min(width.saturating_sub(1));
    let label_indent = " ".repeat(label_indent_width);
    let label_width = width.saturating_sub(label_indent_width).max(1);
    for line in wrap_terminal_text(label, label_width) {
        output.push_str(&label_indent);
        style.push(output, TextStyle::Muted, &line);
        output.push('\n');
    }
    let value_indent_width = inline_indent_width.min(width.saturating_sub(1));
    let value_indent = " ".repeat(value_indent_width);
    let value_width = width.saturating_sub(value_indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&value_indent);
        push_styled_value(output, &line, style, tone);
        output.push('\n');
    }
}

fn push_styled_value(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    tone: Option<TextStyle>,
) {
    match tone {
        Some(tone) => style.push(output, tone, value),
        None => output.push_str(value),
    }
}

fn push_heading(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    width: Option<usize>,
    tone: TextStyle,
) {
    if let Some(width) = width {
        push_wrapped_value(output, "", value, width, style, tone);
    } else {
        style.push(output, tone, value);
        output.push('\n');
    }
}

fn push_tail_plain_line(output: &mut String, value: &str, width: Option<usize>) {
    if let Some(width) = width {
        push_wrapped_plain(output, "", value, width);
    } else {
        output.push_str(value);
        output.push('\n');
    }
}

fn push_tail_styled_line(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    tone: TextStyle,
    width: Option<usize>,
) {
    if let Some(width) = width {
        push_wrapped_value(output, "", value, width, style, tone);
    } else {
        style.push(output, tone, value);
        output.push('\n');
    }
}

fn push_wrapped_value(
    output: &mut String,
    indent: &str,
    value: &str,
    width: usize,
    style: PresentationStyle,
    tone: TextStyle,
) {
    let indent_width = cell_width(indent)
        .expect("account value indent must be terminal-safe")
        .min(width.saturating_sub(1));
    let indent = " ".repeat(indent_width);
    let value_width = width.saturating_sub(indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&indent);
        style.push(output, tone, &line);
        output.push('\n');
    }
}

fn wrap_terminal_text(value: &str, width: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = cell_width(grapheme).unwrap_or(1);
        if grapheme_width > width {
            if !line.is_empty() {
                lines.push(line);
                line = String::new();
                line_width = 0;
            }
            lines.push("?".to_owned());
            continue;
        }
        if !line.is_empty() && line_width + grapheme_width > width {
            lines.push(line);
            line = String::new();
            line_width = 0;
        }
        line.push_str(grapheme);
        line_width += grapheme_width;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn render_bucket(
    output: &mut String,
    bucket: &AccountCapacityBucket,
    nested: bool,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) {
    let windows = [
        ("Primary", bucket.primary().copied()),
        ("Secondary", bucket.secondary().copied()),
    ]
    .into_iter()
    .filter_map(|(label, window)| window.map(|window| (label, window)))
    .collect::<Vec<_>>();
    let (branch_prefix, last_branch_prefix) = match glyph_profile {
        GlyphProfile::Rich => ("  ├─ ", "  └─ "),
        GlyphProfile::Ascii => ("  |-- ", "  `-- "),
        _ => ("  |-- ", "  `-- "),
    };

    for (index, (label, window)) in windows.iter().enumerate() {
        let prefix = if nested {
            if index + 1 == windows.len() {
                last_branch_prefix
            } else {
                branch_prefix
            }
        } else {
            "  "
        };
        render_window(output, prefix, label, *window, style, glyph_profile, width);
    }
}

fn render_window(
    output: &mut String,
    prefix: &str,
    fallback_label: &str,
    window: AccountCapacityWindow,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) {
    let label = window
        .window_duration_minutes()
        .map_or_else(|| format!("{fallback_label} limit:"), display_window_label);
    let remaining = window.remaining_percent_basis_points();
    let tone = capacity_tone(remaining);
    let reset = window
        .resets_at_unix_seconds()
        .map(|reset| format!("(resets {})", display_reset(reset)));
    let full_bar = remaining_bar(window.remaining_percent(), 20, glyph_profile);
    let percent = format!("{}% left", display_percent(remaining));
    let plain_line = format!(
        "{prefix}{label:<15} [{full_bar}] {percent}{}",
        reset
            .as_deref()
            .map_or(String::new(), |value| format!(" {value}"))
    );
    if width.is_none() || width.is_some_and(|width| line_width(&plain_line) <= width) {
        let _ = write!(output, "{prefix}{label:<15} [");
        style.push(output, tone, &full_bar);
        let _ = write!(output, "] {percent}");
        if let Some(reset) = reset {
            let _ = write!(output, " {reset}");
        }
        output.push('\n');
        return;
    }

    render_narrow_window(
        output,
        NarrowWindowOptions {
            prefix,
            label: &label,
            percent: &percent,
            reset: reset.as_deref(),
            remaining_percent: window.remaining_percent(),
            tone,
            style,
            glyph_profile,
            width: width.expect("narrow window rendering requires a bounded width"),
        },
    );
}

struct NarrowWindowOptions<'a> {
    prefix: &'a str,
    label: &'a str,
    percent: &'a str,
    reset: Option<&'a str>,
    remaining_percent: u8,
    tone: TextStyle,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: usize,
}

fn render_narrow_window(output: &mut String, options: NarrowWindowOptions<'_>) {
    let NarrowWindowOptions {
        prefix,
        label,
        percent,
        reset,
        remaining_percent,
        tone,
        style,
        glyph_profile,
        width,
    } = options;
    let label_prefix = format!("{prefix}{label} [");
    let suffix = format!("] {percent}");
    let bar_width = width
        .saturating_sub(line_width(&label_prefix) + line_width(&suffix))
        .min(20);
    if bar_width > 0 {
        output.push_str(&label_prefix);
        style_meter(
            output,
            style,
            tone,
            remaining_percent,
            bar_width,
            glyph_profile,
        );
        output.push_str(&suffix);
        output.push('\n');
    } else {
        push_wrapped_plain(output, "", &format!("{prefix}{label}"), width);
        let indent_width = line_width(prefix).min(width.saturating_sub(1));
        let indent = " ".repeat(indent_width);
        let value_suffix = format!("] {percent}");
        let bar_width = width
            .saturating_sub(indent_width + 1 + line_width(&value_suffix))
            .min(20);
        if bar_width > 0 {
            output.push_str(&indent);
            output.push('[');
            style_meter(
                output,
                style,
                tone,
                remaining_percent,
                bar_width,
                glyph_profile,
            );
            output.push_str(&value_suffix);
            output.push('\n');
        } else {
            output.push_str(&indent);
            style_meter(output, style, tone, remaining_percent, 1, glyph_profile);
            output.push('\n');
            push_wrapped_plain(output, &indent, percent, width);
        }
    }

    if let Some(reset) = reset {
        let indent_width = line_width(prefix).min(width.saturating_sub(1));
        push_wrapped_plain(
            output,
            &" ".repeat(indent_width),
            &format!("resets {reset}"),
            width,
        );
    }
}

fn style_meter(
    output: &mut String,
    style: PresentationStyle,
    tone: TextStyle,
    remaining_percent: u8,
    width: usize,
    glyph_profile: GlyphProfile,
) {
    let meter = remaining_bar(remaining_percent, width, glyph_profile);
    style.push(output, tone, &meter);
}

fn push_wrapped_plain(output: &mut String, indent: &str, value: &str, width: usize) {
    let indent_width = line_width(indent).min(width.saturating_sub(1));
    let indent = " ".repeat(indent_width);
    let value_width = width.saturating_sub(indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&indent);
        output.push_str(&line);
        output.push('\n');
    }
}

fn line_width(value: &str) -> usize {
    cell_width(value).unwrap_or_else(|_| value.chars().count())
}

fn capacity_tone(remaining_percent_basis_points: u16) -> TextStyle {
    if remaining_percent_basis_points >= 5_000 {
        TextStyle::Positive
    } else if remaining_percent_basis_points >= 2_000 {
        TextStyle::Warning
    } else {
        TextStyle::Danger
    }
}

fn display_percent(percent_basis_points: u16) -> String {
    yo_tui::meter::format_percent(percent_basis_points)
}

fn display_window_label(minutes: u64) -> String {
    const MINUTES_PER_DAY: u64 = 24 * 60;
    if minutes == 7 * MINUTES_PER_DAY {
        return "Weekly limit:".to_owned();
    }
    if minutes != 0 && minutes.is_multiple_of(60) {
        let hours = minutes / 60;
        return format!("{hours}h limit:");
    }
    format!("{minutes}m limit:")
}

fn display_reset(unix_seconds: i64) -> String {
    jiff::Timestamp::from_second(unix_seconds).map_or_else(
        |_| format!("unix:{unix_seconds}"),
        |timestamp| {
            timestamp
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%b %-d, %H:%M %Z")
                .to_string()
        },
    )
}

fn is_additional_bucket(
    snapshot: &AccountCapacitySnapshot,
    bucket: &AccountCapacityBucket,
    index: usize,
) -> bool {
    index != 0 && bucket.id() != Some(snapshot.provider().as_str())
}

fn additional_bucket_heading(bucket: &AccountCapacityBucket) -> String {
    if let Some(name) = bucket.name().filter(|name| !name.trim().is_empty()) {
        return terminal_safe(name);
    }
    "Additional".to_owned()
}

fn display_plan(snapshot: &AccountCapacitySnapshot) -> String {
    let mut plans = snapshot.buckets().iter().filter_map(|bucket| bucket.plan());
    let Some(first) = plans.next() else {
        return "Unknown".to_owned();
    };
    if plans.all(|plan| plan == first) {
        match first {
            "prolite" => "Pro Lite".to_owned(),
            "supergrok" => "SuperGrok".to_owned(),
            _ => display_identifier(first),
        }
    } else {
        "Multiple".to_owned()
    }
}

fn display_status(snapshot: &AccountCapacitySnapshot) -> (String, TextStyle) {
    if let Some(reason) = snapshot
        .buckets()
        .iter()
        .find_map(|bucket| bucket.limit_reason())
    {
        return (
            format!("Limited · {}", display_identifier(reason)),
            TextStyle::Danger,
        );
    }
    if snapshot.buckets().iter().any(|bucket| {
        bucket.plan().is_some()
            || bucket.primary().is_some()
            || bucket.secondary().is_some()
            || bucket.credits().is_some()
    }) {
        ("Available".to_owned(), TextStyle::Positive)
    } else {
        ("Unknown".to_owned(), TextStyle::Muted)
    }
}

fn display_credits(credits: &yo_core::AccountCredits) -> String {
    if credits.unlimited() {
        "Unlimited".to_owned()
    } else if !credits.has_credits() {
        "None".to_owned()
    } else {
        credits
            .balance()
            .map_or_else(|| "Available".to_owned(), terminal_safe)
    }
}

fn display_identifier(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let safe = terminal_safe(part);
            let mut characters = safe.chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use yo_core::{
        AccountCredits, AccountId, LocalConnectionOperationRepositories, LocalCredentialRepository,
        ProviderId,
    };

    use super::*;

    struct AccountCredentialFixture {
        root: PathBuf,
        credentials: LocalCredentialRepository,
        provider: ProviderId,
        account: AccountId,
    }

    impl AccountCredentialFixture {
        fn new(name: &str) -> Self {
            let fixture = Self::without_model(name);
            let add = fixture
                .credentials
                .prepare_set(&fixture.provider, &fixture.account)
                .unwrap();
            fixture
                .credentials
                .commit(&add, Some(&ApiCredential::new("sk-sp-model").unwrap()))
                .unwrap();
            fixture
        }

        fn without_model(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "yo-account-session-{}-{name}-{nonce}",
                std::process::id()
            ));
            let credentials = LocalCredentialRepository::new(root.join("credentials.yaml"));
            let provider = ProviderId::new("qwencloud").unwrap();
            let account = AccountId::new("default").unwrap();
            Self {
                root,
                credentials,
                provider,
                account,
            }
        }

        fn persist_session(&self, value: &str) {
            let mutation = self
                .credentials
                .prepare_set_account_session(&self.provider, &self.account)
                .unwrap();
            self.credentials
                .commit_account_session(&mutation, &ApiCredential::new(value).unwrap())
                .unwrap();
        }

        fn repositories(&self) -> LocalConnectionOperationRepositories {
            LocalConnectionOperationRepositories::in_directory(&self.root).unwrap()
        }
    }

    impl Drop for AccountCredentialFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn qwen_snapshot(provider: &ProviderId, account: &AccountId) -> AccountCapacitySnapshot {
        AccountCapacitySnapshot::new(
            provider.clone(),
            account.clone(),
            vec![AccountCapacityBucket::new(
                Some("qwencloud".to_owned()),
                None,
                Some("moderato".to_owned()),
                None,
                None,
                None,
                None,
            )],
        )
    }

    // QwenCloud login prompt는 누락과 만료를 같은 구조 안에서 구분하고 API key가 아니라
    // browser session만 저장·교체한다는 행동을 짧게 보여 줍니다.
    #[test]
    fn qwencloud_session_prompt_distinguishes_first_save_from_replacement() {
        let provider = ProviderId::new("qwencloud").unwrap();
        let account = AccountId::new("default").unwrap();

        let missing = qwencloud_account_session_prompt(&provider, &account, false)
            .render(NonZeroU16::new(80).unwrap());
        let expired = qwencloud_account_session_prompt(&provider, &account, true)
            .render(NonZeroU16::new(80).unwrap());

        assert!(missing.contains("+ Browser session\n  Not saved · save for future"));
        assert!(expired.contains("~ Browser session\n  Expired · replace the saved session"));
        assert!(missing.contains("stored locally"));
        assert!(missing.ends_with("Cookie (hidden): "));
        assert!(!missing.contains("credentials.yaml"));
    }

    // account query는 전체, Provider 전체, exact Provider:Account 범위를 구분합니다.
    #[test]
    fn parses_account_query_scopes() {
        assert_eq!(parse_query(None).unwrap(), AccountQuery::All);
        assert_eq!(
            parse_query(Some("codex")).unwrap(),
            AccountQuery::Provider(ProviderId::new("codex").unwrap())
        );
        assert_eq!(
            parse_query(Some("kimi:default")).unwrap(),
            AccountQuery::Exact(AccountCoordinate::new("kimi", "default").unwrap())
        );
        assert_eq!(
            parse_query(Some("qwencloud:default")).unwrap(),
            AccountQuery::Exact(AccountCoordinate::new("qwencloud", "default").unwrap())
        );
        for source in [
            "kimi:",
            "kimi:default:extra",
            "qwencloud:",
            "qwencloud:default:extra",
        ] {
            assert!(parse_query(Some(source)).is_err());
        }
    }

    // host cache의 email label을 입력해도 stable 내부 AccountId 좌표로 찾아야 합니다.
    #[test]
    fn resolves_a_host_account_by_its_email_label() {
        let root = std::env::temp_dir().join(format!(
            "yo-account-alias-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("0123456789abcdef").unwrap(),
            Vec::new(),
        ))
        .with_account_label("person@example.test")
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let query = parse_query(Some("codex:person@example.test")).unwrap();

        let targets = resolve_targets(&query, &connections, &[report], false).unwrap();

        assert_eq!(
            targets,
            vec![AccountTarget::Exact(
                AccountCoordinate::new("codex", "0123456789abcdef").unwrap()
            )]
        );
        let _ = fs::remove_dir_all(root);
    }

    // host refresh 결과의 인증 email이 exact 요청과 일치하면 새 좌표를 정상적으로 승인합니다.
    #[test]
    fn exact_host_refresh_accepts_the_requested_email_identity() {
        let query = parse_query(Some("grok:person@example.test")).unwrap();
        let target =
            AccountTarget::Exact(AccountCoordinate::new("grok", "fedcba9876543210").unwrap());
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("grok").unwrap(),
            AccountId::new("fedcba9876543210").unwrap(),
            Vec::new(),
        ))
        .with_account_label("person@example.test");

        assert_eq!(exact_refresh_matches(&query, &target, &report), Some(true));
    }

    // 저장되지 않은 Kimi 계정은 임의의 cache 행으로 취급하지 않고 선택을 거부합니다.
    #[test]
    fn does_not_select_an_unsupported_cached_account() {
        let root = std::env::temp_dir().join(format!(
            "yo-account-eligibility-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("retired").unwrap(),
            Vec::new(),
        ));
        let query = parse_query(Some("kimi:retired")).unwrap();

        let error = resolve_targets(&query, &connections, &[report], false).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("no stored account-capacity source")
        );
        let _ = fs::remove_dir_all(root);
    }

    // 이전 account- 접두어 cache도 읽을 수 있어 기존 사용자 cache를 잃지 않습니다.
    #[test]
    fn accepts_a_legacy_host_cache_key_after_the_prefix_removal() {
        let root = std::env::temp_dir().join(format!(
            "yo-account-legacy-key-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("account-0123456789abcdef").unwrap(),
            Vec::new(),
        ));
        let query = parse_query(Some("codex:0123456789abcdef")).unwrap();

        let targets = resolve_targets(&query, &connections, &[report], false).unwrap();

        assert_eq!(
            targets,
            vec![AccountTarget::Exact(
                AccountCoordinate::new("codex", "account-0123456789abcdef").unwrap()
            )]
        );
        let _ = fs::remove_dir_all(root);
    }

    // 상세 출력은 마지막 관측 시각과 한 번도 refresh하지 않은 host 상태를 함께 보여줍니다.
    #[test]
    fn renders_last_refresh_time_and_unobserved_accounts() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];

        let output = render_records(
            &records,
            &AccountQuery::All,
            false,
            true,
            &[],
            &[],
            PresentationStyle::Plain,
        );

        assert!(output.contains(&format!(
            "Updated  {}",
            display_observed_at("2026-09-03T01:02:03Z")
        )));
        assert!(output.contains("Local Grok"));
        assert!(output.contains("Account  Not resolved"));
        assert!(!output.contains("Current host"));
        assert!(output.contains("Status   Not refreshed"));
        assert!(output.contains("Updated  Never"));
    }

    // 여러 계정의 table 목록은 컬럼형 표에서 남은 한도, 관측 age, 다음 실행 명령을 제공합니다.
    #[test]
    fn account_table_includes_age_and_scope_commands() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(82, Some(10_080), None).unwrap()),
                None,
                None,
                None,
            )],
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];

        let output = render_records(
            &records,
            &AccountQuery::All,
            false,
            false,
            &[],
            &[],
            PresentationStyle::Plain,
        );

        assert!(output.contains("Account capacity · 2 accounts"));
        assert!(output.contains("Account capacity · 2 accounts\nPROVIDER"));
        assert!(!output.contains("Account capacity · 2 accounts\n\nPROVIDER"));
        assert!(output.contains("PROVIDER"));
        assert!(output.contains("ACCOUNT"));
        assert!(output.contains("PLAN"));
        assert!(output.contains("LIMITS"));
        assert!(output.contains("UPDATED"));
        assert!(output.contains("codex"));
        assert!(output.contains("default"));
        assert!(output.contains("Pro Lite"));
        assert!(output.contains("7d ▂ 18%"));
        assert!(output.contains("grok"));
        assert!(output.contains("Local host"));
        assert!(output.contains("Not refreshed"));
        assert!(output.contains("Never"));
        assert!(
            output.lines().any(|line| {
                line.contains("codex") && line.contains(" (") && line.contains("ago")
            })
        );
        assert!(output.contains("Detail: yo account --detail"));
        assert!(output.contains("Refresh: yo account --refresh"));

        let ansi = render_records(
            &records,
            &AccountQuery::All,
            false,
            false,
            &[],
            &[],
            PresentationStyle::Ansi,
        );
        assert!(ansi.contains("\u{1b}[31m7d ▂ 18%\u{1b}[0m"));
        assert!(ansi.contains("\u{1b}[2m ("));
        assert_eq!(crate::presentation::strip_ansi(&ansi), output);
    }

    // account text는 table level meter와 상세 horizontal bar 모두에 공용 ASCII profile을
    // 적용하며, Rich glyph로 조용히 되돌아가지 않습니다.
    #[test]
    fn account_text_uses_the_requested_ascii_meter_profile() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(82, Some(10_080), None).unwrap()),
                None,
                None,
                None,
            )],
        );
        let report = AccountCapacityReport::plain(snapshot);
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];

        let table = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::All,
                refreshed: false,
                detail: false,
                glyph_profile: GlyphProfile::Ascii,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Unbounded,
                style: PresentationStyle::Plain,
            },
        );
        assert!(table.contains("7d : 18%"));
        assert!(!table.contains('▂'));

        let detail = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::All,
                refreshed: false,
                detail: true,
                glyph_profile: GlyphProfile::Ascii,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Unbounded,
                style: PresentationStyle::Plain,
            },
        );
        assert!(detail.contains("[###-----------------]"));
        assert!(!detail.contains('█'));
    }

    // 여러 계정은 terminal 폭에 정확히 맞을 때 table에 남고, 한 칸이라도 부족하면 기존
    // 상세 block으로 전환합니다.
    #[test]
    fn account_table_switches_to_detail_when_the_terminal_is_narrow() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(82, Some(10_080), None).unwrap()),
                None,
                None,
                None,
            )],
        );
        let report = AccountCapacityReport::plain(snapshot);
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];
        let compact = render_table_records(
            &records,
            &AccountQuery::All,
            PresentationStyle::Plain,
            GlyphProfile::Rich,
        );
        let required_width = compact
            .lines()
            .map(|line| cell_width(line).unwrap())
            .max()
            .unwrap();
        let exact = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::All,
                refreshed: false,
                detail: false,
                glyph_profile: GlyphProfile::Rich,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Bounded(
                    NonZeroU16::new(required_width as u16).unwrap(),
                ),
                style: PresentationStyle::Plain,
            },
        );
        assert!(exact.contains("PROVIDER"));

        let narrow = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::All,
                refreshed: false,
                detail: false,
                glyph_profile: GlyphProfile::Rich,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Bounded(
                    NonZeroU16::new(required_width.saturating_sub(1) as u16).unwrap(),
                ),
                style: PresentationStyle::Plain,
            },
        );
        assert!(!narrow.contains("PROVIDER"));
        assert!(narrow.contains("Account capacity · 2 accounts\n\n"));
        assert!(narrow.contains("Codex · default"));
    }

    // detail은 긴 account label과 reset metadata를 자르지 않고 값을 감싸며, horizontal
    // meter만 줄여 bounded terminal 안에 유지합니다.
    #[test]
    fn account_detail_wraps_long_values_for_a_narrow_terminal() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("stable-account").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(82, Some(10_080), Some(1_900_000_000)).unwrap()),
                None,
                None,
                None,
            )],
        )
        .with_account_label("yon.long.account.label.for.narrow.terminals@example.com界");
        let report = AccountCapacityReport::plain(snapshot);
        let coordinate = report.coordinate();
        let records = [AccountCapacityRecord {
            target: AccountTarget::Exact(coordinate.clone()),
            report: Some(report),
        }];
        let output = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::Exact(coordinate.clone()),
                refreshed: false,
                detail: false,
                glyph_profile: GlyphProfile::Rich,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Bounded(NonZeroU16::new(32).unwrap()),
                style: PresentationStyle::Plain,
            },
        );

        assert!(
            output.contains("yon.long.account.label"),
            "output:\n{output}"
        );
        assert!(
            output.contains("terminals@example.com"),
            "output:\n{output}"
        );
        assert!(output.contains("resets"));
        assert!(output.lines().all(|line| cell_width(line).unwrap() <= 32));

        let very_narrow = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::Exact(coordinate),
                refreshed: false,
                detail: false,
                glyph_profile: GlyphProfile::Rich,
                warnings: &[],
                failures: &[],
                output_width: AccountOutputWidth::Bounded(NonZeroU16::new(1).unwrap()),
                style: PresentationStyle::Plain,
            },
        );
        assert!(
            very_narrow
                .lines()
                .all(|line| cell_width(line).unwrap() <= 1)
        );
        assert!(very_narrow.contains('?'));
    }

    // table 선택은 표 본문뿐 아니라 warning·실패·다음 명령 안내까지 terminal 폭 안에
    // 들어오는지 확인하므로, 긴 진단이 있으면 상세 block으로 전환합니다.
    #[test]
    fn account_table_considers_diagnostic_tail_width() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(82, Some(10_080), None).unwrap()),
                None,
                None,
                None,
            )],
        );
        let report = AccountCapacityReport::plain(snapshot);
        let records = [
            AccountCapacityRecord {
                target: AccountTarget::Exact(report.coordinate()),
                report: Some(report),
            },
            AccountCapacityRecord {
                target: AccountTarget::LocalHost {
                    provider: ProviderId::new("grok").unwrap(),
                },
                report: None,
            },
        ];
        let table = render_table_records(
            &records,
            &AccountQuery::All,
            PresentationStyle::Plain,
            GlyphProfile::Rich,
        );
        let width = table
            .lines()
            .map(|line| cell_width(line).unwrap())
            .max()
            .unwrap();
        let warnings = [CliDiagnostic::warning("a".repeat(width + 1))];

        let output = render_records_with_width(
            &records,
            AccountRenderOptions {
                query: &AccountQuery::All,
                refreshed: true,
                detail: false,
                glyph_profile: GlyphProfile::Rich,
                warnings: &warnings,
                failures: &[],
                output_width: AccountOutputWidth::Bounded(NonZeroU16::new(width as u16).unwrap()),
                style: PresentationStyle::Plain,
            },
        );

        assert!(!output.contains("\nPROVIDER"));
        assert!(output.contains("Refresh warnings"));
        assert!(
            output
                .lines()
                .all(|line| cell_width(line).unwrap() <= width)
        );
    }

    // 부분 refresh 실패도 retry 명령과 원래 오류를 함께 표시합니다.
    #[test]
    fn refresh_failures_include_a_retry_command_and_the_original_error() {
        let records = [AccountCapacityRecord {
            target: AccountTarget::LocalHost {
                provider: ProviderId::new("grok").unwrap(),
            },
            report: None,
        }];
        let failures = [AccountRefreshFailure {
            target: "Local Grok".to_owned(),
            message: "login required".to_owned(),
        }];

        let output = render_records(
            &records,
            &AccountQuery::All,
            true,
            false,
            &[],
            &failures,
            PresentationStyle::Plain,
        );

        assert!(output.contains("Retry: yo account --refresh"));
        assert!(output.contains("Refresh failures\n  Local Grok · login required"));
    }

    // Codex 호환성 warning은 data 뒤에 한 번 표시되고, 실패와 다음 명령 안내보다 앞선다.
    #[test]
    fn refresh_warnings_are_ordered_before_failures_and_commands() {
        let records = [AccountCapacityRecord {
            target: AccountTarget::LocalHost {
                provider: ProviderId::new("codex").unwrap(),
            },
            report: None,
        }];
        let warnings = [CliDiagnostic::warning(
            "Codex app-server compatibility warning",
        )];
        let failures = [AccountRefreshFailure {
            target: "Local Codex".to_owned(),
            message: "login required".to_owned(),
        }];

        let output = render_records(
            &records,
            &AccountQuery::All,
            true,
            false,
            &warnings,
            &failures,
            PresentationStyle::Plain,
        );

        let data = output.find("Account").unwrap();
        let warning = output.find("Refresh warnings").unwrap();
        let failure = output.find("Refresh failures").unwrap();
        let retry = output.find("Retry: yo account --refresh").unwrap();
        assert!(data < warning);
        assert!(warning < failure);
        assert!(failure < retry);
        assert_eq!(
            output
                .matches("Codex app-server compatibility warning")
                .count(),
            1
        );
    }

    // 상대 시간 표시는 짧은 단위부터 일 단위까지 결정적인 문구로 유지합니다.
    #[test]
    fn displays_compact_age_units() {
        assert_eq!(display_age(0), "just now");
        assert_eq!(display_age(120), "2m ago");
        assert_eq!(display_age(3_720), "1h 2m ago");
        assert_eq!(display_age(90_000), "1d 1h ago");
    }

    // 저장된 account_session이 없으면 no-echo 입력에서 받은 exact Cookie를 먼저 같은 account
    // record에 게시하고 그 값으로 한 번만 refresh하며 model API key는 보존합니다.
    #[test]
    fn missing_qwencloud_session_is_captured_persisted_and_refreshed_once() {
        let fixture = AccountCredentialFixture::new("missing");
        let mut captures = Vec::new();
        let mut refreshes = 0;
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();

        let snapshot = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |expired| {
                captures.push(expired);
                ApiCredential::new("cna=x; login_qwencloud_ticket=fresh")
                    .map_err(|error| AppError::single("fixture session", error))
            },
            |session| {
                refreshes += 1;
                assert_eq!(
                    session.expose_secret(),
                    "cna=x; login_qwencloud_ticket=fresh"
                );
                Ok(qwen_snapshot(&fixture.provider, &fixture.account))
            },
        )
        .unwrap();

        assert_eq!(snapshot.provider(), &fixture.provider);
        assert_eq!(captures, [false]);
        assert_eq!(refreshes, 1);
        let stored = fixture.credentials.capture().unwrap();
        assert_eq!(
            stored
                .resolve(&fixture.provider, &fixture.account)
                .unwrap()
                .expose_secret(),
            "sk-sp-model"
        );
        assert_eq!(
            stored
                .resolve_account_session(&fixture.provider, &fixture.account)
                .unwrap()
                .expose_secret(),
            "cna=x; login_qwencloud_ticket=fresh"
        );
    }

    // Provider가 stored session의 만료를 명시하면 replacement를 한 번만 입력·저장하고 두 번째
    // refresh 결과를 반환하며 같은 호출에서 세 번째 request나 prompt를 만들지 않습니다.
    #[test]
    fn expired_qwencloud_session_is_replaced_and_retried_exactly_once() {
        let fixture = AccountCredentialFixture::new("expired");
        fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
        let mut captures = Vec::new();
        let mut refreshes = 0;
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();

        let snapshot = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |expired| {
                captures.push(expired);
                ApiCredential::new("cna=x; login_qwencloud_ticket=replacement")
                    .map_err(|error| AppError::single("fixture replacement", error))
            },
            |session| {
                refreshes += 1;
                if session.expose_secret().ends_with("=expired") {
                    Err(qwencloud::expired_session_error())
                } else {
                    Ok(qwen_snapshot(&fixture.provider, &fixture.account))
                }
            },
        )
        .unwrap();

        assert_eq!(snapshot.account(), &fixture.account);
        assert_eq!(captures, [true]);
        assert_eq!(refreshes, 2);
        assert_eq!(
            fixture
                .credentials
                .capture()
                .unwrap()
                .resolve_account_session(&fixture.provider, &fixture.account)
                .unwrap()
                .expose_secret(),
            "cna=x; login_qwencloud_ticket=replacement"
        );
    }

    // 새로 입력한 session도 Provider에게 만료로 거절되면 그 오류를 terminal로 반환하고
    // prompt 한 번과 전체 remote attempt 두 번의 경계를 넘지 않습니다.
    #[test]
    fn replacement_rejection_does_not_start_a_third_refresh() {
        let fixture = AccountCredentialFixture::new("replacement-rejected");
        fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
        let mut captures = 0;
        let mut refreshes = 0;
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();

        let error = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |_| {
                captures += 1;
                ApiCredential::new("cna=x; login_qwencloud_ticket=also-expired")
                    .map_err(|source| AppError::single("fixture replacement", source))
            },
            |_| {
                refreshes += 1;
                Err::<AccountCapacitySnapshot, _>(qwencloud::expired_session_error())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("console session expired"));
        assert_eq!(captures, 1);
        assert_eq!(refreshes, 2);
    }

    // API credential record가 없으면 operation recovery와 snapshot 확인 뒤 Cookie 입력이나
    // Provider refresh를 시작하지 않아, 저장할 수 없는 secret을 먼저 요구하지 않습니다.
    #[test]
    fn missing_model_credential_fails_before_cookie_capture() {
        let fixture = AccountCredentialFixture::without_model("missing-model-credential");
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();
        let mut captures = 0;
        let mut refreshes = 0;

        let error = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |_| {
                captures += 1;
                ApiCredential::new("cna=x; login_qwencloud_ticket=unused")
                    .map_err(|source| AppError::single("fixture session", source))
            },
            |_| {
                refreshes += 1;
                Ok(qwen_snapshot(&fixture.provider, &fixture.account))
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("has no API credential"));
        assert_eq!(captures, 0);
        assert_eq!(refreshes, 0);
    }

    // Cookie 입력 중 credentials.yaml을 별도 writer가 갱신하면 입력 전에 준비한 exact
    // revision CAS가 충돌하고, 새 API key를 보존한 채 account session을 게시하지 않습니다.
    #[test]
    fn concurrent_credential_change_conflicts_instead_of_replanning_after_capture() {
        let fixture = AccountCredentialFixture::new("capture-conflict");
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();

        let error = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |_| {
                let replacement = fixture
                    .credentials
                    .prepare_set(&fixture.provider, &fixture.account)
                    .unwrap();
                fixture
                    .credentials
                    .commit(
                        &replacement,
                        Some(&ApiCredential::new("sk-concurrent").unwrap()),
                    )
                    .unwrap();
                ApiCredential::new("cna=x; login_qwencloud_ticket=stale")
                    .map_err(|source| AppError::single("fixture session", source))
            },
            |_| -> Result<AccountCapacitySnapshot, _> {
                panic!("a conflicted account session must not reach the Provider")
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("conflict"));
        let current = fixture.credentials.capture().unwrap();
        assert_eq!(
            current
                .resolve(&fixture.provider, &fixture.account)
                .unwrap()
                .expose_secret(),
            "sk-concurrent"
        );
        assert!(
            current
                .resolve_account_session(&fixture.provider, &fixture.account)
                .is_none()
        );
    }

    // 만료 교체 prompt 중 더 최신 account session이 게시되면 stale replacement가 이를
    // 덮지 못하고 exact revision conflict로 끝나며 최신 session bytes를 그대로 보존합니다.
    #[test]
    fn concurrent_session_replacement_is_not_overwritten_after_expiry() {
        let fixture = AccountCredentialFixture::new("replacement-conflict");
        fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
        let repositories = fixture.repositories();
        let mut operation = repositories.acquire().unwrap();
        operation.recover_pending_operation().unwrap();
        let credentials = operation.capture_credentials().unwrap();

        let error = refresh_qwencloud_capacity_with(
            &mut operation,
            &fixture.credentials,
            credentials,
            &fixture.provider,
            &fixture.account,
            |_| {
                let replacement = fixture
                    .credentials
                    .prepare_set_account_session(&fixture.provider, &fixture.account)
                    .unwrap();
                fixture
                    .credentials
                    .commit_account_session(
                        &replacement,
                        &ApiCredential::new("cna=x; login_qwencloud_ticket=concurrent-replacement")
                            .unwrap(),
                    )
                    .unwrap();
                ApiCredential::new("cna=x; login_qwencloud_ticket=stale-replacement")
                    .map_err(|source| AppError::single("fixture replacement", source))
            },
            |_| Err::<AccountCapacitySnapshot, _>(qwencloud::expired_session_error()),
        )
        .unwrap_err();

        assert!(error.to_string().contains("conflict"));
        assert_eq!(
            fixture
                .credentials
                .capture()
                .unwrap()
                .resolve_account_session(&fixture.provider, &fixture.account)
                .unwrap()
                .expose_secret(),
            "cna=x; login_qwencloud_ticket=concurrent-replacement"
        );
    }

    // 공용 snapshot 출력은 Provider가 준 사용률과 reset을 그대로 표시하고 남은 비율만
    // 보수 산술로 더해, Session token 합계나 cache token을 계정 quota로 오인하지 않습니다.
    #[test]
    fn renders_capacity_separately_from_session_usage() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                Some("plus".to_owned()),
                Some(AccountCapacityWindow::new(37, Some(300), Some(1_800_000_000)).unwrap()),
                None,
                Some(AccountCredits::new(Some("12.5".to_owned()), true, false)),
                None,
            )],
        );

        let output = render(&snapshot, PresentationStyle::Plain);

        assert!(output.contains("Codex · default"));
        assert!(output.contains("Codex · default\n  Plan     Plus"));
        assert!(!output.contains("Codex · default\n\n  Plan"));
        assert!(output.contains("Plan     Plus"));
        assert!(output.contains("Status   Available"));
        assert!(output.contains("Credits  12.5"));
        assert!(output.contains("5h limit:"));
        assert!(output.contains("[████████████░░░░░░░░] 63% left"));
        assert!(output.contains("(resets "));
        assert!(output.contains("\n\nUsage limits\n"));
        assert!(!output.contains("token"));
    }

    // Provider가 소수 사용률을 보고하면 text도 보존된 0.01% 정밀도를 표시하되,
    // 정수 비율에는 불필요한 `.0`을 붙이지 않습니다.
    #[test]
    fn renders_fractional_capacity_without_noisy_zeroes() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("qwencloud".to_owned()),
                None,
                Some("standard".to_owned()),
                Some(
                    AccountCapacityWindow::from_used_percent_basis_points(163, Some(10_080), None)
                        .unwrap(),
                ),
                None,
                None,
                None,
            )],
        );

        let output = render(&snapshot, PresentationStyle::Plain);

        assert!(output.contains("98.37% left"));
        assert!(!output.contains("98.370"));
    }

    // Kimi `/me`의 공개 등급명은 일반 식별자 변환을 거쳐도 원문 의미를 유지해
    // 실제 계정 출력에서 Unknown 대신 Provider가 보고한 Moderato를 보여줍니다.
    #[test]
    fn renders_kimi_account_level_name_as_the_plan() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("kimi".to_owned()),
                None,
                Some("Moderato".to_owned()),
                Some(AccountCapacityWindow::new(0, Some(10_080), None).unwrap()),
                None,
                None,
                None,
            )],
        );

        let output = render(&snapshot, PresentationStyle::Plain);

        assert!(output.contains("Kimi · default"));
        assert!(output.contains("Plan     Moderato"));
    }

    // Grok 인증 응답은 subscription tier만 제공하므로 이를 플랜과 가용 상태로 표시하되
    // Provider가 보고하지 않은 잔여 한도 창은 만들지 않습니다.
    #[test]
    fn renders_grok_subscription_without_inventing_capacity_windows() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("grok").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("grok".to_owned()),
                None,
                Some("supergrok".to_owned()),
                None,
                None,
                None,
                None,
            )],
        );

        let output = render(&snapshot, PresentationStyle::Plain);

        assert!(output.contains("Grok · default"));
        assert!(output.contains("Plan     SuperGrok"));
        assert!(output.contains("Status   Available"));
        assert!(output.contains("No capacity information available."));
        assert!(!output.contains("% left"));
    }

    // Provider 내부 limit ID가 여러 개여도 일반 화면은 이를 제품명처럼 노출하지 않고,
    // 표준 한도와 용도가 확정되지 않은 추가 한도로만 구분합니다.
    #[test]
    fn hides_internal_bucket_ids_behind_neutral_sections() {
        let bucket = |id: &str, name: Option<&str>, duration| {
            AccountCapacityBucket::new(
                Some(id.to_owned()),
                name.map(str::to_owned),
                Some("prolite".to_owned()),
                Some(AccountCapacityWindow::new(0, Some(duration), None).unwrap()),
                name.map(|_| AccountCapacityWindow::new(0, Some(10_080), None).unwrap()),
                None,
                None,
            )
        };
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![
                bucket("codex", None, 10_080),
                bucket("codex_bengalfox", Some("GPT-5.3-Codex-Spark"), 300),
            ],
        );

        let output = render(&snapshot, PresentationStyle::Plain);

        assert!(output.contains("Plan     Pro Lite"));
        assert!(output.contains("  Weekly limit:"));
        assert!(output.contains("  GPT-5.3-Codex-Spark limit:\n  ├─ 5h limit:"));
        assert!(output.contains("\n  └─ Weekly limit:"));
        assert!(!output.contains("bengalfox"));
        assert!(!output.contains("bucket:"));

        let ascii_output = render_snapshot(
            &snapshot,
            None,
            PresentationStyle::Plain,
            GlyphProfile::Ascii,
            None,
        );
        assert!(ascii_output.contains("  GPT-5.3-Codex-Spark limit:\n  |-- 5h limit:"));
        assert!(ascii_output.contains("\n  `-- Weekly limit:"));
        assert!(!ascii_output.contains('├'));
        assert!(!ascii_output.contains('└'));
    }

    // TTY 출력만 제목과 상태를 꾸미고, 파이프나 파일로 보내는 기본 렌더링에는
    // 제어 문자를 넣지 않아 같은 정보를 안정적인 일반 텍스트로 유지합니다.
    #[test]
    fn ansi_decoration_is_explicit_and_plain_output_stays_clean() {
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("codex".to_owned()),
                None,
                None,
                Some(AccountCapacityWindow::new(90, None, None).unwrap()),
                None,
                None,
                None,
            )],
        );

        let plain = render(&snapshot, PresentationStyle::Plain);
        let ansi = render(&snapshot, PresentationStyle::Ansi);

        assert!(!plain.contains('\u{1b}'));
        assert!(ansi.contains("\u{1b}[1m"));
        assert!(ansi.contains("\u{1b}[31m"));
        assert_eq!(crate::presentation::strip_ansi(&ansi), plain);
    }
}
