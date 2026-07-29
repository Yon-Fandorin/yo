#!/usr/bin/env bash
set -euo pipefail

readonly checker="$(pwd)/tools/validation/slice-review-impact.sh"
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
        echo "expected Slice review impact case to fail: $1" >&2
        exit 1
    fi
}

expect_fail \
    "accepted commits cannot silently omit review disposition" \
    "docs: clarify the code map" \
    "docs/src/architecture/overview.md" \
    "develop"

expect_pass \
    "ordinary docs may explain why no additional lens applies" \
    $'docs: clarify the code map\n\nSlice-Review: none - wording only; documented ownership is unchanged' \
    "docs/src/architecture/overview.md" \
    "develop"

expect_fail \
    "production code requires fresh-context review" \
    $'fix(tui): restore the terminal\n\nSlice-Review: none - tests pass' \
    "crates/yo-tui/src/terminal/mode/transaction.rs" \
    "develop"

expect_pass \
    "production code accepts concrete fresh-context evidence" \
    $'fix(tui): restore the terminal\n\nSlice-Review: fresh-context - reviewer terminal-ops found no unresolved findings' \
    "crates/yo-tui/src/terminal/mode/transaction.rs" \
    "develop"

expect_fail \
    "tool configuration cannot evade review through an unlisted extension" \
    $'build(sot): revise tool dependencies\n\nSlice-Review: none - configuration only' \
    "tools/methexis/Cargo.toml" \
    "develop"

expect_fail \
    "public product orientation requires fresh-context review" \
    $'docs: revise product scope\n\nSlice-Review: none - prose only' \
    "README.md" \
    "develop"

expect_fail \
    "workflow authority requires fresh-context review" \
    $'docs: revise workflow\n\nSlice-Review: none - prose only' \
    "CONTRIBUTING.md" \
    "develop"

expect_fail \
    "semantic SOT authority requires fresh-context review" \
    $'docs(sot): revise lifecycle\n\nSlice-Review: none - prose only' \
    "methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md" \
    "develop"

expect_fail \
    "Wave commits require integration review as well as fresh-context review" \
    $'feat(core): revise runtime\n\nSlice-Review: fresh-context - reviewer core-contract found no unresolved findings' \
    "crates/yo-core/src/runtime/mod.rs" \
    "wave/w1-runtime"

expect_pass \
    "Wave commits accept both required review lenses" \
    $'feat(core): revise runtime\n\nSlice-Review: fresh-context - reviewer core-contract found no unresolved findings\nSlice-Review: integration - reviewer wave-coordinator found no sibling conflict' \
    "crates/yo-core/src/runtime/mod.rs" \
    "wave/w1-runtime"

expect_fail \
    "none cannot hide beside completed review evidence" \
    $'docs: revise workflow\n\nSlice-Review: fresh-context - reviewer workflow found no unresolved findings\nSlice-Review: none - no review needed' \
    "CONTRIBUTING.md" \
    "develop"

expect_fail \
    "unknown review spellings do not count as evidence" \
    $'docs: clarify the code map\n\nSlice-Review: approved - looks good' \
    "docs/src/architecture/overview.md" \
    "develop"

expect_fail \
    "review-looking prose outside the trailer block is not evidence" \
    $'docs: clarify the code map\n\nThe example Slice-Review: none - wording only is not this commit disposition.' \
    "docs/src/architecture/overview.md" \
    "develop"

expect_fail \
    "duplicate fresh-context evidence remains ambiguous" \
    $'fix(tui): restore the terminal\n\nSlice-Review: fresh-context - first reviewer passed\nSlice-Review: fresh-context - second value duplicates the lens' \
    "crates/yo-tui/src/terminal/mode/transaction.rs" \
    "develop"

expect_fail \
    "integration review alone cannot replace fresh-context review for code" \
    $'feat(core): revise runtime\n\nSlice-Review: integration - sibling integration passed' \
    "crates/yo-core/src/runtime/mod.rs" \
    "develop"

expect_fail \
    "Wave docs still require integration review" \
    $'docs: clarify the Wave\n\nSlice-Review: none - wording only' \
    "docs/src/architecture/overview.md" \
    "wave/w1-runtime"

expect_pass \
    "working Slice commits defer review evidence to the accepted commit" \
    "fix(tui): restore the terminal" \
    "crates/yo-tui/src/terminal/mode/transaction.rs" \
    "slice/direct/terminal-restoration"

