#!/usr/bin/env bash
set -euo pipefail

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX

readonly checker="$(pwd)/tools/validation/bounded-run.sh"
fixture=$(mktemp -d)
trap 'rm -rf "${fixture}"' EXIT

readonly log_root="${fixture}/logs"
mkdir -p "${log_root}"
readonly summary_root="${fixture}/summaries"
mkdir -p "${summary_root}"
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

clean_repository="${fixture}/clean-repository"
git init --quiet "${clean_repository}"
git -C "${clean_repository}" config user.name "Bounded Run Test"
git -C "${clean_repository}" config user.email "bounded-run@example.invalid"
git -C "${clean_repository}" commit --quiet --allow-empty -m base
clean_head=$(git -C "${clean_repository}" rev-parse HEAD)
(
    cd "${clean_repository}"
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${clean_repository}/run-logs" \
        bash "${checker}" clean-state -- true
) >"${fixture}/clean.out" 2>"${fixture}/clean.err"
clean_summary=$(<"${fixture}/clean.out")
if [[ -s "${fixture}/clean.err" ||
    "${clean_summary}" != *'"head_commit":"'"${clean_head}"'"'* ||
    "${clean_summary}" != *'"worktree_state":"clean"'* ]]; then
    echo "clean state: wrapper artifacts must not dirty the launch snapshot" >&2
    exit 1
fi

PATH="${fixture}/bin:${PATH}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${summary_root}/success.json" success -- bash -c \
    'printf "visible only in the full log\n"; printf "diagnostic\n" >&2' \
    >"${fixture}/success.out" 2>"${fixture}/success.err"

if [[ -s "${fixture}/success.err" ]]; then
    echo "success: wrapper must keep command output out of stderr" >&2
    exit 1
fi
if ! cmp -s "${fixture}/success.out" "${summary_root}/success.json"; then
    echo "success: published summary must be byte-identical to stdout" >&2
    exit 1
fi
if [[ $(wc -l <"${fixture}/success.out") -ne 1 ]]; then
    echo "success: expected exactly one summary line" >&2
    exit 1
fi
success_summary=$(<"${fixture}/success.out")
if [[ "${success_summary}" != *'"schema":"yo.validation-run-summary/v1alpha2"'* ||
    "${success_summary}" != *'"name":"success"'* ||
    "${success_summary}" != *'"status":"passed"'* ||
    "${success_summary}" != *'"exit_code":0'* ||
    "${success_summary}" != *'"log_hash":"sha256:1c1e319bdabcf409b2276fa2cce92da2a75b5d642552bfba278bfe680a2a5789"'* ||
    "${success_summary}" != *'"head_commit":"'* ||
    "${success_summary}" != *'"command_argv_count":3'* ||
    "${success_summary}" != *'"command_argv_hash":"sha256:b2feeb2dc7a19ae550541f96076627745b156652ed171a1f7bc182cbdee19b74"'* ||
    "${success_summary}" != *'"reused":false'* ||
    "${success_summary}" != *'"reuse_policy":"reviewed-descendant/v1"'* ]]; then
    echo "success: unexpected summary" >&2
    exit 1
fi
if [[ "${success_summary}" != *'"worktree_state":"clean"'* &&
    "${success_summary}" != *'"worktree_state":"dirty"'* ]]; then
    echo "success: missing bounded worktree state" >&2
    exit 1
fi
success_log=$(find "${log_root}" -type f -name 'success.log.*' -print)
if [[ -z "${success_log}" || "$(<"${success_log}")" != $'visible only in the full log\ndiagnostic' ]]; then
    echo "success: complete combined log was not retained" >&2
    exit 1
fi

PATH="${fixture}/bin:${PATH}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --reusable-local reusable-local -- true \
    >"${fixture}/reusable-local.out" 2>"${fixture}/reusable-local.err"
