# Independent omission audit — R01–R36

> 별도 fresh-context 검수자의 누락 감사 및 수정 후보 재검토 기록이다.
> 최종 판정은 이 보고서의 마지막 addendum과 [전체 검수표](./review-index.md)를 함께 따른다.

2026-10-05. Fresh-context, read-only audit of the three lane reports, the research candidate, and selected original source boundaries. This audit wrote only this report. No network, build, tests, model requests or subagents were used. Static test-body inspection is not a test pass.

Sources: Yo `880467b3186ac7ace0111acd37cb1aa334c9dc4c`; Pi `cd32f7725fdbddbaecdff5b1e68491563394e0ca` / v1.0.2; Codex `a956835d020762cb2b570053af06f643a11c0ecc` / rust-v0.160.0. Research documents are the changing working candidate, not an accepted contract. Repository source and external source trees were not edited.

Read AGENTS.md and CONTRIBUTING.md, research review-index, README, module-comparison, usability-comparison, implementation, and relevant evidence/layer-review sections; root README and relevant remote-host/continuation authority. Read `/tmp/yo-review-a.md`, `-b.md`, `-c.md`, including their explicit limits and corrected-candidate notes. Original-source reads below identify independently rechecked functions/sections; this is not a second full inspection of every source cited by those reports.

## Findings sent to author

### G1 — R07/R08/R14/R29/R31/R35: bounded ledger has no complete pressure→stop→recovery journey

**Material design gap, not an existing implementation defect.** Research README §8, paragraphs beginning `Ledger는 record encoded-byte` and `첫 초과에서`, specify 4,096 keys, no GC/reuse, and rejection of new dispatch once full. Existing operations reserve their own correlation space. They do not reserve the new record needed to interrupt a running Turn or decline a pending request. The user can therefore reach permanent pressure, and potentially cannot stop through the common service. `inspect/recovery` names an action without defining how safe usable capacity returns.

Independent baseline evidence: `crates/yo-core/src/agent_session/admission.rs:223–242` sends an observed request response/exact active interrupt through the urgent queue. `crates/yo-tui/src/runner/tests/backpressure.rs` contains tests that preserve exit and an observed approval response when the normal lane is full, and preserve request input while urgent retry is occupied. Both code/test bodies were read. A host admission layer that refuses all operations before this boundary defeats the existing stop behavior. Codex `codex-rs/tui/src/chatwidget/reconnect.rs:restore_reconnected_input` also shows why an old operation must not be resent merely to regain usability: exact receipt IDs distinguish confirmed and unconfirmed inputs.

**Minimum correction:** Reserve bounded authenticated stop/revoke/shutdown capacity per live Session before ordinary admission consumes capacity. Preserve authorization and exact target correlation; this is not an unlogged permission-expansion path. Provide explicit quiescent rotation with a durable monotonic operation epoch/retired-before watermark. All retired-epoch mutations must fail even after host restart; no old UUID becomes fresh work. Publish rotation crash-atomically, force client reinitialization, and report old operation status as retired/unavailable rather than success. Pending/unknown effects prevent rotation until explicit recovery; do not erase uncertainty or automatically redispatch. Separate host incarnation from the durable operation epoch.

The user journey must be pressure→inspect→quiesce/stop→explicit rotate→reinitialize→resume. Test full ordinary ledger while a Turn is running/approval is pending, first excess, exact stop after overload, crash around rotation commit, old-client replay before/after restart, and unresolved effects that refuse rotation. The data model can remain a small bounded ledger, without an unbounded archive or generic transaction database.

### G2 — R08/R21/R29/R34: saved reusable permission cannot yet be inspected/revoked through a defined user/API route

