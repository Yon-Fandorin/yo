#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 <run-name> -- <command> [args...]" >&2
}

if [[ $# -lt 3 ]]; then
    usage
    exit 64
fi

readonly run_name=$1
shift

if [[ ! "${run_name}" =~ ^[a-z0-9][a-z0-9._-]*$ || ${#run_name} -gt 64 ]]; then
    echo "bounded validation: run name must match [a-z0-9][a-z0-9._-]* and be at most 64 bytes" >&2
    exit 64
fi
if [[ $1 != "--" ]]; then
    usage
    exit 64
fi
shift
if [[ $# -eq 0 ]]; then
    usage
    exit 64
fi

repository=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "bounded validation: run from a Git worktree" >&2
    exit 64
}
readonly repository
readonly log_root="${YO_BOUNDED_VALIDATION_LOG_ROOT:-${repository}/.local-exclude/validation-runs}"
readonly failure_tail_bytes=16384

reject_control_bytes() {
    local label=$1
    local value=$2
    local control_pattern=$'[\001-\037]'
    if [[ ${value} == *${control_pattern}* ]]; then
        echo "bounded validation: ${label} contains unsupported control bytes" >&2
        exit 64
    fi
}

reject_control_bytes "repository path" "${repository}"
reject_control_bytes "log root" "${log_root}"

umask 077
mkdir -p "${log_root}"
log_path=$(mktemp "${log_root}/${run_name}.log.XXXXXX")
readonly log_path
reject_control_bytes "log path" "${log_path}"

started_at=$(date +%s)
set +e
"$@" >"${log_path}" 2>&1
command_status=$?
set -e
finished_at=$(date +%s)

readonly elapsed_seconds=$((finished_at - started_at))
log_bytes=$(wc -c <"${log_path}")
readonly log_bytes="${log_bytes//[[:space:]]/}"

if [[ "${log_path}" == "${repository}/"* ]]; then
    report_path=${log_path#"${repository}/"}
else
    report_path=${log_path}
fi

json_escape() {
    local value=$1
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//$'\n'/\\n}
    value=${value//$'\r'/\\r}
    value=${value//$'\t'/\\t}
    printf '%s' "${value}"
}

if [[ ${command_status} -eq 0 ]]; then
    result=passed
else
    result=failed
fi

printf '{"schema":"yo.validation-run-summary/v1","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s"}\n' \
    "$(json_escape "${run_name}")" \
    "${result}" \
    "${command_status}" \
    "${elapsed_seconds}" \
    "${log_bytes}" \
    "$(json_escape "${report_path}")"

if [[ ${command_status} -ne 0 ]]; then
    printf '%s\n' \
        "bounded validation: final ${failure_tail_bytes} log bytes follow; complete log: ${report_path}" >&2
    tail -c "${failure_tail_bytes}" "${log_path}" >&2
    printf '\n%s\n' "bounded validation: end of failure excerpt" >&2
fi

exit "${command_status}"
