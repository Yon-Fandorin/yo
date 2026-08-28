use std::fmt::Write as _;

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

use super::{
    AppError,
    command::{AccountCommand, OutputFormat},
    config,
    connection::{HiddenSecretAction, HiddenSecretPrompt, read_hidden_secret},
    presentation::{PresentationStyle, TextStyle, remaining_bar},
};

pub(crate) fn run(command: AccountCommand) -> Result<String, AppError> {
    if !command.refresh {
        return Err(AppError::message(
            "account capacity currently requires an explicit --refresh",
        ));
    }
    let report = match parse_source(&command.source)? {
        AccountSource::Codex => AccountCapacityReport::plain(read_codex_capacity()?),
        AccountSource::Grok => AccountCapacityReport::plain(read_grok_capacity()?),
        AccountSource::Kimi(account) => AccountCapacityReport::plain(read_kimi_capacity(account)?),
        AccountSource::QwenCloud(account) => read_qwencloud_capacity(account)?,
    };
    match command.format {
        OutputFormat::Text => Ok(render(&report.snapshot, PresentationStyle::for_stdout())),
        OutputFormat::Json => json::render(&report)
            .map_err(|error| AppError::single("serializing account capacity JSON", error)),
    }
}

struct AccountCapacityReport {
    snapshot: AccountCapacitySnapshot,
    provider_data: Option<AccountProviderData>,
}