expect_actual_git_path_detection() {
    repository="${fixture}/actual-git"
    mkdir -p "${repository}/tools/example"
    (
        cd "${repository}"
        git init --quiet
        git config user.name "Slice Review Test"
        git config user.email "slice-review@example.invalid"
        git switch -c develop >/dev/null 2>&1
        printf '%s\n' "obsolete" >tools/example/obsolete.sh
        git add tools/example/obsolete.sh
        git commit --quiet -m "test: seed deletion fixture"
        git switch -c wave/w1-review >/dev/null 2>&1
        git rm --quiet tools/example/obsolete.sh

        printf '%s\n' \
            "test: remove obsolete tool" \
            "" \
            "Slice-Review: fresh-context - deletion reviewer passed" >message
        if bash "${checker}" message >/dev/null 2>&1; then
            echo "expected actual Wave deletion to require integration review" >&2
            exit 1
        fi

        printf '%s\n' \
            "test: remove obsolete tool" \
            "" \
            "Slice-Review: fresh-context - deletion reviewer passed" \
            "Slice-Review: integration - Wave integration passed" >message
        bash "${checker}" message
    )
}

expect_clean_index_amend_detection() {
    repository="${fixture}/amend"
    mkdir -p "${repository}/tools/example"
    (
        cd "${repository}"
        git init --quiet
        git config user.name "Slice Review Test"
        git config user.email "slice-review@example.invalid"
        git switch -c develop >/dev/null 2>&1
        printf '%s\n' "reviewed code" >tools/example/check.sh
        git add tools/example/check.sh
        git commit --quiet -m "test: seed amend fixture"
        printf '%s\n' \
            "test: rewrite message" \
            "" \
            "Slice-Review: none - message-only amend" >message
        if bash "${checker}" message >/dev/null 2>&1; then
            echo "expected clean-index amend to retain the original review requirement" >&2
            exit 1
        fi
    )
}

expect_merge_commit_exemption() {
    repository="${fixture}/merge"
    mkdir -p "${repository}"
    (
        cd "${repository}"
        git init --quiet
        git config user.name "Slice Review Test"
        git config user.email "slice-review@example.invalid"
        git switch -c develop >/dev/null 2>&1
        printf '%s\n' "base" >README.md
        git add README.md
        git commit --quiet -m "test: seed merge fixture"
        git branch wave/w1-review
        printf '%s\n' "change" >CONTRIBUTING.md
        git add CONTRIBUTING.md
        git commit --quiet -m "test: advance develop"
        git switch wave/w1-review >/dev/null 2>&1
        git merge --quiet --no-ff --no-commit develop
        printf '%s\n' "Merge develop into Wave" >message
        bash "${checker}" message
    )
}

expect_unreviewed_wave_merge_is_not_exempt() {
    repository="${fixture}/unreviewed-wave-merge"
    mkdir -p "${repository}"
    (
        cd "${repository}"
        git init --quiet
        git config user.name "Slice Review Test"
        git config user.email "slice-review@example.invalid"
        git switch -c develop >/dev/null 2>&1
        printf '%s\n' "base" >README.md
        git add README.md
        git commit --quiet -m "test: seed unreviewed Wave merge fixture"
        git switch -c wave/w1-review >/dev/null 2>&1
        git switch -c slice/w1-review/unreviewed >/dev/null 2>&1
        mkdir -p tools/example
        printf '%s\n' "unreviewed" >tools/example/change.sh
        git add tools/example/change.sh
        git commit --quiet -m "test: seed unreviewed Wave Slice"
        git switch wave/w1-review >/dev/null 2>&1
        git merge --quiet --no-ff --no-commit slice/w1-review/unreviewed
        printf '%s\n' "Merge unreviewed Wave Slice" >message
        if bash "${checker}" message >/dev/null 2>&1; then
            echo "expected a true Slice merge into a Wave to require review" >&2
            exit 1
        fi
    )
}

expect_develop_merge_is_not_exempt() {
    repository="${fixture}/develop-merge"
    mkdir -p "${repository}"
    (
        cd "${repository}"
        git init --quiet
        git config user.name "Slice Review Test"
        git config user.email "slice-review@example.invalid"
        git switch -c develop >/dev/null 2>&1
        printf '%s\n' "base" >README.md
        git add README.md
        git commit --quiet -m "test: seed develop merge fixture"
        git switch -c slice/direct/unreviewed >/dev/null 2>&1
        mkdir -p tools/example
        printf '%s\n' "unreviewed" >tools/example/change.sh
        git add tools/example/change.sh
        git commit --quiet -m "test: seed unreviewed Slice"
        git switch develop >/dev/null 2>&1
        git merge --quiet --no-ff --no-commit slice/direct/unreviewed
        printf '%s\n' "Merge unreviewed Slice" >message
        if bash "${checker}" message >/dev/null 2>&1; then
            echo "expected a true merge into develop to require Slice review" >&2
            exit 1
        fi
    )
}

expect_actual_git_path_detection
expect_clean_index_amend_detection
expect_merge_commit_exemption
expect_unreviewed_wave_merge_is_not_exempt
expect_develop_merge_is_not_exempt
