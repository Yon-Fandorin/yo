#!/usr/bin/env bash
set -euo pipefail

readonly checker="$(pwd)/tools/validation/yo-cli-unix-matrix.sh"
fixture=$(mktemp -d)
trap 'rm -rf "${fixture}"' EXIT

mkdir -p "${fixture}/bin"

cat >"${fixture}/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$#" >>"${YO_UNIX_MATRIX_CARGO_ARGS}"
printf '<%s>\n' "$@" >>"${YO_UNIX_MATRIX_CARGO_ARGS}"
EOF
chmod +x "${fixture}/bin/cargo"

run_case() {
    local name=$1
    local target=$2
    local expected_status=$3
    local expected_output=$4
    shift 4
    local expected_cargo_argc=$#
    local -a expected_cargo_argv=("$@")
    local output_file="${fixture}/${name}.out"
    local cargo_args="${fixture}/${name}.cargo"
    local rustc_args="${fixture}/${name}.rustc"

    cat >"${fixture}/bin/rustc" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$#" >>'${rustc_args}'
printf '<%s>\n' "\$@" >>'${rustc_args}'
printf '%s\n' 'host: ${target}'
EOF
    chmod +x "${fixture}/bin/rustc"

    set +e
    PATH="${fixture}/bin:${PATH}" \
        YO_UNIX_MATRIX_CARGO_ARGS="${cargo_args}" \
        bash "${checker}" >"${output_file}" 2>&1
    local actual_status=$?
    set -e

    if [[ "${actual_status}" -ne "${expected_status}" ]]; then
        echo "${name}: expected status ${expected_status}, got ${actual_status}" >&2
        exit 1
    fi
    if [[ "$(<"${output_file}")" != "${expected_output}" ]]; then
        echo "${name}: unexpected report" >&2
        cat "${output_file}" >&2
        exit 1
    fi
    if [[ "$(<"${rustc_args}")" != $'1\n<-vV>' ]]; then
        echo "${name}: rustc host query changed" >&2
        exit 1
    fi
    if [[ ${expected_cargo_argc} -gt 0 ]]; then
        local expected_cargo_trace="${fixture}/${name}.cargo.expected"
        {
            printf '%s\n' "${expected_cargo_argc}"
            printf '<%s>\n' "${expected_cargo_argv[@]}"
        } >"${expected_cargo_trace}"
        if ! cmp -s "${expected_cargo_trace}" "${cargo_args}"; then
            echo "${name}: compile argv or invocation count changed" >&2
            exit 1
        fi
    elif [[ -e "${cargo_args}" ]]; then
        echo "${name}: unsupported hosts must fail before Cargo" >&2
        exit 1
    fi
}

run_case \
    "linux" \
    "x86_64-unknown-linux-gnu" \
    0 \
    "yo-cli Unix compile: linux=verified(current host) macos=unverified(not run on current host)" \
    check --quiet --locked -p yo-cli --all-targets --target x86_64-unknown-linux-gnu

run_case \
    "macos" \
    "aarch64-apple-darwin" \
    0 \
    "yo-cli Unix compile: linux=unverified(not run on current host) macos=verified(current host)" \
    check --quiet --locked -p yo-cli --all-targets --target aarch64-apple-darwin

run_case \
    "unsupported" \
    "x86_64-pc-windows-msvc" \
    1 \
    "yo-cli Unix compile: linux=unverified(unsupported current host) macos=unverified(unsupported current host)"
