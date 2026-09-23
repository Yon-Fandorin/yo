#!/usr/bin/env bash
set -euo pipefail

readonly checker="$(pwd)/tools/validation/developer-docs-translations.sh"
fixture=$(mktemp -d -t yo-developer-docs-translations.XXXXXX)
trap 'rm -rf "${fixture}"' EXIT

reset_fixture() {
    rm -rf "${fixture}/canonical" "${fixture}/korean"
    mkdir -p "${fixture}/canonical" "${fixture}/korean"
    printf '# Canonical\n[Overview](README.md#canonical)\n' >"${fixture}/canonical/README.md"
    printf '# 한국어\n[개요](README.md#한국어)\n' >"${fixture}/korean/README.md"
    (
        cd "${fixture}/canonical"
        shasum --algorithm 256 README.md
    ) >"${fixture}/source.sha256"
}

expect_pass() {
    local description=$1
    if ! bash "${checker}" \
        "${fixture}/canonical" \
        "${fixture}/korean" \
        "${fixture}/source.sha256"; then
        echo "expected translation validation to pass: ${description}" >&2
        exit 1
    fi
}

expect_fail() {
    local description=$1
    if bash "${checker}" \
        "${fixture}/canonical" \
        "${fixture}/korean" \
        "${fixture}/source.sha256" >/dev/null 2>&1; then
        echo "expected translation validation to fail: ${description}" >&2
        exit 1
    fi
}

reset_fixture
expect_pass "한국어 Projection은 페이지 집합과 링크 경로를 유지하고 원문 해시를 통과한다"

printf '# Changed canonical\n' >"${fixture}/canonical/README.md"
expect_fail "the canonical source changed after translation review"

reset_fixture
printf '# Extra page\n' >"${fixture}/canonical/extra.md"
expect_fail "the Korean Projection is missing a canonical page"

reset_fixture
printf '# Extra projection\n' >"${fixture}/korean/extra.md"
expect_fail "the Korean Projection contains an unowned extra page"

reset_fixture
printf '\n## 검토되지 않은 절\n' >>"${fixture}/korean/README.md"
expect_fail "the Korean Projection structure diverges from its canonical page"

reset_fixture
printf '# 한국어\n[개요](other.md#한국어)\n' >"${fixture}/korean/README.md"
expect_fail "한국어 Projection의 링크 대상 경로가 원문과 다르다"
