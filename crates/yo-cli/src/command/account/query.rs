use yo_core::ProviderId;

use super::domain::{
    AccountCapacityReport, AccountCoordinate, AccountQuery, AccountTarget,
    account_selector_matches, cached_report_is_selectable, host_command_available,
    is_stored_capacity_account, latest_host_cache,
};
use crate::AppError;

#[cfg(test)]
mod tests;

pub(super) fn parse_query(source: Option<&str>) -> Result<AccountQuery, AppError> {
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

pub(super) fn resolve_targets(
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
