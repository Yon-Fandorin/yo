# Contributing to yo

Make tracked changes in reviewable slices. Squash only human-approved slices into `develop`. This file is the repository workflow authority.

## Branches

| Branch | Role |
| --- | --- |
| `main` | Releasable state |
| `develop` | Integration point for approved work |
| `<type>/<scope>` | One reviewable slice |

Branch from current `develop`. Use `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, or `chore/` followed by a short purpose, such as `feat/tui-surface`.

Use `hotfix/*` only for a released `main` emergency. Use `spike/*` for disposable investigation. Do not use `agent/*` by default; name the purpose, not the worker.

## Slices

One branch owns one observable outcome. Split work that can be approved or reverted independently.

Include the required implementation or docs, discriminating validation, public-contract updates, and known limits. Exclude unrelated cleanup. A reviewer must be able to assess the purpose, contract, and failure behavior together.

## Workflow

```bash
git switch develop
git switch -c <type>/<scope>
```

Commits may preserve useful review steps. Present the outcome, evidence, and limits, then wait for explicit human approval.

```bash
git switch develop
git merge --squash <type>/<scope>
git commit
```

The squash commit describes the approved outcome. Delete the slice branch when no longer needed.

Squash an approved `hotfix/*` into `main` and carry it into `develop`. Discard `spike/*`; reimplement accepted findings on a new branch from current `develop`.

## Local checks

Install the version selected by `hk.pkl`, then register its repository-local hooks:

```bash
cargo install hk --version 1.52.0 --locked
hk install
```

`hk.pkl` owns the hook set. `hk check` verifies changes without editing them; `hk fix` applies available fixes. Git `pre-commit` runs checks only.

## History boundary

Treat `rib` as read-only. Keep audits, comparisons, and disposable prototypes in `.local-exclude/`; never force-add it. Track only independently rewritten and reviewed results.

Do not rewrite shared history or force-push without explicit approval.

## Merge gate

Request approval only when:

- the diff contains one agreed slice;
- relevant tests, documentation checks, and `git diff --check` pass;
- tracked files contain no `.local-exclude/` content or `rib` copies; and
- the outcome, evidence, and limits are ready for review.

Approval applies only to that slice. Follow-up work requires a new branch and review.