reusable_summary=$(<"${fixture}/reusable-local.out")
if [[ -s "${fixture}/reusable-local.err" ||
    "${reusable_summary}" != *'"schema":"yo.validation-run-summary/v1alpha3"'* ||
    "${reusable_summary}" != *'"reuse_policy":"reviewed-descendant-context/v1"'* ||
    "${reusable_summary}" != *'"reuse_context":{"schema":"yo.validation-reuse-context/v1alpha1"'* ||
    "${reusable_summary}" != *'"toolchain_hash":"sha256:'* ||
    "${reusable_summary}" != *'"external_state":"none-declared"'* ]]; then
    echo "reusable local: expected a context-bound v1alpha3 summary" >&2
    exit 1
fi

set +e
PATH="${fixture}/bin:${PATH}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${summary_root}/failure.json" failure -- bash -c \
    'printf "BEGIN-OF-FULL-LOG\n"; head -c 20000 /dev/zero | tr "\0" x; printf "\nEND-OF-FULL-LOG\n"; exit 7' \
    >"${fixture}/failure.out" 2>"${fixture}/failure.err"
failure_status=$?
set -e

if [[ ${failure_status} -ne 7 ]]; then
    echo "failure: wrapper did not preserve command status" >&2
    exit 1
fi
if ! cmp -s "${fixture}/failure.out" "${summary_root}/failure.json"; then
    echo "failure: published summary must preserve the failed child result" >&2
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

printf '%s\n' 'existing summary' >"${summary_root}/existing.json"
set +e
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${summary_root}/existing.json" collision -- \
    bash -c 'touch "$1"' _ "${fixture}/collision-ran" \
    >"${fixture}/collision.out" 2>"${fixture}/collision.err"
collision_status=$?
set -e

if [[ ${collision_status} -ne 73 || -e "${fixture}/collision-ran" ||
    -s "${fixture}/collision.out" ||
    "$(<"${summary_root}/existing.json")" != 'existing summary' ]]; then
    echo "summary collision: existing evidence must stop before the child and remain unchanged" >&2
    exit 1
fi
if ! grep -q 'summary output already exists' "${fixture}/collision.err"; then
    echo "summary collision: expected a focused diagnostic" >&2
    exit 1
fi

set +e
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${fixture}/missing/summary.json" missing-parent -- \
    bash -c 'touch "$1"' _ "${fixture}/missing-parent-ran" \
    >"${fixture}/missing-parent.out" 2>"${fixture}/missing-parent.err"
missing_parent_status=$?
set -e

if [[ ${missing_parent_status} -ne 73 || -e "${fixture}/missing-parent-ran" ||
    -e "${fixture}/missing" || -s "${fixture}/missing-parent.out" ]]; then
    echo "missing parent: publication must fail before filesystem or child effects" >&2
    exit 1
fi
if ! grep -q 'summary output parent must already exist' "${fixture}/missing-parent.err"; then
    echo "missing parent: expected a focused diagnostic" >&2
    exit 1
fi

mkdir -p "${fixture}/race-bin"
system_ln=$(command -v ln)
cat >"${fixture}/race-bin/ln" <<'EOF'
#!/usr/bin/env bash
mkdir -- "$2"
exec "${SYSTEM_LN}" "$@"
EOF
chmod +x "${fixture}/race-bin/ln"

set +e
PATH="${fixture}/race-bin:${fixture}/bin:${PATH}" \
SYSTEM_LN="${system_ln}" \
SYSTEM_MKTEMP="${system_mktemp}" \
YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${summary_root}/raced-directory.json" raced-directory -- true \
    >"${fixture}/raced-directory.out" 2>"${fixture}/raced-directory.err"
raced_directory_status=$?
set -e
raced_directory_entries=$(ls -A "${summary_root}/raced-directory.json")

if [[ ${raced_directory_status} -ne 73 ||
    ! -d "${summary_root}/raced-directory.json" ||
    -n ${raced_directory_entries} ||
    -s "${fixture}/raced-directory.out" ]]; then
    echo "raced directory: publication must fail without leaving a nested summary" >&2
    exit 1
fi
if ! grep -q 'cannot atomically create summary output' "${fixture}/raced-directory.err"; then
    echo "raced directory: expected a focused diagnostic" >&2
    exit 1
fi

echo "bounded validation runner: all tests passed"
