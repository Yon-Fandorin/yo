#!/usr/bin/env bash
set -euo pipefail

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX

readonly checker="$(pwd)/tools/validation/bounded-run.sh"
fixture=$(mktemp -d)
interruption_wrapper_pid=""
interruption_child_pid=""
term_ignoring_wrapper_pid=""
term_ignoring_child_pid=""
term_ignoring_descendant_pid=""
cleanup() {
    if [[ -n ${interruption_wrapper_pid} ]] && kill -0 "${interruption_wrapper_pid}" 2>/dev/null; then
        kill -KILL "${interruption_wrapper_pid}" 2>/dev/null || true
    fi
    if [[ -n ${interruption_child_pid} ]] && kill -0 "${interruption_child_pid}" 2>/dev/null; then
        kill -KILL "${interruption_child_pid}" 2>/dev/null || true
    fi
    if [[ -n ${term_ignoring_wrapper_pid} ]] && kill -0 "${term_ignoring_wrapper_pid}" 2>/dev/null; then
        kill -KILL "${term_ignoring_wrapper_pid}" 2>/dev/null || true
    fi
    if [[ -n ${term_ignoring_child_pid} ]] && kill -0 "${term_ignoring_child_pid}" 2>/dev/null; then
        kill -KILL "${term_ignoring_child_pid}" 2>/dev/null || true
    fi
    if [[ -n ${term_ignoring_descendant_pid} ]] && kill -0 "${term_ignoring_descendant_pid}" 2>/dev/null; then
        kill -KILL "${term_ignoring_descendant_pid}" 2>/dev/null || true
    fi
    rm -rf "${fixture}"
}
trap cleanup EXIT

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
rm -rf -- "${clean_repository}/run-logs"
clean_fixture_state=$(git -C "${clean_repository}" status --porcelain=v1 --untracked-files=normal)
if [[ -n "${clean_fixture_state}" ]]; then
    echo "clean state: fixture repository was not restored after the snapshot check" >&2
    exit 1
fi
printf '%s\n' 'fixture change' >"${clean_repository}/worktree-dirty"

(
    cd "${clean_repository}"
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
    bash "${checker}" --summary-out "${summary_root}/success.json" success -- bash -c \
    'printf "visible only in the full log\n"; printf "diagnostic\n" >&2' \
) >"${fixture}/success.out" 2>"${fixture}/success.err"

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
    "${success_summary}" != *'"head_commit":"'"${clean_head}"'"'* ||
    "${success_summary}" != *'"command_argv_count":3'* ||
    "${success_summary}" != *'"command_argv_hash":"sha256:b2feeb2dc7a19ae550541f96076627745b156652ed171a1f7bc182cbdee19b74"'* ||
    "${success_summary}" != *'"worktree_state":"dirty"'* ||
    "${success_summary}" != *'"reused":false'* ||
    "${success_summary}" != *'"reuse_policy":"reviewed-descendant/v1"'* ]]; then
    echo "success: unexpected summary for the dirty fixture repository" >&2
    exit 1
fi
rm -f -- "${clean_repository}/worktree-dirty"
clean_fixture_state=$(git -C "${clean_repository}" status --porcelain=v1 --untracked-files=normal)
if [[ -n "${clean_fixture_state}" ]]; then
    echo "success: fixture repository was not restored after the worktree check" >&2
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

(
    cd "${clean_repository}"
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        bash "${checker}" --resource-class cargo-heavy leased -- true
) >"${fixture}/leased.out" 2>"${fixture}/leased.err"
leased_summary=$(<"${fixture}/leased.out")
if [[ -s "${fixture}/leased.err" ||
    "${leased_summary}" != *'"schema":"yo.validation-run-summary/v1alpha4"'* ||
    "${leased_summary}" != *'"class":"cargo-heavy"'* ||
    "${leased_summary}" != *'"status":"acquired"'* ||
    -d "${clean_repository}/.local-exclude/validation-leases/cargo-heavy" ]]; then
    echo "resource lease: expected one released cargo-heavy lease and v1alpha4 evidence" >&2
    exit 1
fi

interruption_child_script="${fixture}/interruption-child.sh"
interruption_child_pid_file="${fixture}/interruption-child.pid"
interruption_child_marker="${fixture}/interruption-child.signal"
cat >"${interruption_child_script}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$$" >"${INTERRUPTION_CHILD_PID_FILE}"
record_signal() {
    printf '%s\n' "$1" >"${INTERRUPTION_CHILD_MARKER}"
    exit "$2"
}
trap 'record_signal TERM 143' TERM
trap 'record_signal INT 130' INT
trap 'record_signal HUP 129' HUP
while :; do
    :
done
EOF
chmod +x "${interruption_child_script}"

