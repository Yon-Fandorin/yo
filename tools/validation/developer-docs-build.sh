#!/usr/bin/env bash
set -euo pipefail

readonly output='target/developer-docs'
mkdir -p "$(dirname "${output}")"
staging=$(mktemp -d 'target/developer-docs.XXXXXX')
trap 'rm -rf "${staging}"' EXIT

mdbook build docs --dest-dir "${staging}/en"
mdbook build docs/ko --dest-dir "${staging}/ko"
cp docs/theme/language-index.html "${staging}/index.html"

rm -rf "${output}"
mv "${staging}" "${output}"
trap - EXIT
