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
done < <(find docs/src docs/ko/src -type f -name '*.md' -print)

bash tools/validation/developer-docs-translations.sh
bash tools/validation/developer-docs-translations-tests.sh
bash tools/validation/developer-docs-build.sh

for output in target/developer-docs/en/index.html target/developer-docs/en/toc.html; do
    if ! grep -Eq 'href="theme/yo-[^"]+\.css"' "${output}"; then
        echo "${output}: built English Developer Docs do not reference the yo theme" >&2
        exit 1
    fi
done

for output in target/developer-docs/ko/index.html target/developer-docs/ko/toc.html; do
    if ! grep -Fq 'href="../theme/yo.css"' "${output}"; then
        echo "${output}: built Korean Developer Docs do not reference the shared yo theme" >&2
        exit 1
    fi
done

if ! find target/developer-docs/en/theme \
    -maxdepth 1 -type f -name 'yo-*.css' -print -quit | grep -q .; then
    echo "built English Developer Docs do not contain the hashed yo theme asset" >&2
    exit 1
fi

for asset in \
    target/developer-docs/theme/yo.css \
    target/developer-docs/theme/language-switch.js; do
    if [[ ! -f "${asset}" ]]; then
        echo "built Korean Developer Docs do not contain shared asset: ${asset}" >&2
        exit 1
    fi
done

grep -Fq 'window.location.replace' target/developer-docs/index.html || {
    echo "built Developer Docs do not contain the language entry point" >&2
    exit 1
}

for language in en ko; do
    if ! grep -Fq "href=\"./${language}/\"" target/developer-docs/index.html; then
        echo "built Developer Docs do not contain the ${language} fallback link" >&2
        exit 1
    fi
done

for stale in architecture validation workflows print.html toc.html; do
    if [[ -e "target/developer-docs/${stale}" ]]; then
        echo "built Developer Docs retain stale single-language output: ${stale}" >&2
        exit 1
    fi
done

if grep -R -Eq 'href="[^"]*README\.html([#"]|$)' \
    target/developer-docs/en target/developer-docs/ko; then
    echo "built Developer Docs contain an internal README.html link instead of an index route" >&2
    exit 1
fi

grep -Fq 'className = "yo-language-switch"' \
    target/developer-docs/theme/language-switch.js || {
    echo "built Developer Docs do not contain the language switch" >&2
    exit 1
}
