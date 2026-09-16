use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
};

#[cfg(test)]
mod tests;

use super::super::current_repository;
use crate::{
    cost_report, review_continuation_preflight, review_delivery, review_delta, review_egress,
    review_packet, review_prepare, review_result, review_target_admission,
};

pub(super) fn run(
    scope: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    match scope {
        value if value == "review-packet" => run_review_packet(arguments),
        value if value == "review-prepare" => run_review_prepare(arguments),
        value if value == "review-delta" => run_review_delta(arguments),
        value if value == "review-deliver" => run_review_delivery(arguments),
        value if value == "review-continuation-preflight" => {
            run_review_continuation_preflight(arguments)
        },
        value if value == "review-result-correction-preflight" => {
            run_review_result_correction_preflight(arguments)
        },
        value if value == "cost-report" => run_cost_report(arguments),
        value if value == "review-egress" => run_review_egress(arguments),
        value if value == "review-target-admission" => run_review_target_admission(arguments),
        _ => Err(super::super::general_usage()),
    }
}

fn run_cost_report(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(cost_report_usage)?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(cost_report_usage)?;
    if arguments.next().is_some() {
        return Err(cost_report_usage());
    }
    let repository = current_repository()?;
    cost_report::run(&repository, &request, &output)
}

fn run_review_prepare(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_prepare_usage)?;
    if arguments.next().is_some() {
        return Err(review_prepare_usage());
    }
    let repository = current_repository()?;
    review_prepare::run(&repository, &request)
}

fn run_review_result_correction_preflight(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_result_correction_preflight_usage)?;
    if arguments.next().is_some() {
        return Err(review_result_correction_preflight_usage());
    }
    let repository = current_repository()?;
    review_result::correction_preflight(&repository, &request)
}

fn run_review_packet(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(review_packet_usage)?;
    let (mode, request) = match first.to_str() {
        Some("--check-readiness") => (
            ReviewPacketMode::CheckReadiness,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_packet_usage)?,
        ),
        Some("--preflight") => (
            ReviewPacketMode::Preflight,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_packet_usage)?,
        ),
        _ => (ReviewPacketMode::Publish, PathBuf::from(first)),
    };
    if arguments.next().is_some() {
        return Err(review_packet_usage());
    }
    let repository = current_repository()?;
    match mode {
        ReviewPacketMode::CheckReadiness => {
            review_packet::check_readiness(&repository, &request, &mut io::stdout().lock())
        },
        ReviewPacketMode::Preflight => {
            review_packet::preflight(&repository, &request, &mut io::stdout().lock())
        },
        ReviewPacketMode::Publish => review_packet::run(&repository, &request),
    }
}

enum ReviewPacketMode {
    CheckReadiness,
    Preflight,
    Publish,
}

fn run_review_delta(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_delta_usage)?;
    if arguments.next().is_some() {
        return Err(review_delta_usage());
    }
    let repository = current_repository()?;
    review_delta::run(&repository, &request)
}

fn run_review_egress(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_egress_usage)?;
    if arguments.next().is_some() {
        return Err(review_egress_usage());
    }
    let repository = current_repository()?;
    review_egress::run(&repository, &request)
}

fn run_review_delivery(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(review_delivery_usage)?;
    let (finalize, request) = if first == "finalize" {
        (
            true,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_delivery_usage)?,
        )
    } else {
        (false, PathBuf::from(first))
    };
    if arguments.next().is_some() {
        return Err(review_delivery_usage());
    }
    let repository = current_repository()?;
    if finalize {
        review_delivery::finalize(&repository, &request)
    } else {
        review_delivery::run(&repository, &request)
    }
}

fn run_review_target_admission(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_target_admission_usage)?;
    if arguments.next().is_some() {
        return Err(review_target_admission_usage());
    }
    review_target_admission::run(&request)
}

fn run_review_continuation_preflight(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_continuation_preflight_usage)?;
    if arguments.next().is_some() {
        return Err(review_continuation_preflight_usage());
    }
    let repository = current_repository()?;
    review_continuation_preflight::run(&repository, &request)
}

fn review_packet_usage() -> String {
    "usage: cargo xtask slice review-packet [--check-readiness|--preflight] <request.json>"
        .to_owned()
}

fn review_prepare_usage() -> String {
    "usage: cargo xtask slice review-prepare <request.json>".to_owned()
}

fn review_delta_usage() -> String {
    "usage: cargo xtask slice review-delta <request.json>".to_owned()
}

fn review_egress_usage() -> String {
    "usage: cargo xtask slice review-egress <request.json>".to_owned()
}

fn review_delivery_usage() -> String {
    "usage: cargo xtask slice review-deliver <request.json|finalize FINALIZE.json>".to_owned()
}

fn review_target_admission_usage() -> String {
    "usage: cargo xtask slice review-target-admission <request.json>".to_owned()
}

fn review_continuation_preflight_usage() -> String {
    "usage: cargo xtask slice review-continuation-preflight <request.json>".to_owned()
}

fn review_result_correction_preflight_usage() -> String {
    "usage: cargo xtask slice review-result-correction-preflight <request.json>".to_owned()
}

fn cost_report_usage() -> String {
    "usage: cargo xtask slice cost-report <request.json> <output.json>".to_owned()
}
