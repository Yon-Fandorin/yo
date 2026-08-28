#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 [--summary-out <path>] [--reusable-local] [--resource-class <cargo-heavy|independent>] <run-name> -- <command> [args...]" >&2
}

summary_out=""
reusable_local=false
resource_class=""
while [[ ${1:-} == --* ]]; do
    case ${1:-} in
        --summary-out)
            if [[ $# -lt 2 || -z $2 ]]; then
                usage
                exit 64
            fi
            summary_out=$2
            shift 2
            ;;
        --reusable-local)
            reusable_local=true
            shift
            ;;
        --resource-class)
            if [[ $# -lt 2 || ! ${2:-} =~ ^(cargo-heavy|independent)$ ]]; then
                usage
                exit 64
            fi
            resource_class=$2
            shift 2
            ;;
        *)
            usage
            exit 64
            ;;
    esac
done

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

if [[ -z ${resource_class} ]]; then
    if [[ ${1##*/} == cargo ]]; then
        resource_class=cargo-heavy
    else
        resource_class=unleased
    fi
fi
if [[ ${resource_class} == independent ]]; then
    if [[ -z ${CARGO_TARGET_DIR:-} || ${CARGO_TARGET_DIR} != /* ]]; then
        echo "bounded validation: independent resource class requires an absolute CARGO_TARGET_DIR" >&2
        exit 64
    fi
fi
readonly resource_class

repository=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "bounded validation: run from a Git worktree" >&2
    exit 64
}
readonly repository
git_common_directory=$(git -C "${repository}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || {
    echo "bounded validation: cannot resolve shared Git directory" >&2
    exit 64
}
readonly git_common_directory
readonly workspace_root=${git_common_directory%/.git}
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
reject_control_bytes "shared Git directory" "${git_common_directory}"
if [[ ${resource_class} == independent ]]; then
    reject_control_bytes "CARGO_TARGET_DIR" "${CARGO_TARGET_DIR}"
fi
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

if [[ ${reusable_local} == true ]]; then
    platform_os=$(uname -s | tr '[:upper:]' '[:lower:]') || {
        echo "bounded validation: cannot identify the platform" >&2
        exit 69
    }
    case ${platform_os} in
        darwin) platform_os=macos ;;
    esac
    readonly platform_os
    platform_arch=$(uname -m) || {
        echo "bounded validation: cannot identify the architecture" >&2
        exit 69
    }
    case ${platform_arch} in
        arm64) platform_arch=aarch64 ;;
        amd64) platform_arch=x86_64 ;;
    esac
    readonly platform_arch

    toolchain_frame=$(mktemp "${log_root}/${run_name}.toolchain.XXXXXX")
    readonly toolchain_frame
    trap 'rm -f -- "${toolchain_frame}"' EXIT
    printf 'yo.validation-toolchain/v1alpha1\0' >"${toolchain_frame}"
    for tool in rustc cargo; do
        tool_output=$("${tool}" -Vv 2>/dev/null) || {
            echo "bounded validation: cannot fingerprint ${tool} -Vv" >&2
            exit 69
        }
        tool_bytes=$(printf '%s' "${tool_output}" | wc -c)
        tool_bytes=${tool_bytes//[[:space:]]/}
        printf '%s:' "${tool_bytes}" >>"${toolchain_frame}"
        printf '%s\0' "${tool_output}" >>"${toolchain_frame}"
    done
    toolchain_digest=$(sha256_file "${toolchain_frame}") || {
        echo "bounded validation: cannot hash the toolchain context" >&2
        exit 69
    }
    if [[ ! ${toolchain_digest} =~ ^[0-9a-f]{64}$ ]]; then
        echo "bounded validation: SHA-256 tool returned a non-canonical toolchain digest" >&2
        exit 69
    fi
    readonly toolchain_hash="sha256:${toolchain_digest}"
    rm -f -- "${toolchain_frame}"
    trap - EXIT
fi

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

lease_path=""
lease_key="none"
summary_temp=""
cleanup() {
    if [[ -n ${summary_temp} ]]; then
        rm -f -- "${summary_temp}"
    fi
    if [[ -n ${lease_path} ]]; then
        rm -f -- "${lease_path}/owner"
        rmdir -- "${lease_path}" 2>/dev/null || true
    fi
}
if [[ ${resource_class} != unleased ]]; then
    lease_root="${workspace_root}/.local-exclude/validation-leases"
    mkdir -p -- "${lease_root}"
    if [[ ${resource_class} == cargo-heavy ]]; then
        lease_key=cargo-heavy
    else
        resource_frame=$(mktemp "${log_root}/${run_name}.resource.XXXXXX")
        printf 'yo.validation-resource/v1alpha1\0%s\0' "${CARGO_TARGET_DIR}" >"${resource_frame}"
        resource_digest=$(sha256_file "${resource_frame}") || {
            rm -f -- "${resource_frame}"
            echo "bounded validation: cannot identify independent validation resource" >&2
            exit 69
        }
        rm -f -- "${resource_frame}"
        lease_key="independent-${resource_digest}"
    fi
    lease_path="${lease_root}/${lease_key}"
    if ! mkdir -- "${lease_path}" 2>/dev/null; then
        echo "bounded validation: resource lease is already held: ${lease_key}" >&2
        exit 75
    fi
    trap cleanup EXIT
    printf 'pid=%s\nrun=%s\nrepository=%s\n' "$$" "${run_name}" "${repository}" >"${lease_path}/owner"
fi
readonly lease_key
trap cleanup EXIT

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

if [[ ${resource_class} != unleased && ${reusable_local} == true ]]; then
    printf -v summary_line '{"schema":"yo.validation-run-summary/v1alpha4","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s","log_hash":"%s","head_commit":"%s","worktree_state":"%s","command_argv_count":%d,"command_argv_hash":"%s","reused":false,"reuse_policy":"reviewed-descendant-context/v1","reuse_context":{"schema":"yo.validation-reuse-context/v1alpha1","platform_os":"%s","platform_arch":"%s","toolchain_hash":"%s","external_state":"none-declared"},"resource_lease":{"schema":"yo.validation-resource-lease/v1alpha1","class":"%s","key":"%s","status":"acquired","wait_attempts":0}}' \
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
        "${command_argv_hash}" \
        "${platform_os}" \
        "${platform_arch}" \
        "${toolchain_hash}" \
        "${resource_class}" \
        "${lease_key}"
elif [[ ${resource_class} != unleased ]]; then
    printf -v summary_line '{"schema":"yo.validation-run-summary/v1alpha4","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s","log_hash":"%s","head_commit":"%s","worktree_state":"%s","command_argv_count":%d,"command_argv_hash":"%s","reused":false,"reuse_policy":"reviewed-descendant/v1","resource_lease":{"schema":"yo.validation-resource-lease/v1alpha1","class":"%s","key":"%s","status":"acquired","wait_attempts":0}}' \
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
        "${command_argv_hash}" \
        "${resource_class}" \
        "${lease_key}"
elif [[ ${reusable_local} == true ]]; then
    printf -v summary_line '{"schema":"yo.validation-run-summary/v1alpha3","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s","log_hash":"%s","head_commit":"%s","worktree_state":"%s","command_argv_count":%d,"command_argv_hash":"%s","reused":false,"reuse_policy":"reviewed-descendant-context/v1","reuse_context":{"schema":"yo.validation-reuse-context/v1alpha1","platform_os":"%s","platform_arch":"%s","toolchain_hash":"%s","external_state":"none-declared"}}' \
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
        "${command_argv_hash}" \
        "${platform_os}" \
        "${platform_arch}" \
        "${toolchain_hash}"
else
    printf -v summary_line '{"schema":"yo.validation-run-summary/v1alpha2","name":"%s","status":"%s","exit_code":%d,"elapsed_seconds":%d,"log_bytes":%d,"log_path":"%s","log_hash":"%s","head_commit":"%s","worktree_state":"%s","command_argv_count":%d,"command_argv_hash":"%s","reused":false,"reuse_policy":"reviewed-descendant/v1"}' \
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
fi
readonly summary_line

if [[ -n ${summary_out} ]]; then
    summary_temp=$(mktemp "${summary_parent}/.yo-validation-summary.XXXXXX") || {
        echo "bounded validation: cannot prepare summary output: ${summary_out}" >&2
        exit 73
    }
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
    summary_temp=""
fi

printf '%s\n' "${summary_line}"

if [[ ${command_status} -ne 0 ]]; then
    printf '%s\n' \
        "bounded validation: final ${failure_tail_bytes} log bytes follow; complete log: ${report_path}" >&2
    tail -c "${failure_tail_bytes}" "${log_path}" >&2
    printf '\n%s\n' "bounded validation: end of failure excerpt" >&2
fi

exit "${command_status}"
