#!/usr/bin/env bash
set -euo pipefail

message_file=${1:?usage: slice-review-impact.sh <commit-message-file> [changed-paths-file] [branch]}
changed_paths_file=${2:-}
branch=${3:-$(git symbolic-ref --quiet --short HEAD || true)}

case "${branch}" in
    slice/*|task/*|spike/*)
        exit 0
        ;;
esac

if [[ "${branch}" == wave/* ]] &&
    merged_head=$(git rev-parse --quiet --verify MERGE_HEAD) &&
    git merge-base --is-ancestor "${merged_head}" refs/heads/develop; then
        exit 0
fi

if [[ -z "${branch}" ]]; then
    echo "Slice review impact cannot classify a detached or unresolved branch" >&2
    exit 1
fi

if [[ -n "${changed_paths_file}" ]]; then
    changed=$(cat "${changed_paths_file}")
else
    changed=$(git diff --cached --name-only --diff-filter=ACDMR)
    if [[ -z "${changed}" ]] && git rev-parse --quiet --verify HEAD >/dev/null; then
        changed=$(git diff-tree --root --no-commit-id --name-only -r HEAD)
    fi
fi

fresh_context_paths=$(
    grep -E \
        '^(AGENTS\.md$|README\.md$|CONTRIBUTING\.md$|hk\.pkl$|Cargo\.toml$|Cargo\.lock$|rust-toolchain(\.toml)?$|rustfmt\.toml$|\.gitignore$|\.github/workflows/|crates/|tools/|docs/book\.toml$|docs-internal/design/|methexis/)' \
        <<<"${changed}" || true
)

trailers=$(git interpret-trailers --parse <"${message_file}")
values=$(sed -nE 's/^Slice-Review:[[:space:]]*(.+)$/\1/p' <<<"${trailers}")
fresh_context=$(grep -E '^fresh-context - .+' <<<"${values}" || true)
integration=$(grep -E '^integration - .+' <<<"${values}" || true)
none=$(grep -E '^none - .+' <<<"${values}" || true)
value_count=$(printf '%s\n' "${values}" | sed '/^$/d' | wc -l | tr -d ' ')
recognized_count=$(
    printf '%s\n' "${fresh_context}" "${integration}" "${none}" |
        sed '/^$/d' |
        wc -l |
        tr -d ' '
)

fail_with_usage() {
    echo "$1" >&2
    echo "record completed review evidence with one or more trailers:" >&2
    echo "  Slice-Review: fresh-context - <reviewer and result>" >&2
    echo "  Slice-Review: integration - <reviewer and result>" >&2
    echo "or, only when no lens is required:" >&2
    echo "  Slice-Review: none - <why no additional review lens applies>" >&2
    exit 1
}

if [[ "${value_count}" -eq 0 ]]; then
    fail_with_usage "accepted commits must record Slice review disposition"
fi

if [[ "${recognized_count}" -ne "${value_count}" ]]; then
    fail_with_usage "invalid Slice-Review trailer"
fi

if [[ -n "${none}" && "${value_count}" -ne 1 ]]; then
    fail_with_usage "Slice-Review none cannot be combined with completed review lenses"
fi

if [[ $(printf '%s\n' "${fresh_context}" | sed '/^$/d' | wc -l | tr -d ' ') -gt 1 ]]; then
    fail_with_usage "fresh-context review must be recorded exactly once"
fi

if [[ $(printf '%s\n' "${integration}" | sed '/^$/d' | wc -l | tr -d ' ') -gt 1 ]]; then
    fail_with_usage "integration review must be recorded exactly once"
fi

if [[ -n "${fresh_context_paths}" && -z "${fresh_context}" ]]; then
    echo "these changes require fresh-context review:" >&2
    while IFS= read -r path; do
        printf '  changed: %s\n' "${path}" >&2
    done <<<"${fresh_context_paths}"
    fail_with_usage "fresh-context review evidence is missing"
fi

if [[ "${branch}" == wave/* && -z "${integration}" ]]; then
    fail_with_usage "accepted Wave Slice commits require integration review evidence"
fi
