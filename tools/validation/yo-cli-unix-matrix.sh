#!/usr/bin/env bash
set -euo pipefail

host_target="$(rustc -vV | sed -n 's/^host: //p')"

cargo check --quiet --locked -p yo-cli --all-targets

case "${host_target}" in
    *-linux-*)
        printf '%s\n' "yo-cli target matrix: linux=verified macos=unverified(host unavailable)"
        ;;
    *-apple-darwin)
        printf '%s\n' "yo-cli target matrix: linux=unverified(host unavailable) macos=verified"
        ;;
    *)
        printf '%s\n' "yo-cli target matrix: linux=unverified(host unavailable) macos=unverified(host unavailable)"
        exit 1
        ;;
esac
