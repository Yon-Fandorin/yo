use std::fmt::Write as _;

use yo_backend_delegated_codex::{
    CodexBackendConfig, read_account_capacity as read_codex_account_capacity,
};
use yo_backend_delegated_grok::{
    GrokBackendConfig, read_account_capacity as read_grok_account_capacity,
};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId,
    KimiCatalogSeed, LocalConnectionRepository, LocalCredentialRepository, ProviderId,
    read_kimi_account_capacity,
};

mod json;

use super::{
    AppError,
    command::{AccountCommand, OutputFormat},
    config,
    presentation::{PresentationStyle, TextStyle, remaining_bar},
};

pub(crate) fn run(command: AccountCommand) -> Result<String, AppError> {
    if !command.refresh {
        return Err(AppError::message(
            "account capacity currently requires an explicit --refresh",
        ));
    }
    let snapshot = match parse_source(&command.source)? {
        AccountSource::Codex => read_codex_capacity()?,
        AccountSource::Grok => read_grok_capacity()?,
        AccountSource::Kimi(account) => read_kimi_capacity(account)?,
    };
    match command.format {
        OutputFormat::Text => Ok(render(&snapshot, PresentationStyle::for_stdout())),
        OutputFormat::Json => json::render(&snapshot)
            .map_err(|error| AppError::single("serializing account capacity JSON", error)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccountSource<'a> {
    Codex,
    Grok,
    Kimi(&'a str),
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
    Err(AppError::message(format!(
        "unsupported account source `{source}`; current support: codex, grok, kimi:<account>"
    )))
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
    let remaining = window.remaining_percent();
    let tone = if remaining >= 50 {
        TextStyle::Positive
    } else if remaining >= 20 {
        TextStyle::Warning
    } else {
        TextStyle::Danger
    };
    style.push(output, tone, &remaining_bar(remaining, 20));
    let _ = write!(output, "] {remaining:>3}% left");
    if let Some(reset) = window.resets_at_unix_seconds() {
        let _ = write!(output, " (resets {})", display_reset(reset));
    }
    output.push('\n');
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
    use yo_core::{AccountCredits, AccountId, ProviderId};

    use super::*;

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
        for source in [
            "kimi",
            "kimi:",
            "kimi:default:extra",
            "qwencloud",
            "grok:default",
        ] {
            assert!(parse_source(source).is_err());
        }
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
        assert!(output.contains("[████████████░░░░░░░░]  63% left"));
        assert!(output.contains("(resets "));
        assert!(!output.contains("token"));
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
                Some("SuperGrok".to_owned()),
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
