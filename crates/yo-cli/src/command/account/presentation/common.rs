use yo_core::{AccountCapacityBucket, AccountCapacitySnapshot};

use crate::{
    command::account::domain::{AccountQuery, display_identifier, terminal_safe},
    interaction::TextStyle,
};

pub(in crate::command::account) fn display_observed_at(value: &str) -> String {
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
pub(in crate::command::account) fn capacity_tone(remaining_percent_basis_points: u16) -> TextStyle {
    if remaining_percent_basis_points >= 5_000 {
        TextStyle::Positive
    } else if remaining_percent_basis_points >= 2_000 {
        TextStyle::Warning
    } else {
        TextStyle::Danger
    }
}
pub(in crate::command::account) fn is_additional_bucket(
    snapshot: &AccountCapacitySnapshot,
    bucket: &AccountCapacityBucket,
    index: usize,
) -> bool {
    index != 0 && bucket.id() != Some(snapshot.provider().as_str())
}

pub(in crate::command::account) fn additional_bucket_heading(
    bucket: &AccountCapacityBucket,
) -> String {
    if let Some(name) = bucket.name().filter(|name| !name.trim().is_empty()) {
        return terminal_safe(name);
    }
    "Additional".to_owned()
}

pub(in crate::command::account) fn display_plan(snapshot: &AccountCapacitySnapshot) -> String {
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

pub(in crate::command::account) fn display_status(
    snapshot: &AccountCapacitySnapshot,
) -> (String, TextStyle) {
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
pub(in crate::command::account) fn display_credits(credits: &yo_core::AccountCredits) -> String {
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
pub(in crate::command::account) fn account_scope_label(query: &AccountQuery) -> String {
    match query {
        AccountQuery::All => "Account capacity".to_owned(),
        AccountQuery::Provider(provider) => {
            format!("{} account capacity", display_identifier(provider.as_str()))
        },
        AccountQuery::Exact(target) => target.reference(),
    }
}

pub(in crate::command::account) fn account_command(query: &AccountQuery, flag: &str) -> String {
    let source = match query {
        AccountQuery::All => String::new(),
        AccountQuery::Provider(provider) => format!(" {provider}"),
        AccountQuery::Exact(target) => format!(" {}", shell_quote(&target.reference())),
    };
    format!("yo account{source} {flag}")
}

pub(in crate::command::account) fn shell_quote(value: &str) -> String {
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+')
    }) {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(in crate::command::account) fn display_observed_at_with_age(value: &str) -> String {
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

pub(in crate::command::account) fn display_age(seconds: u64) -> String {
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
