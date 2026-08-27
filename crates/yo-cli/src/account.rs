use std::fmt::Write as _;

use yo_backend_delegated_codex::{CodexBackendConfig, read_account_capacity};
use yo_core::{AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow};

use super::{AppError, command::AccountCommand};

pub(crate) fn run(command: AccountCommand) -> Result<String, AppError> {
    if command.source != "codex" {
        return Err(AppError::message(format!(
            "unsupported account source `{}`; current support: codex",
            command.source
        )));
    }
    if !command.refresh {
        return Err(AppError::message(
            "account capacity currently requires an explicit --refresh",
        ));
    }
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let snapshot = read_account_capacity(CodexBackendConfig::new(cwd))
        .map_err(|error| AppError::single("refreshing Codex account capacity", error))?;
    Ok(render(&snapshot))
}

fn render(snapshot: &AccountCapacitySnapshot) -> String {
    let mut output = format!("{}:{}\n", snapshot.provider(), snapshot.account());
    if snapshot.buckets().is_empty() {
        output.push_str("capacity: unknown\n");
        return output;
    }
    let show_bucket = snapshot.buckets().len() > 1;
    for bucket in snapshot.buckets() {
        if show_bucket {
            let identity = bucket.id().or_else(|| bucket.name()).unwrap_or("unknown");
            let _ = writeln!(output, "bucket: {identity}");
        }
        render_bucket(&mut output, bucket, show_bucket);
    }
    output
}

fn render_bucket(output: &mut String, bucket: &AccountCapacityBucket, indent: bool) {
    let prefix = if indent { "  " } else { "" };
    let _ = writeln!(
        output,
        "{prefix}plan: {}",
        bucket.plan().unwrap_or("unknown")
    );
    if let Some(window) = bucket.primary() {
        render_window(output, prefix, "primary", *window);
    }
    if let Some(window) = bucket.secondary() {
        render_window(output, prefix, "secondary", *window);
    }
    if let Some(credits) = bucket.credits() {
        let value = if credits.unlimited() {
            "unlimited".to_owned()
        } else if let Some(balance) = credits.balance() {
            balance.to_owned()
        } else if credits.has_credits() {
            "available".to_owned()
        } else {
            "none".to_owned()
        };
        let _ = writeln!(output, "{prefix}credits: {value}");
    }
    if let Some(reason) = bucket.limit_reason() {
        let _ = writeln!(output, "{prefix}status: limited ({reason})");
    } else if bucket.primary().is_some()
        || bucket.secondary().is_some()
        || bucket.credits().is_some()
    {
        let _ = writeln!(output, "{prefix}status: available");
    } else {
        let _ = writeln!(output, "{prefix}status: unknown");
    }
}

fn render_window(output: &mut String, prefix: &str, label: &str, window: AccountCapacityWindow) {
    let _ = write!(
        output,
        "{prefix}{label}: {}% used, {}% remaining",
        window.used_percent(),
        window.remaining_percent()
    );
    if let Some(duration) = window.window_duration_minutes() {
        let _ = write!(output, ", window {duration}m");
    }
    if let Some(reset) = window.resets_at_unix_seconds() {
        let display = jiff::Timestamp::from_second(reset).map_or_else(
            |_| format!("unix:{reset}"),
            |timestamp| timestamp.to_string(),
        );
        let _ = write!(output, ", resets {display}");
    }
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use yo_core::{AccountCredits, AccountId, ProviderId};

    use super::*;

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

        let output = render(&snapshot);

        assert!(output.contains("codex:default"));
        assert!(output.contains("plan: plus"));
        assert!(output.contains("primary: 37% used, 63% remaining, window 300m"));
        assert!(output.contains("credits: 12.5"));
        assert!(!output.contains("token"));
    }
}
