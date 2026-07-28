#!/usr/bin/env bash
set -euo pipefail

missing=0
file_list="$(mktemp /tmp/yo-test-explanations.XXXXXX)"
trap 'rm -f -- "${file_list}"' EXIT

rg --files crates tools/librarian tools/methexis -g '*.rs' | LC_ALL=C sort > "${file_list}"

while IFS= read -r file; do
    if ! awk '
        BEGIN {
            previous = ""
            failed = 0
        }

        /^[[:space:]]*#\[test\]([[:space:]]|$)/ {
            if (previous !~ /^[[:space:]]*\/\//) {
                printf "%s:%d: #[test] requires an explanatory line-comment immediately above it; review verifies Korean readability\n", FILENAME, NR
                failed = 1
            }
        }

        {
            previous = $0
        }

        END {
            exit failed
        }
    ' "${file}"; then
        missing=1
    fi
done < "${file_list}"

exit "${missing}"
