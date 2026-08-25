#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 [--summary-out <path>] <run-name> -- <command> [args...]" >&2
}

summary_out=""
if [[ ${1:-} == "--summary-out" ]]; then
    if [[ $# -lt 2 || -z $2 ]]; then
        usage
        exit 64
    fi
    summary_out=$2
    shift 2
fi

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
readonly command_argv_count=$#

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
if [[ -n ${summary_out} ]]; then
    reject_control_bytes "summary output path" "${summary_out}"
    if [[ ${summary_out} != /* ]]; then
        invocation_directory=$(pwd -P) || {
            echo "bounded validation: cannot resolve invocation directory" >&2
            exit 64
        }
        summary_out="${invocation_directory}/${summary_out}"
    fi
    reject_control_bytes "resolved summary output path" "${summary_out}"
    readonly summary_out
    summary_parent=${summary_out%/*}
    if [[ -z ${summary_parent} ]]; then
        summary_parent=/
    fi
    if [[ ! -d ${summary_parent} ]]; then
        echo "bounded validation: summary output parent must already exist: ${summary_parent}" >&2
        exit 73
    fi
    if [[ -e ${summary_out} || -L ${summary_out} ]]; then
        echo "bounded validation: summary output already exists: ${summary_out}" >&2
        exit 73
    fi
fi

head_commit=$(git -C "${repository}" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) || {
    echo "bounded validation: cannot resolve worktree HEAD" >&2
    exit 64
}
readonly head_commit
worktree_status=$(git -C "${repository}" status --porcelain=v1 --untracked-files=normal) || {
    echo "bounded validation: cannot inspect worktree state" >&2
    exit 64
}
readonly worktree_status
if [[ -n "${worktree_status}" ]]; then
    readonly worktree_state=dirty
else
    readonly worktree_state=clean
fi

umask 077
mkdir -p "${log_root}"
log_path=$(mktemp "${log_root}/${run_name}.log.XXXXXX")
readonly log_path
reject_control_bytes "log path" "${log_path}"

if command -v sha256sum >/dev/null 2>&1; then
    readonly sha256_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    readonly sha256_tool=shasum
elif command -v openssl >/dev/null 2>&1; then
    readonly sha256_tool=openssl
else
    echo "bounded validation: SHA-256 tool not found (expected sha256sum, shasum, or openssl)" >&2
    exit 69
fi

sha256_file() {
    local path=$1
    case "${sha256_tool}" in
        sha256sum)
            sha256sum "${path}" | awk '{print $1}'
            ;;
        shasum)
            shasum -a 256 "${path}" | awk '{print $1}'
            ;;
        openssl)
            openssl dgst -sha256 "${path}" | awk '{print $NF}'
            ;;
    esac
}

argv_frame=$(mktemp "${log_root}/${run_name}.argv.XXXXXX")
readonly argv_frame
trap 'rm -f -- "${argv_frame}"' EXIT
printf 'yo.validation-run-argv/v1alpha1\0' >"${argv_frame}"
for argument in "$@"; do
    argument_bytes=$(printf '%s' "${argument}" | wc -c)
    argument_bytes=${argument_bytes//[[:space:]]/}
    printf '%s:' "${argument_bytes}" >>"${argv_frame}"
    printf '%s\0' "${argument}" >>"${argv_frame}"
done
command_argv_digest=$(sha256_file "${argv_frame}") || {
    echo "bounded validation: cannot hash command arguments" >&2
    exit 69
}
if [[ ! ${command_argv_digest} =~ ^[0-9a-f]{64}$ ]]; then
    echo "bounded validation: SHA-256 tool returned a non-canonical argument digest" >&2
    exit 69
fi
readonly command_argv_hash="sha256:${command_argv_digest}"
rm -f -- "${argv_frame}"
trap - EXIT

started_at=$(date +%s)
set +e
"$@" >"${log_path}" 2>&1
command_status=$?
set -e
finished_at=$(date +%s)

readonly elapsed_seconds=$((finished_at - started_at))
log_bytes=$(wc -c <"${log_path}")
readonly log_bytes="${log_bytes//[[:space:]]/}"
log_digest=$(sha256_file "${log_path}") || {
    echo "bounded validation: cannot hash complete log; complete log: ${log_path}" >&2
    exit 69
}
if [[ ! ${log_digest} =~ ^[0-9a-f]{64}$ ]]; then
    echo "bounded validation: SHA-256 tool returned a non-canonical log digest; complete log: ${log_path}" >&2
    exit 69
fi
readonly log_hash="sha256:${log_digest}"

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

printf -v summary_line '{"schema":"yo.validation-run-summary/v1alpha1","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s","log_hash":"%s","head_commit":"%s","worktree_state":"%s","command_argv_count":%d,"command_argv_hash":"%s","reused":false}' \
    "$(json_escape "${run_name}")" \
    "${result}" \
    "${command_status}" \
    "${elapsed_seconds}" \
    "${log_bytes}" \
    "$(json_escape "${report_path}")" \
    "${log_hash}" \
    "${head_commit}" \
    "${worktree_state}" \
    "${command_argv_count}" \
    "${command_argv_hash}"
readonly summary_line

if [[ -n ${summary_out} ]]; then
    summary_temp=$(mktemp "${summary_parent}/.yo-validation-summary.XXXXXX") || {
        echo "bounded validation: cannot prepare summary output: ${summary_out}" >&2
        exit 73
    }
    readonly summary_temp
    trap 'rm -f -- "${summary_temp}"' EXIT
    if ! printf '%s\n' "${summary_line}" >"${summary_temp}"; then
        echo "bounded validation: cannot write summary output: ${summary_out}" >&2
        exit 73
    fi
    if ! ln "${summary_temp}" "${summary_out}" 2>/dev/null; then
        echo "bounded validation: cannot atomically create summary output: ${summary_out}" >&2
        exit 73
    fi
    if [[ ! -f ${summary_out} || ! ${summary_temp} -ef ${summary_out} ]]; then
        nested_summary="${summary_out}/${summary_temp##*/}"
        if [[ -f ${nested_summary} && ${summary_temp} -ef ${nested_summary} ]]; then
            if ! rm -f -- "${nested_summary}"; then
                echo "bounded validation: cannot clean misplaced summary link: ${nested_summary}" >&2
                exit 73
            fi
        fi
        echo "bounded validation: cannot atomically create summary output: ${summary_out}" >&2
        exit 73
    fi
    if ! rm -f -- "${summary_temp}"; then
        echo "bounded validation: summary published but temporary link cleanup failed: ${summary_temp}" >&2
        exit 73
    fi
    trap - EXIT
fi

printf '%s\n' "${summary_line}"

if [[ ${command_status} -ne 0 ]]; then
    printf '%s\n' \
        "bounded validation: final ${failure_tail_bytes} log bytes follow; complete log: ${report_path}" >&2
    tail -c "${failure_tail_bytes}" "${log_path}" >&2
    printf '\n%s\n' "bounded validation: end of failure excerpt" >&2
fi

exit "${command_status}"