**Material user/owner connection gap.** Research README §5 defines `PermissionGrant`, explicit saved-rule choice, expiry and dispatch-time revocation. Implementation has `ToolPermissionPort::...revoke_generation` and revoke-race fixtures. Neither §5 nor §8's API groups identifies the user action and host operation for reviewing and revoking a saved rule later, or explains which grant caused automatic execution. A's request review stops at offered choices; B's R21 review stops at backend revoke semantics. This is the same cross-owner gap they correctly found for MCP management and Goal controls elsewhere.

Independent source evidence: Yo `crates/yo-core/src/tool/approval.rs` binds approval to exact Turn/call/tool/normalized args/effect/host and has no persistent-grant semantics. Codex `codex-rs/core/src/tools/sandboxing.rs:41–110` separates session cached decisions from call approval; `codex-rs/tui/src/slash_command.rs` exposes a Permissions entry. These are responsibility references, not proof that Codex implements the proposed Yo durable revoke semantics.

**Minimum correction:** C provides an illustrative `/permissions` list/read/revoke route for host/workspace-scoped grants, with lifetime/expiry/tool/resource/sandbox summaries and matched-grant provenance in tool details. Host permission store owns the mutation; exact grant ID, expected revision and authorized actor are required. Persistent revoke advances durable generation atomically and survives restart; controller revoke is explicitly a different operation. D exposes the same optional service operations. No general rule editor is necessary: widening requires a new explicit approval.

Consumer task: save rule→repeat permitted operation without redundant prompt→inspect why permitted→revoke while another matching call waits→next dispatch blocked/new approval→restart still revoked; stale revoke/unsupported profile preserves current draft. Report already-running-effect limits honestly.

### G3 — R29/R35: reviewer-noted host aggregate resource profile was not carried into the candidate

**Small integration omission / open implementation parameter.** C's R29/R35 explicitly says that attachment/client total limits must be finite before D. README §8 specifies per-attachment 128-message/4-MiB control queues and total upload reservations, but per-attachment bounds alone permit unbounded aggregate fanout. A finite Session count does not bound its observer count.

**Minimum correction:** Carry host-wide authenticated connection/attachment cardinality and aggregate queued-byte budget into D's explicit open numeric profile and acceptance list. Reject the first excess without consuming existing controllers' stop reserve. Values can remain marked to be fixed before D; no invented performance guarantee is needed. This is not a claim that current local Yo has an unbounded network server.

### G4 — R36: completion wording must retain inspection limits

**Reporting requirement, not a new product feature.** The index has all 36 rows and maps all previous 29 layers; no old layer disappeared in that mapping. Each lane report has row-specific source anchors, comparison/judgment and future checks. However, A repeatedly says representative/partial inspection; B and C enumerate narrower source and runtime limits. Old implementation-document statements that all previous reviews found no material issues refer to an earlier candidate/scope and cannot close the current R01–R36 audit.

Minimum correction: final status distinguishes (a) all rows assigned and reported, (b) material candidate findings corrected/rechecked, (c) source inspection gaps, (d) intentional deferrals, and (e) unrun implementation/acceptance tests. Preserve row-level evidence in durable research documents or an appropriately linked summary; ephemeral `/tmp` reports alone are not a future reader's evidence index. The author already indicated this correction is planned.

## Coverage audit of the reviewers

The following checks the reports' scope honestly; it does not assert I reread all their source paths.

