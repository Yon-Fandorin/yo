#!/usr/bin/env bash
set -euo pipefail

readonly checker="$(pwd)/tools/validation/bounded-run.sh"
fixture=$(mktemp -d)
trap 'rm -rf "${fixture}"' EXIT

readonly log_root="${fixture}/logs"
mkdir -p "${log_root}"
readonly system_mktemp="$(command -v mktemp)"
mkdir -p "${fixture}/bin"
cat >"${fixture}/bin/mktemp" <<'EOF'
#!/usr/bin/env bash
if [[ $# -ne 1 || $1 != *XXXXXX ]]; then
    echo "portable mktemp fixture: template must end in XXXXXX" >&2
    exit 64
fi
exec "${SYSTEM_MKTEMP}" "$1"
EOF
chmod +x "${fixture}/bin/mktemp"

PATH="${fixture}/bin:${PATH}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" success -- bash -c \
    'printf "visible only in the full log\n"; printf "diagnostic\n" >&2' \
    >"${fixture}/success.out" 2>"${fixture}/success.err"

if [[ -s "${fixture}/success.err" ]]; then
    echo "success: wrapper must keep command output out of stderr" >&2
    exit 1
fi
if [[ $(wc -l <"${fixture}/success.out") -ne 1 ]]; then
    echo "success: expected exactly one summary line" >&2
    exit 1
fi
success_summary=$(<"${fixture}/success.out")
if [[ "${success_summary}" != *'"schema":"yo.validation-run-summary/v1"'* ||
    "${success_summary}" != *'"name":"success"'* ||
    "${success_summary}" != *'"status":"passed"'* ||
    "${success_summary}" != *'"exit_code":0'* ]]; then
    echo "success: unexpected summary" >&2
    exit 1
fi
success_log=$(find "${log_root}" -type f -name 'success.log.*' -print)
if [[ -z "${success_log}" || "$(<"${success_log}")" != $'visible only in the full log\ndiagnostic' ]]; then
    echo "success: complete combined log was not retained" >&2
    exit 1
fi

set +e
PATH="${fixture}/bin:${PATH}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" failure -- bash -c \
    'printf "BEGIN-OF-FULL-LOG\n"; head -c 20000 /dev/zero | tr "\0" x; printf "\nEND-OF-FULL-LOG\n"; exit 7' \
    >"${fixture}/failure.out" 2>"${fixture}/failure.err"
failure_status=$?
set -e

if [[ ${failure_status} -ne 7 ]]; then
    echo "failure: wrapper did not preserve command status" >&2
    exit 1
fi
failure_summary=$(<"${fixture}/failure.out")
if [[ "${failure_summary}" != *'"status":"failed"'* ||
    "${failure_summary}" != *'"exit_code":7'* ]]; then
    echo "failure: unexpected summary" >&2
    exit 1
fi
if grep -q 'BEGIN-OF-FULL-LOG' "${fixture}/failure.err" ||
    ! grep -q 'END-OF-FULL-LOG' "${fixture}/failure.err"; then
    echo "failure: stderr must contain only the bounded tail" >&2
    exit 1
fi
if [[ $(wc -c <"${fixture}/failure.err") -gt 17000 ]]; then
    echo "failure: stderr exceeded the bounded diagnostic budget" >&2
    exit 1
fi
failure_log=$(find "${log_root}" -type f -name 'failure.log.*' -print)
if ! grep -q 'BEGIN-OF-FULL-LOG' "${failure_log}" ||
    ! grep -q 'END-OF-FULL-LOG' "${failure_log}"; then
    echo "failure: full log was truncated" >&2
    exit 1
fi

set +e
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" '../invalid' -- bash -c 'touch "$1"' _ "${fixture}/ran" \
    >"${fixture}/invalid.out" 2>"${fixture}/invalid.err"
invalid_status=$?
set -e

if [[ ${invalid_status} -ne 64 || -e "${fixture}/ran" ]]; then
    echo "invalid name: wrapper must reject before running the command" >&2
    exit 1
fi

control_log_root="${fixture}/control"$'\001'"path"
set +e
YO_BOUNDED_VALIDATION_LOG_ROOT="${control_log_root}" \
    bash "${checker}" control-path -- bash -c 'touch "$1"' _ "${fixture}/control-ran" \
    >"${fixture}/control.out" 2>"${fixture}/control.err"
control_status=$?
set -e

if [[ ${control_status} -ne 64 || -e "${fixture}/control-ran" ||
    -e "${control_log_root}" ]]; then
    echo "control path: wrapper must reject before filesystem or child effects" >&2
    exit 1
fi
if ! grep -q 'log root contains unsupported control bytes' "${fixture}/control.err"; then
    echo "control path: expected a focused diagnostic" >&2
    exit 1
fi

echo "bounded validation runner: all tests passed"
