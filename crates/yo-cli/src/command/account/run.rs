use std::collections::BTreeMap;

use yo_core::LocalConnectionRepository;

use super::{
    domain::{
        AccountCapacityRecord, AccountRefreshFailure, exact_refresh_matches,
        target_display_reference,
    },
    presentation::{AccountRenderOptions, account_output_width, json, render_records_with_width},
    query::{parse_query, resolve_targets},
    refresh::{query_exact_account, refresh_target},
    storage,
};
use crate::{
    AppError,
    command::{OutputFormat, account::Command as AccountCommand},
    config,
    diagnostic::CliDiagnostic,
    presentation::PresentationStyle,
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