| IDs | Report coverage and cross-check | Remaining inspection/execution boundary |
|---|---|---|
| R01–R03 | A traces installation guidance, pre-Session config failure and supported auth/cancel. C/R33 reads actual upstream installer/package routes. | Actual Yo artifact install and complete provider expiry/browser/device/SSH behavior remain unverified. A/C do not erase this by combining their reports. |
| R04–R06 | A has current/reserved/default semantics, exact workspace and typed reference/image/editor state; B preserves admitted catalog/resource ownership. Independently read Yo image host `bind`, `validate_images`, `Preparation::start`: inherited/current committed bytes reconstruct provenance; one preparation job is bounded. | Full upstream skill/template/image parity and actual IME/clipboard/editor behavior were not inspected/executed. Proposed remote uploads must preserve this existing input authority. |
| R07–R09 | A tests acceptance-preserved drafts, paused queues, offered choices and status sources; B/C preserve local/remote distinction. Independently read admission urgent routing and all three TUI backpressure test bodies. | G1 was missed by both the UI-pressure and ledger reviewers. Host retry/status producer coverage remains partial. |
| R10–R12 | A correctly preserves existing Changes/Output, bounded session catalog and current resume/fork eligibility; C separates public catalog privacy from raw history. | Full fork durability/lineage tests were not reread by A; actual terminal/assistive technology and ignored SSH tests remain unrun. |
| R13–R17 | B compares neutral factory, client ownership, private state and model/provider/connector axes; candidate does not merely rename/move files. B3 repaired cache-work accounting. | Every dependency feature, delegated adapter/version/dialect and auth path was not audited. No runtime/performance equivalence claim follows. |
| R18–R20 | B retains counted=sent=recorded snapshots, summary/private-request dispatch distinctions, closed tool schema and renderer separation. | Recursive child-directory instructions are explicitly deferred; this is not silently implemented. Full typed-search upstream consumer comparison and all codec/dialect branches remain partial. |
| R21–R24 | B repairs MCP management, Goal controls and cache provenance. Goal fresh seed/parent reservation/reconcile are distinguished from intentional fork. | G2 was missed between approval UX and grant-store semantics. Full OS confinement, MCP protocol/HTTP OAuth and multi-agent/worktree implementations were not verified; codemode is intentionally H-deferred. |
| R25–R28 | C identifies Journal authority, memory snapshot/CAS, secret/public catalog separation and self-contained images. Independently rechecked current image provenance and inspected relevant persistence/lineage authority. | Memory/upload/new schema are proposed; not current tested implementations. Secret export/provider lifetime and all filesystem/power-loss combinations remain partial. |
| R29–R30 | C distinguishes Pi ordinary RPC/experimental services, Codex native WS/browser boundary and commercial UI source unavailability. Independently read Codex reconnect exact confirmation and Pi experimental `connection.ts` generation/lifecycle setup. | G1/G3 join omissions; actual browser/remote/DOM accessibility/profile handoff remain unrun. Static Pi HTML export is not a live-controller implementation. |
| R31–R33 | C fixes mode-specific gap recovery and first-visible performance baseline, and separates compile/PTY/package/eval evidence. | No actual auth/model/OS/SSH/install/upgrade/rollback/performance run; rollback compatibility remains package/schema dependent. |
| R34–R36 | Proposed command names remain proposals, authority is separated, 29→36 reverse map is intact. | G4 final reporting and persistence of evidence are necessary. Assignment coverage is not exhaustive source verification. |

## End-to-end checks

- **Setup→model→typed input→approval→tool→result→resume:** candidate now links no-Session setup, safe config failure, exact catalog selection, preserved drafts/preparation, accepted request semantics, existing output/evidence and exact saved workspace. A's late query/missing workspace and secret-draft findings are useful acceptance additions. Saved-grant management G2 and ledger stop G1 were the missing cross-owner steps.
- **Task budget→child→memory→restart:** F1/F2 owner remains parent Journal, child is a fresh seeded Session, uncertain acceptance retains reservation, child terminal joins once, unavailable parent prevents funded continuation. Memory worker admission holds the scope guard only around its own immutable commit, after other-writer preparation. Worktree scope sharing and standalone transfer are intentionally deferred. No extra material contradiction was found in this inspected design path; the dispatch/lock/crash tests remain implementation work.
- **Remote/controller→upload→disconnect→reconnect:** service owns Session; attachment does not. Exact generation prevents late control, upload is provisional until admitted immutable image bytes, and receipt reconciliation prevents resend. Secret input is single-use volatile and cannot borrow ordinary operation hashing. G1 pressure recovery and G3 aggregate connections were left open between otherwise detailed boundaries.