impl AccountCapacityReport {
    fn plain(snapshot: AccountCapacitySnapshot) -> Self {
        Self {
            snapshot,
            provider_data: None,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum AccountProviderData {
    QwenCloud(qwencloud::QwenCloudProviderData),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccountSource<'a> {
    Codex,
    Grok,
    Kimi(&'a str),
    QwenCloud(&'a str),
}

fn parse_source(source: &str) -> Result<AccountSource<'_>, AppError> {
    if source == "codex" {
        return Ok(AccountSource::Codex);
    }
    if source == "grok" {
        return Ok(AccountSource::Grok);
    }
    if let Some(account) = source.strip_prefix("kimi:")
        && !account.is_empty()
        && !account.contains(':')
    {
        return Ok(AccountSource::Kimi(account));
    }
    if let Some(account) = source.strip_prefix("qwencloud:")
        && !account.is_empty()
        && !account.contains(':')
    {
        return Ok(AccountSource::QwenCloud(account));
    }
    Err(AppError::message(format!(
        "unsupported account source `{source}`; current support: codex, grok, kimi:<account>, qwencloud:<account>"
    )))
}

fn read_qwencloud_capacity(account: &str) -> Result<AccountCapacityReport, AppError> {
    let provider = ProviderId::new("qwencloud")
        .map_err(|error| AppError::single("resolving the QwenCloud Provider", error))?;
    let account = AccountId::new(account)
        .map_err(|error| AppError::single("resolving the QwenCloud Account", error))?;
    let config = config::load().map_err(|error| AppError::single("loading config", error))?;
    let state_directory = config
        .connection_path()
        .parent()
        .ok_or_else(|| AppError::message("Yo configuration has no state directory"))?
        .to_owned();
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

fn read_codex_capacity() -> Result<AccountCapacitySnapshot, AppError> {
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    read_codex_account_capacity(CodexBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Codex account capacity", error))
}

fn read_grok_capacity() -> Result<AccountCapacitySnapshot, AppError> {
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    read_grok_account_capacity(GrokBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Grok account capacity", error))
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

fn render(snapshot: &AccountCapacitySnapshot, style: PresentationStyle) -> String {
    let mut output = String::new();
    style.push(
        &mut output,
        TextStyle::Bold,
        &format!(
            "{} account",
            display_identifier(snapshot.provider().as_str())
        ),
    );
    output.push('\n');
    push_field(
        &mut output,
        "Account",
        &terminal_safe(snapshot.account().as_str()),
        style,
        None,
    );
    push_field(&mut output, "Plan", &display_plan(snapshot), style, None);
    let (status, status_tone) = display_status(snapshot);
    push_field(&mut output, "Status", &status, style, Some(status_tone));
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
        );
    }

    output.push('\n');
    style.push(&mut output, TextStyle::Bold, "Usage limits");
    output.push('\n');
    if !snapshot.buckets().iter().any(|bucket| {
        bucket.primary().is_some() || bucket.secondary().is_some() || bucket.credits().is_some()
    }) {
        style.push(
            &mut output,
            TextStyle::Muted,
            "  No capacity information available.",
        );
        output.push('\n');
        return output;
    }

    for (index, bucket) in snapshot.buckets().iter().enumerate() {
        let nested = is_additional_bucket(snapshot, bucket, index);
        if nested {
            let heading = additional_bucket_heading(bucket);
            output.push_str("  ");
            style.push(&mut output, TextStyle::Bold, &format!("{heading} limit:"));
            output.push('\n');
        }
        render_bucket(&mut output, bucket, nested, style);
    }
    output
}

fn push_field(
    output: &mut String,
    label: &str,
    value: &str,
    style: PresentationStyle,
    tone: Option<TextStyle>,
) {
    output.push_str("  ");
    style.push(output, TextStyle::Muted, &format!("{label:<9}"));
    match tone {
        Some(tone) => style.push(output, tone, value),
        None => output.push_str(value),
    }
    output.push('\n');
}

fn render_bucket(
    output: &mut String,
    bucket: &AccountCapacityBucket,
    nested: bool,
    style: PresentationStyle,
) {
    let windows = [
        ("Primary", bucket.primary().copied()),
        ("Secondary", bucket.secondary().copied()),
    ]
    .into_iter()
    .filter_map(|(label, window)| window.map(|window| (label, window)))
    .collect::<Vec<_>>();

    for (index, (label, window)) in windows.iter().enumerate() {
        let prefix = if nested {
            if index + 1 == windows.len() {
                "  └─ "
            } else {
                "  ├─ "
            }
        } else {
            "  "
        };
        render_window(output, prefix, label, *window, style);
    }
}

fn render_window(
    output: &mut String,
    prefix: &str,
    fallback_label: &str,
    window: AccountCapacityWindow,
    style: PresentationStyle,
) {
    let label = window
        .window_duration_minutes()
        .map_or_else(|| format!("{fallback_label} limit:"), display_window_label);
    let _ = write!(output, "{prefix}{label:<15} [");
    let remaining = window.remaining_percent_basis_points();
    let tone = if remaining >= 5_000 {
        TextStyle::Positive
    } else if remaining >= 2_000 {
        TextStyle::Warning
    } else {
        TextStyle::Danger
    };
    style.push(output, tone, &remaining_bar(window.remaining_percent(), 20));
    let _ = write!(output, "] {}% left", display_percent(remaining));
    if let Some(reset) = window.resets_at_unix_seconds() {
        let _ = write!(output, " (resets {})", display_reset(reset));
    }
    output.push('\n');
}

fn display_percent(percent_basis_points: u16) -> String {
    let whole = percent_basis_points / 100;
    let fractional = percent_basis_points % 100;
    if fractional == 0 {
        whole.to_string()
    } else if fractional.is_multiple_of(10) {
        format!("{whole}.{}", fractional / 10)
    } else {
        format!("{whole}.{fractional:02}")
    }
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
            .render(std::num::NonZeroU16::new(80).unwrap());
        let expired = qwencloud_account_session_prompt(&provider, &account, true)
            .render(std::num::NonZeroU16::new(80).unwrap());

        assert!(missing.contains("+ Browser session\n  Not saved · save for future"));
        assert!(expired.contains("~ Browser session\n  Expired · replace the saved session"));
        assert!(missing.contains("stored locally"));
        assert!(missing.ends_with("Cookie (hidden): "));
        assert!(!missing.contains("credentials.yaml"));
    }

    // Kimi source는 저장된 account를 명시하는 exact 두 segment 문법만 받아 Provider나
    // 기본 계정을 추측하지 않습니다.
    #[test]
    fn parses_exact_account_sources() {
        assert_eq!(parse_source("codex").unwrap(), AccountSource::Codex);
        assert_eq!(parse_source("grok").unwrap(), AccountSource::Grok);
        assert_eq!(
            parse_source("kimi:default").unwrap(),
            AccountSource::Kimi("default")
        );
        assert_eq!(
            parse_source("qwencloud:default").unwrap(),
            AccountSource::QwenCloud("default")
        );
        for source in [
            "kimi",
            "kimi:",
            "kimi:default:extra",
            "qwencloud",
            "qwencloud:",
            "qwencloud:default:extra",
            "grok:default",
        ] {
            assert!(parse_source(source).is_err());
        }
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

        assert!(output.contains("Codex account"));
        assert!(output.contains("Account  default"));
        assert!(output.contains("Plan     Plus"));
        assert!(output.contains("Status   Available"));
        assert!(output.contains("Credits  12.5"));
        assert!(output.contains("5h limit:"));
        assert!(output.contains("[████████████░░░░░░░░] 63% left"));
        assert!(output.contains("(resets "));
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

        assert!(output.contains("Kimi account"));
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

        assert!(output.contains("Grok account"));
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
