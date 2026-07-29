#!/usr/bin/env bash
set -euo pipefail

readonly canonical=${1:-docs/src}
readonly korean=${2:-docs/ko/src}
readonly manifest_input=${3:-docs/ko/source.sha256}
readonly manifest_directory=$(
    cd "$(dirname "${manifest_input}")"
    pwd
)
readonly manifest="${manifest_directory}/$(basename "${manifest_input}")"

canonical_pages=$(mktemp -t yo-developer-docs-canonical.XXXXXX)
korean_pages=$(mktemp -t yo-developer-docs-korean.XXXXXX)
trap 'rm -f "${canonical_pages}" "${korean_pages}"' EXIT

(
    cd "${canonical}"
    find . -type f -name '*.md' -print | sort
) >"${canonical_pages}"

(
    cd "${korean}"
    find . -type f -name '*.md' -print | sort
) >"${korean_pages}"

if ! diff -u "${canonical_pages}" "${korean_pages}"; then
    echo "Korean Developer Docs must project the exact canonical page set" >&2
    exit 1
fi

count_matches() {
    grep -Ec "$1" "$2" || true
}

link_targets() {
    grep -Eo '\]\([^)]+\)' "$1" |
        sed -E 's/^\]\((.*)\)$/\1/; s/#.*$//' |
        sort || true
}

while IFS= read -r page; do
    page=${page#./}
    for structure in \
        'heading|^#{1,6} ' \
        'table row|^\|' \
        'code fence|^```' \
        'unordered list item|^[[:space:]]*- ' \
        'ordered list item|^[[:space:]]*[0-9]+\. '; do
        label=${structure%%|*}
        pattern=${structure#*|}
        canonical_count=$(count_matches "${pattern}" "${canonical}/${page}")
        korean_count=$(count_matches "${pattern}" "${korean}/${page}")
        if [[ "${canonical_count}" != "${korean_count}" ]]; then
            echo "${page}: Korean Projection ${label} count differs from canonical source" >&2
            exit 1
        fi
    done

    if ! diff -u \
        <(link_targets "${canonical}/${page}") \
        <(link_targets "${korean}/${page}"); then
        echo "${page}: Korean Projection link targets differ from canonical source" >&2
        exit 1
    fi
done <"${canonical_pages}"

(
    cd "${canonical}"
    shasum --algorithm 256 --check --status --strict "${manifest}"
)