set +e
(
    cd "${clean_repository}"
    INTERRUPTION_CHILD_PID_FILE="${interruption_child_pid_file}" \
    INTERRUPTION_CHILD_MARKER="${interruption_child_marker}" \
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        exec bash "${checker}" --summary-out "${summary_root}/interrupted.json" \
        --resource-class cargo-heavy interrupted -- "${interruption_child_script}"
) >"${fixture}/interrupted.out" 2>"${fixture}/interrupted.err" &
interruption_wrapper_pid=$!
for _ in {1..100}; do
    if [[ -s "${interruption_child_pid_file}" ]]; then
        break
    fi
    sleep 0.01
done
if [[ ! -s "${interruption_child_pid_file}" ]]; then
    echo "interruption: child did not become ready" >&2
    exit 1
fi
interruption_child_pid=$(<"${interruption_child_pid_file}")
kill -TERM "${interruption_wrapper_pid}"
interruption_wait_deadline=$(( $(date +%s) + 10 ))
while kill -0 "${interruption_wrapper_pid}" 2>/dev/null; do
    if [[ $(date +%s) -ge ${interruption_wait_deadline} ]]; then
        echo "interruption: wrapper did not exit before the deadline" >&2
        kill -KILL "${interruption_wrapper_pid}" 2>/dev/null || true
        exit 1
    fi
    sleep 0.05
done
wait "${interruption_wrapper_pid}"
interrupted_status=$?
set -e
interruption_wrapper_pid=""
interruption_child_alive=false
if kill -0 "${interruption_child_pid}" 2>/dev/null; then
    interruption_child_alive=true
fi
if [[ ${interrupted_status} -ne 143 ]]; then
    echo "interruption: wrapper must preserve the forwarded TERM status" >&2
    exit 1
fi
if [[ ! -f "${interruption_child_marker}" ||
    "$(<"${interruption_child_marker}")" != TERM ||
    ${interruption_child_alive} == true ||
    ! -f "${summary_root}/interrupted.json" ||
    ! -s "${fixture}/interrupted.out" ||
    -d "${clean_repository}/.local-exclude/validation-leases/cargo-heavy" ]]; then
    echo "interruption: child, summary, and lease state were not finalized" >&2
    exit 1
fi
if ! cmp -s "${fixture}/interrupted.out" "${summary_root}/interrupted.json" ||
    ! grep -q '"exit_code":143' "${summary_root}/interrupted.json"; then
    echo "interruption: published summary must preserve the signal status" >&2
    exit 1
fi

(
    cd "${clean_repository}"
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        bash "${checker}" --resource-class cargo-heavy after-interruption -- true
) >"${fixture}/after-interruption.out" 2>"${fixture}/after-interruption.err"
after_interruption_summary=$(<"${fixture}/after-interruption.out")
if [[ -s "${fixture}/after-interruption.err" ||
    "${after_interruption_summary}" != *'"status":"passed"'* ||
    -d "${clean_repository}/.local-exclude/validation-leases/cargo-heavy" ]]; then
    echo "interruption: a subsequent lease must be acquired after cleanup" >&2
    exit 1
fi

term_ignoring_child_script="${fixture}/term-ignoring-child.sh"
term_ignoring_child_pid_file="${fixture}/term-ignoring-child.pid"
term_ignoring_descendant_pid_file="${fixture}/term-ignoring-descendant.pid"
term_ignoring_lease_marker="${fixture}/term-ignoring-lease-released-while-alive"
cat >"${term_ignoring_child_script}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
trap ':' TERM
trap ':' INT
trap ':' HUP
printf '%s\n' "$$" >"${TERM_IGNORING_CHILD_PID_FILE}"
(
    trap ':' TERM
    trap ':' INT
    trap ':' HUP
    while :; do
        if [[ ! -d ${TERM_IGNORING_LEASE_PATH} ]]; then
            printf '%s\n' lease-released-while-alive >"${TERM_IGNORING_LEASE_MARKER}"
            exit 99
        fi
    done
) &
term_ignoring_descendant_pid=$!
printf '%s\n' "${term_ignoring_descendant_pid}" >"${TERM_IGNORING_DESCENDANT_PID_FILE}"
while :; do
    :
done
EOF
chmod +x "${term_ignoring_child_script}"

set +e
(
    cd "${clean_repository}"
    TERM_IGNORING_CHILD_PID_FILE="${term_ignoring_child_pid_file}" \
    TERM_IGNORING_DESCENDANT_PID_FILE="${term_ignoring_descendant_pid_file}" \
    TERM_IGNORING_LEASE_PATH="${clean_repository}/.local-exclude/validation-leases/cargo-heavy" \
    TERM_IGNORING_LEASE_MARKER="${term_ignoring_lease_marker}" \
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        exec bash "${checker}" --summary-out "${summary_root}/term-ignoring.json" \
        --resource-class cargo-heavy term-ignoring -- "${term_ignoring_child_script}"
) >"${fixture}/term-ignoring.out" 2>"${fixture}/term-ignoring.err" &
term_ignoring_wrapper_pid=$!
term_ignoring_ready_deadline=$(( $(date +%s) + 10 ))
while [[ ! -s "${term_ignoring_child_pid_file}" ||
    ! -s "${term_ignoring_descendant_pid_file}" ]]; do
    if [[ $(date +%s) -ge ${term_ignoring_ready_deadline} ]]; then
        echo "term-ignoring descendant: child did not become ready" >&2
        exit 1
    fi
    sleep 0.01
