#!/usr/bin/env bash
set -euo pipefail

host_target="$(rustc -vV | sed -n 's/^host: //p')"

case "${host_target}" in
    *-linux-*)
        report="yo-cli Unix compile: linux=verified(current host) macos=unverified(not run on current host)"
        ;;
    *-apple-darwin)
        report="yo-cli Unix compile: linux=unverified(not run on current host) macos=verified(current host)"
        ;;
    *)
        printf '%s\n' \
            "yo-cli Unix compile: linux=unverified(unsupported current host) macos=unverified(unsupported current host)"
        exit 1
        ;;
esac

# Pin the checked target to the detected host so Cargo configuration or
# CARGO_BUILD_TARGET cannot turn a current-host claim into a cross-compile.
cargo check --quiet --locked -p yo-cli --all-targets --target "${host_target}"
printf '%s\n' "${report}"
