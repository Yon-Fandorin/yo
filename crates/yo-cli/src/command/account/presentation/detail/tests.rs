use std::num::NonZeroU16;

use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, ProviderId,
};
use yo_tui::{GlyphProfile, surface::cell_width};

use super::{render, render_snapshot};
use crate::{
    command::account::{
        domain::{
            AccountCapacityRecord, AccountCapacityReport, AccountQuery, AccountRefreshFailure,
            AccountTarget,
        },
        presentation::{
            AccountOutputWidth, AccountRenderOptions, common::display_observed_at,
            render_records_with_width,
        },
    },
    diagnostic::CliDiagnostic,
    presentation::PresentationStyle,
};

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

// 상세 출력은 마지막 관측 시각과 한 번도 refresh하지 않은 host 상태를 함께 보여줍니다.
#[test]
fn renders_last_refresh_time_and_unobserved_accounts() {
    let snapshot = AccountCapacitySnapshot::new(
        ProviderId::new("codex").unwrap(),
        AccountId::new("default").unwrap(),
        Vec::new(),
    );
    let report =
        AccountCapacityReport::plain(snapshot).with_observed_at("2026-09-03T01:02:03Z".to_owned());
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
