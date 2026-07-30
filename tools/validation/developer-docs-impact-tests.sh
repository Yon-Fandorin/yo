#!/usr/bin/env bash
set -euo pipefail

readonly checker="$(pwd)/tools/validation/developer-docs-impact.sh"
fixture=$(mktemp -d)
trap 'rm -rf "${fixture}"' EXIT

write_fixture() {
    printf '%s\n' "$2" >"${fixture}/message"
    printf '%s\n' "$3" >"${fixture}/paths"
    branch=$4
}

expect_pass() {
    write_fixture "$@"
    bash "${checker}" "${fixture}/message" "${fixture}/paths" "${branch}"
}

expect_fail() {
    write_fixture "$@"
    if bash "${checker}" "${fixture}/message" "${fixture}/paths" "${branch}" >/dev/null 2>&1; then
        echo "expected Developer Docs impact case to fail: $1" >&2
        exit 1
    fi
}

expect_staged_deletion_fail() {
    repository="${fixture}/deletion"
    inherited_git_dir="${fixture}/inherited.git"
    inherited_index="${fixture}/inherited.index"
    inherited_work_tree="${fixture}/inherited-worktree"
    mkdir -p "${repository}/crates/yo-core/src"
    (
        export GIT_DIR="${inherited_git_dir}"
        export GIT_INDEX_FILE="${inherited_index}"
        export GIT_WORK_TREE="${inherited_work_tree}"

        # Git hooks export repository-local context such as GIT_INDEX_FILE. Clear it before
        # initializing the nested fixture so its commits and config cannot mutate the caller.
        local_env_vars=$(git rev-parse --local-env-vars)
        while IFS= read -r name; do
            unset "${name}"
        done <<<"${local_env_vars}"
        cd "${repository}"
        git init --quiet
        git config user.name "Developer Docs Test"
        git config user.email "docs-test@example.invalid"
        printf '%s\n' "obsolete" >crates/yo-core/src/obsolete.rs
        git add crates/yo-core/src/obsolete.rs
        git commit --quiet -m "test: seed deletion fixture"
        git rm --quiet crates/yo-core/src/obsolete.rs
        printf '%s\n' "refactor(core): remove obsolete runtime module" >message
        if bash "${checker}" message >/dev/null 2>&1; then
            echo "expected staged code deletion to require an impact trailer" >&2
            exit 1
        fi
    )
    if [[ -e "${inherited_git_dir}" ||
        -e "${inherited_index}" ||
        -e "${inherited_work_tree}" ]]; then
        echo "nested Git fixture mutated inherited repository state" >&2
        exit 1
    fi
}

expect_pass \
    "docs-only changes need no impact trailer" \
    "docs: explain the runtime" \
    "docs/src/architecture/runtime-flow.md" \
    "develop"

expect_fail \
    "code changes cannot omit the trailer" \
    "refactor(core): move runtime ownership" \
    "crates/yo-core/src/runtime/mod.rs" \
    "develop"

expect_fail \
    "updated requires a Developer Docs change" \
    $'refactor(core): move runtime ownership\n\nDeveloper-Docs-Impact: updated' \
    "crates/yo-core/src/runtime/mod.rs" \
    "develop"

expect_pass \
    "updated accepts a staged Developer Docs change" \
    $'refactor(core): move runtime ownership\n\nDeveloper-Docs-Impact: updated' \
    $'crates/yo-core/src/runtime/mod.rs\ndocs/src/architecture/runtime-flow.md' \
    "develop"

expect_fail \
    "none requires a concrete reason" \
    $'fix(tui): correct a typo\n\nDeveloper-Docs-Impact: none' \
    "crates/yo-tui/src/lib.rs" \
    "develop"

expect_pass \
    "none accepts a concrete reason" \
    $'fix(tui): correct a typo\n\nDeveloper-Docs-Impact: none - exported responsibilities and runtime flow are unchanged' \
    "crates/yo-tui/src/lib.rs" \
    "develop"

expect_fail \
    "multiple trailers are ambiguous" \
    $'fix(tui): correct a typo\n\nDeveloper-Docs-Impact: updated\nDeveloper-Docs-Impact: none - no flow change' \
    $'crates/yo-tui/src/lib.rs\ndocs/src/architecture/overview.md' \
    "develop"

expect_pass \
    "working Slice commits defer the decision to the accepted squash commit" \
    "refactor(core): move runtime ownership" \
    "crates/yo-core/src/runtime/mod.rs" \
    "slice/direct/runtime-ownership"

expect_staged_deletion_fail
