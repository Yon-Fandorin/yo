#!/usr/bin/env bash
set -euo pipefail

message_file=${1:?usage: developer-docs-impact.sh <commit-message-file> [changed-paths-file] [branch]}
changed_paths_file=${2:-}
branch=${3:-$(git symbolic-ref --quiet --short HEAD || true)}

case "${branch}" in
    slice/*|task/*|spike/*)
        exit 0
        ;;
esac

if git rev-parse --quiet --verify MERGE_HEAD >/dev/null; then
    exit 0
fi

if [[ -n "${changed_paths_file}" ]]; then
    all_changed=$(cat "${changed_paths_file}")
else
    all_changed=$(git diff --cached --name-only --diff-filter=ACDMR)
fi
changed=$(
    grep -E '^(crates/|tools/|Cargo\.toml$|Cargo\.lock$)' <<<"${all_changed}" ||
        true
)

if [[ -z "${changed}" ]]; then
    exit 0
fi

values=$(sed -nE 's/^Developer-Docs-Impact:[[:space:]]*(.+)$/\1/p' "${message_file}")
value_count=$(printf '%s\n' "${values}" | sed '/^$/d' | wc -l | tr -d ' ')
if [[ "${value_count}" -ne 1 ]]; then
    echo "code changes require exactly one Developer-Docs-Impact trailer" >&2
    while IFS= read -r path; do
        printf '  changed: %s\n' "${path}" >&2
    done <<<"${changed}"
    echo "use one of:" >&2
    echo "  Developer-Docs-Impact: updated" >&2
    echo "  Developer-Docs-Impact: none - <why responsibilities and flows stay accurate>" >&2
    exit 1
fi

case "${values}" in
    updated)
        if ! grep -q '^docs/src/' <<<"${all_changed}"; then
            echo "Developer-Docs-Impact says updated, but docs/src has no staged change" >&2
            exit 1
        fi
        ;;
    "none - "?*)
        ;;
    *)
        echo "invalid Developer-Docs-Impact value: ${values}" >&2
        echo "expected 'updated' or 'none - <reason>'" >&2
        exit 1
        ;;
esac
