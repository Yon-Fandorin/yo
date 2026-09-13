# File mutation tool comparison

On 2026-09-13, the user-selected `qwen3.8-27b` and `qwen3.8-flash`
completed a real production-host comparison on candidate `06d3b4db`.
Both models used QwenCloud's general API, with valid per-model free quotas
of 1,000,000 tokens before execution and `Free quota only` enabled.
The Responses Token Plan key was not used. Provider cost fields were absent;
free admission is established by the authenticated quota safeguard, rather
than a fabricated zero-cost usage report.

## Method and boundary

Each model ran two tool shapes × three synthetic tasks × three repetitions:
18 cells, with shape order counterbalanced across tasks and repetitions.
Every cell used a fresh disposable workspace and compared exact final file
bytes, including files that must remain unchanged.

- **Distant edits:** change retry behavior and its label while preserving
  unrelated text in the same file.
- **New file:** create an exact two-line limits file without changing README.
- **Ambiguous target:** change only `syncUser`, preserving similar `syncTeam`
  text.

The basic shape exposed production `edit_file` and `write_file`. The
experimental structured `apply_patch` shape accepted Add/Update patch syntax
and translated it into the same production calls. Both used unchanged semantic
admission, registry and `LocalToolHost`; this does not establish an advanced
production tool, multi-file atomicity, or a managed-backend/TUI journey.

The exact endpoint was
`https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions`.
Both shapes used temperature 0, a 2,048-token output limit, `enable_thinking:
false`, first-round required tool selection and subsequent automatic selection.
Each cell allowed at most four requests, with a 72-request ceiling per model.
HTTP failures would stop the matrix; redirects, automatic retries, other models
and subscription fallback were disabled. A malformed-call correction within
the same bounded cell is counted as another model request.

## Observed results

Counts and token totals below cover nine cells per row. Time is the sum of
cell elapsed time, including transport and host execution; it is a small
sample under one host environment, not a latency guarantee.

| Model | Shape | Exact-byte passes | Requests / tool calls | Production calls | Malformed calls | Input / output tokens | Total tokens | Argument bytes | Time (ms) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `qwen3.8-27b` | Basic | 9/9 | 9 / 9 | 9 | 0 | 5,775 / 801 | 6,576 | 1,797 | 29,045 |
| `qwen3.8-27b` | Structured patch | 9/9 | 10 / 10 | 9 | 1 | 5,071 / 854 | 5,925 | 2,289 | 30,746 |
| `qwen3.8-flash` | Basic | 9/9 | 9 / 9 | 9 | 0 | 5,775 / 792 | 6,567 | 1,797 | 29,759 |
| `qwen3.8-flash` | Structured patch | 9/9 | 9 / 9 | 9 | 0 | 4,497 / 763 | 5,260 | 2,057 | 26,231 |

All 36 cells passed, with no production execution failures or missing token
usage. `27b` used 19 requests and 12,501 tokens; `flash` used 18 requests and
11,827 tokens. The combined campaign used 37 requests and 24,328 tokens.
The one malformed `27b` patch had invalid markers, caused no host mutation,
and was corrected by the next request in that cell.

For this matrix, patch syntax reduced total tokens by 9.9% for `27b` and
19.9% for `flash`, while increasing argument bytes by 27.4% and 14.5%.
It did not improve final-file correctness; `27b` also needed one extra call.
Retain the current basic tool surface. These results do not justify promoting
the experimental adapter or replacing basic editing. A future advanced
capability needs its own accepted contract and wider evidence.

Both models share QwenCloud. They are the requested two-model comparison,
not a third independent-provider sample. Historical virtual-workspace results
and interrupted OpenRouter trials are excluded from these aggregates.

## Validation identity

The live matrix used the candidate host identified below. Its original offline
symlink oracle incorrectly expected `write_file` to replace the link itself;
independent review rejected that expectation against the accepted nonregular
target contract. File-type bit containment admitted symlinks and sockets as
regular targets. The corrected host masks the complete type field and compares
it exactly with the regular-file type at target capture, descriptor admission
and scratch verification.

The new production-host regression reproduces the earlier failure and checks
regular-target, dangling and credential symlinks, directories, FIFOs and sockets.
Every attempt now fails with exact `unavailable` output while preserving entry
identity, mode, link destination and referenced bytes, with no scratch residue.
The corrected bridge passed all six offline task fixtures and ambiguity,
traversal, symlink-entry preservation, namespace, parser and free-admission
controls. The tools package passed 77 tests, with one ignored. That ignored
check is not counted as verified. The live matrix used regular files only;
its original results are retained under their original candidate and host
identity, without a live rerun or a claim about a newly tested model artifact.
All disposable cell workspaces were removed. No real API key entered the Rust
host; no credentials, raw transcripts or private reasoning are retained here.

Run the regression through the production host with:

```bash
cargo test --locked -p yo-cli write_file_rejects_nonregular_entries_without_replacing_them
```

The corrected offline host SHA-256 was
`fe5a1a90f861e57925492829c724a9541fe81367b630472a4290b57b27712f30`.

Fix candidate `1e98cba2` passed the full Linux CLI package: 544 tests passed,
31 ignored across its unit and integration groups. The same candidate passed
78 file-tool tests on the fingerprint-verified Apple Silicon Mac, with one
ignored, and CLI all-target Clippy. Its exact temporary checkout and Git cache
were removed. Formatting, test explanations, workspace all-target Clippy,
Unix compile checks and EN/KO documentation checks passed. The rebuilt stock
Yo SHA-256 was
`22cb8cb9587a5fd1eebbb31e67372f8d3dc3cb73324c5b4afdbb95daa7e898a0`.

Native Codex `0.154.0`, `gpt-5.6-sol` with effort `high`, independently
accepted the exact fix candidate in Session
`01a0999f-8e2a-7880-a45e-a917bd3941e1`. The chain contained one original
review and one same-Session finding resolution, with zero reviewer tool calls.
Reported review usage totaled 233,144 input tokens, including 72,192 cached,
and 9,191 output tokens across both invocations. The authenticated review host
is separate from the free service probes; no zero-cost review claim is made.
The temporary general API key and its helpers were removed after the service
checks. Accepted results are owned here; local experiment and review captures
are disposable caches.

The original live-matrix host SHA-256 was
`160e6367da5feb60a32ae420885ee0f979433ae50239b3ff2a31cc2e86fa9a51`.
Frozen runner, bridge and fixture source SHA-256 values were respectively
`57110c6a5e3f1bb206e23bd9459d400fe37451c0fff8546ac22ee09a65dc6dcf`,
`363a46868ecd2dfce21be7baa975630bbba496b9c517fd3304325fb775d9c4cf`,
and `615be4a57766bdabc25b4e490faeff788db9cd299635b4c483facc0400256781`.
