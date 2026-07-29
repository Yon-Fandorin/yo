#!/usr/bin/env bash
set -euo pipefail

readonly expected_mdbook='mdbook v0.5.4'

if ! command -v mdbook >/dev/null 2>&1; then
    echo "developer docs require ${expected_mdbook}; install it with:" >&2
    echo "  cargo install mdbook --version 0.5.4 --locked" >&2
    exit 1
fi

actual_mdbook=$(mdbook --version)
if [[ "${actual_mdbook}" != "${expected_mdbook}" ]]; then
    echo "developer docs require ${expected_mdbook}, found ${actual_mdbook}" >&2
    exit 1
fi

repository='https://github.com/Yon-Fandorin/yo/blob/develop/'
while IFS= read -r document; do
    if grep -Eq '^[[:space:]]*\[[^^][^]]*\]:' "${document}" ||
        grep -Eq '<https?://[^>]+>' "${document}"; then
        echo "${document}: use inline Markdown links so validation can inspect every target" >&2
        exit 1
    fi
    while IFS= read -r target; do
        target=${target%%#*}
        [[ -z "${target}" ]] && continue
        case "${target}" in
            http://*|https://*)
                if [[ "${target}" == "${repository}"* ]]; then
                    path=${target#"${repository}"}
                    [[ -e "${path}" ]] || {
                        echo "${document}: repository link target does not exist: ${path}" >&2
                        exit 1
                    }
                fi
                ;;
            mailto:*|\#*)
                ;;
            *)
                path="$(dirname "${document}")/${target}"
                [[ -e "${path}" ]] || {
                    echo "${document}: local link target does not exist: ${target}" >&2
                    exit 1
                }
                ;;
        esac
    done < <(
        grep -Eo '\]\([^)]+\)' "${document}" |
            sed -E 's/^\]\((.*)\)$/\1/'
    )
done < <(find docs/src -type f -name '*.md' -print)

mdbook build docs

for output in target/developer-docs/index.html target/developer-docs/toc.html; do
    if ! grep -Eq 'href="theme/yo-[^"]+\.css"' "${output}"; then
        echo "${output}: built Developer Docs do not reference the yo theme" >&2
        exit 1
    fi
done

if ! find target/developer-docs/theme -maxdepth 1 -type f -name 'yo-*.css' -print -quit |
    grep -q .; then
    echo "built Developer Docs do not contain the hashed yo theme asset" >&2
    exit 1
fi
