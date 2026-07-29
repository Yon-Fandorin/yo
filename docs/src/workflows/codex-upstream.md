# Follow Codex app-server upstream

Use this workflow when an installed Codex minor line is rejected, an upstream
schema or event changes, or the adapter must deliberately move to a newer
app-server. This is operational validation guidance, not a second owner for the
adapter contract.

The exact admitted minor lines live beside the compatibility check in
[`protocol.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/protocol.rs).
The behavioral boundary remains owned by the
[Codex app-server KnowledgeUnit](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md).
Do not copy the current version set into this guide.

## Gate before admission

A newer installed executable is a candidate, not evidence of compatibility.
Keep an unknown minor line fail-closed until all applicable gates pass:

| Gate | Establishes | Does not establish |
|---|---|---|
| Official docs and release inspection | The documented lifecycle and announced changes | Compatibility with yo's private adapter |
| Version-specific schema inspection | The candidate's exact wire shapes | Runtime ordering or authenticated behavior |
| Deterministic adapter tests | Parsing, correlation, mapping, and failure behavior | Compatibility with the installed process |
| Installed initialize test | Real stdio handshake and cleanup | A completed coding Turn |
| Installed coding-loop test | Real Turn, tool, file-change, event, and cleanup flow | Other hosts or terminal routes |
| TUI smoke test | The user can enter, submit, observe, and exit | macOS, SSH, or nested tmux unless run there |

Do not replace these gates with a permissive minimum-version comparison.
Retain an older verified line unless removal has its own compatibility evidence
and review.

## Inspect the candidate

Record the candidate executable and read the
[official app-server documentation](https://developers.openai.com/codex/app-server)
and [official Codex releases](https://github.com/openai/codex/releases). The
app-server documentation states that generated schemas are specific to the
Codex version that produced them.

Generate the candidate schema outside the repository:

```bash
schema_dir="$(mktemp -d)"
codex --version
codex app-server generate-json-schema --out "$schema_dir"
```

Compare only the wire surface yo consumes:

- `initialize` and `initialized`;
- `thread/start`, `turn/start`, `turn/steer`, and `turn/interrupt`;
- Turn and Item lifecycle notifications;
- text, command-execution, and file-change updates;
- approval server requests and responses; and
- process shutdown and transport closure.

An additive field is not automatically safe, and a large unrelated schema diff
is not automatically blocking. Trace each relevant difference through
`backend/codex`, its deterministic tests, and the provider-neutral event it
produces. Keep generated schemas as disposable evidence unless a separate
Slice demonstrates that a tracked schema is needed.

## Run evidence in layers

First update or add deterministic fixtures without admitting the candidate
minor line. Cover both the expected flow and a discriminating failure before
changing the allowlist:

```bash
cargo test -p yo-core backend::codex::tests
cargo test -p yo-core backend::codex::protocol::tests
```

Then run the real initialization boundary with the candidate installed:

```bash
cargo test -p yo-core \
  backend::codex::tests::local_codex_initializes_and_shuts_down \
  -- --ignored --nocapture
```

When authentication and model access are available, verify the complete coding
loop in its disposable workspace:

```bash
cargo test -p yo-core \
  agent_session::tests::codex::local_codex_completes_a_real_file_change \
  -- --ignored --nocapture
```

Finally build `yo-cli` and perform one TUI smoke run in an applicable terminal
route. Submit a response-only prompt, observe the completed response, and exit
from an empty prompt. If the compatibility change affects a terminal route,
run the corresponding [terminal matrix](../validation/terminal-matrix.md)
command instead of inferring it from a local shell.

## Admit or reject the line

Admit the minor line only after the relevant evidence passes:

1. Add the exact minor line to the compatibility set.
2. Keep the previous verified lines unless deliberately retiring one.
3. Make the positive test exercise every admitted line.
4. Make the negative test use an actually unverified line.
5. Keep malformed or unknown versions fail-closed with an actionable error.
6. Run the [Slice-close baseline](../validation/#slice-close-baseline).
7. Obtain fresh-context review because provider compatibility changes failure
   behavior at a product boundary.

If a gate fails, do not widen the version range. Identify the first changed
wire owner, update the private adapter and deterministic evidence, then rerun
the real boundary. Record unavailable authentication, host, or terminal routes
as unverified rather than passed.

## Report the verification

Keep the accepted commit or review packet concise and reproducible:

```text
Candidate Codex version:
Official docs or release inspected:
Relevant schema differences:
Deterministic commands and results:
Installed initialize result:
Installed coding-loop result:
TUI or environment route:
Unverified cases:
Review result:
```

The commit records evidence, while code and Methexis retain authority. Do not
add a permanent compatibility log or duplicate the admitted version set in
documentation.

## When to extract a skill

Keep this as a guide while compatibility work still requires judgment about
wire relevance and evidence. Extract a repository skill only after multiple
upstream updates repeat the same safe mechanics, such as schema generation,
focused diff extraction, command execution, and report formatting. The skill
should automate those mechanics and route decisions back to this guide and the
owning KnowledgeUnit; it must not approve a version by itself.