done
term_ignoring_child_pid=$(<"${term_ignoring_child_pid_file}")
term_ignoring_descendant_pid=$(<"${term_ignoring_descendant_pid_file}")
kill -TERM "${term_ignoring_wrapper_pid}"
term_ignoring_wait_deadline=$(( $(date +%s) + 10 ))
while kill -0 "${term_ignoring_wrapper_pid}" 2>/dev/null; do
    if [[ $(date +%s) -ge ${term_ignoring_wait_deadline} ]]; then
        echo "term-ignoring descendant: wrapper did not exit before the deadline" >&2
        kill -KILL "${term_ignoring_wrapper_pid}" 2>/dev/null || true
        exit 1
    fi
    sleep 0.05
done
wait "${term_ignoring_wrapper_pid}"
term_ignoring_status=$?
set -e
term_ignoring_wrapper_pid=""
term_ignoring_disappearance_deadline=$(( $(date +%s) + 5 ))
while kill -0 "${term_ignoring_child_pid}" 2>/dev/null ||
    kill -0 "${term_ignoring_descendant_pid}" 2>/dev/null; do
    if [[ $(date +%s) -ge ${term_ignoring_disappearance_deadline} ]]; then
        echo "term-ignoring descendant: process group did not disappear before the deadline" >&2
        exit 1
    fi
    sleep 0.05
done
if [[ ${term_ignoring_status} -ne 143 ||
    -f "${term_ignoring_lease_marker}" ||
    ! -f "${summary_root}/term-ignoring.json" ||
    -d "${clean_repository}/.local-exclude/validation-leases/cargo-heavy" ]]; then
    echo "term-ignoring descendant: process-group cleanup must precede lease release" >&2
    exit 1
fi
if ! grep -q '"exit_code":143' "${summary_root}/term-ignoring.json" ||
    ! cmp -s "${fixture}/term-ignoring.out" "${summary_root}/term-ignoring.json"; then
    echo "term-ignoring descendant: published summary must preserve the signal status" >&2
    exit 1
fi

(
    cd "${clean_repository}"
    PATH="${fixture}/bin:${PATH}" \
    SYSTEM_MKTEMP="${system_mktemp}" \
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        bash "${checker}" --resource-class cargo-heavy after-term-ignoring -- true
) >"${fixture}/after-term-ignoring.out" 2>"${fixture}/after-term-ignoring.err"
after_term_ignoring_summary=$(<"${fixture}/after-term-ignoring.out")
if [[ -s "${fixture}/after-term-ignoring.err" ||
    "${after_term_ignoring_summary}" != *'"status":"passed"'* ||
    -d "${clean_repository}/.local-exclude/validation-leases/cargo-heavy" ]]; then
    echo "term-ignoring descendant: a subsequent lease must be acquired after cleanup" >&2
    exit 1
fi

mkdir -p "${clean_repository}/.local-exclude/validation-leases/cargo-heavy"
set +e
(
    cd "${clean_repository}"
    YO_BOUNDED_VALIDATION_LOG_ROOT="${log_root}" \
        bash "${checker}" --resource-class cargo-heavy collision -- \
        bash -c 'touch "$1"' _ "${fixture}/leased-collision-ran"
) >"${fixture}/leased-collision.out" 2>"${fixture}/leased-collision.err"
leased_collision_status=$?
set -e
rmdir "${clean_repository}/.local-exclude/validation-leases/cargo-heavy"
if [[ ${leased_collision_status} -ne 75 || -e "${fixture}/leased-collision-ran" ||
    -s "${fixture}/leased-collision.out" ]] ||
    ! grep -q 'resource lease is already held' "${fixture}/leased-collision.err"; then
    echo "resource lease collision: the second cargo-heavy child must fail before execution" >&2
    exit 1
fi

set +e
(
    cd "${clean_repository}"
    unset CARGO_TARGET_DIR
    bash "${checker}" --resource-class independent missing-target -- true
) >"${fixture}/independent.out" 2>"${fixture}/independent.err"
independent_status=$?
set -e
if [[ ${independent_status} -ne 64 ]] ||
    ! grep -q 'requires an absolute CARGO_TARGET_DIR' "${fixture}/independent.err"; then
    echo "independent lease: an explicit isolated Cargo target is required" >&2
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
