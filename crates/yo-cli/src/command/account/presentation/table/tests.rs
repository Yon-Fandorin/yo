use std::num::NonZeroU16;

use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId, ProviderId,
};
use yo_tui::{GlyphProfile, surface::cell_width};

use super::render_table_records;
use crate::{
    command::account::{
        domain::{
            AccountCapacityRecord, AccountCapacityReport, AccountQuery, AccountRefreshFailure,
            AccountTarget,
        },
        presentation::{
            AccountOutputWidth, AccountRenderOptions, common::display_age,
            render_records_with_width,
        },
    },
    interaction::{PresentationStyle, diagnostic::CliDiagnostic},
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
        output
            .lines()
            .any(|line| { line.contains("codex") && line.contains(" (") && line.contains("ago") })
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
    assert_eq!(crate::interaction::strip_ansi(&ansi), output);
}

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

// 상대 시간 표시는 짧은 단위부터 일 단위까지 결정적인 문구로 유지합니다.
#[test]
fn displays_compact_age_units() {
    assert_eq!(display_age(0), "just now");
    assert_eq!(display_age(120), "2m ago");
    assert_eq!(display_age(3_720), "1h 2m ago");
    assert_eq!(display_age(90_000), "1d 1h ago");
}