## Review limits

This audit targeted omissions and cross-owner contracts. It is not a comprehensive security audit, all-source proof, independent rerun of every lane's static read, or runtime conformance report. No private commercial Codex desktop/web code was available. Source pin labels were supplied and compared with the research evidence, not refetched. Routine stylistic changes and unrelated CONTRIBUTING edits were out of scope and untouched.

At report creation G1–G3 were sent to the author and await corrected-candidate reinspection. G4 is a completion/reporting condition. A final addendum below should record which corrections were actually reread, rather than treating author acknowledgment as a completed review.

## Corrected-candidate reinspection

After author amendments, independently reread README §5's permissions management, §8 API table/operation example/ledger pressure and host aggregate bounds, implementation candidate interfaces and pressure/permissions/connection tests, usability §3/§7, and review-index §5. Verified permanent A/B/C report files exist under the research folder.

- **G2 resolved in design:** proposed list/read/revoke entry, matched bounded/redacted grant provenance, host store ownership, exact identity/revision/actor, persistent generation-CAS, queued dispatch revalidation, restart behavior and separate controller revoke are explicit. C can implement this in existing CLI composition; D exposes the same optional service. No broader rule editor added.
- **G3 resolved in design:** host-wide connection/attachment count and total queued bytes now have admission reservations, first-excess rejection and protection of existing controller/urgent capacity. Numeric values are explicitly D pre-implementation parameters, not runtime measurements.
- **A follow-up corrections rechecked:** general question/notes preservation excludes secret value retention; actual status/cancel/compaction differences are tasked without invented retries; missing workspace/late search has a recovery task; non-enhanced keyboard, mono and terminal restoration have acceptance requirements; ignored SSH/tmux tests are not treated as executed.
- **G1 substantially addressed, final clarification requested:** current candidate reserves exact interrupt/decline and host revoke/shutdown, defines crash-atomic quiescent epoch rotation, rejects retired mutations, reports retired status unavailable and prohibits resending old operations with a new epoch. Asked author also to reserve the fixed-size maintenance publication itself at ordinary capacity, return the durable operation epoch from initialize, require it for mutations, authorize expected-epoch rotation and preserve it across restart. This closes the selected mechanism; it does not require another design round.
- **G4 partly addressed, final status still pending:** review-index now preserves row-level partial inspection, execution limits, open numeric parameters and deferrals; A/B/C reports have durable links. Asked author to mark implementation's old 275-link/five-file final-check paragraph historical or replace it with current candidate checks, so old evidence is not attached to newly added files.

## Final verdict after last correction

Reread the final ledger paragraphs: fixed-size epoch/fence maintenance space is reserved before ordinary admission; initialize returns durable operationEpoch; mutations require it; host management authorization and expected-epoch CAS govern rotation; restart cannot reset the epoch. Together with exact urgent reservations, crash-atomic quiescent rotation, retired-epoch rejection and no automatic resend, **G1 is resolved in the inspected design**.

Reread implementation's final validation section: the earlier 275-path/three-anchor/five-file checks are now explicitly historical evidence for the earlier 29-layer candidate. Current R01–R36 representative inspection, limitations and forthcoming full-document checks are distinct. Permanent lane report links and the index's explicit inspection/execution/deferral/open-parameter categories satisfy **G4's reporting correction**. Root still needs to publish this final audit report, update its completed status, and execute/report the current document checks; this reviewer does not claim those checks have run.

**Final verdict: no remaining material finding in the inspected corrected design.** G1–G4 are resolved at the proposal/reporting level, and A's late follow-up acceptance corrections were independently reread. The 36-item index covers all prior 29 layer categories, and the three lane reports provide traceable representative evidence and limits. This verdict does not mean every source path was inspected or every implementation/UX contract was executed. The unrun implementation, provider/OS/terminal/browser/installation, performance and model-quality checks, partial source scopes, and finite D-profile choices above remain explicit future work.
