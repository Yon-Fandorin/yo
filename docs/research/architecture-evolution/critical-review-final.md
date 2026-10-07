# Final integrated delta review

> Status: non-authoritative independent documentation review

Verdict: **PASS for the reviewed documentation delta after corrections. No material finding remains.** All four findings below were corrected by the integrator and independently reread in the actual files. The integrated architectural direction is acceptable; no further design round or larger implementation scope is requested. This verdict is about the documentation candidate, not product readiness.

Correction recheck: implementation D now includes controller/intent/receipt semantics for the first supported local mutations, with a separate ACK-loss clarification; README assigns standalone/child Plan to its own Session and parent Goal/task funding to the parent; active F1 routing is updated to F1a/F1b; both erroneous backend links now point to managed; product F7 explicitly distinguishes display toggles from model reasoning/image-input control. The following findings are retained as resolved review history, not outstanding work.

Reviewed the new critical-review and three lane reports, revised README/implementation/usability/review-index, and task-relevant retained architecture sections. All 36 domains are present. The eight architecture findings are materially reflected: product-owned prompt assembly, concrete structured-process route, honest scheduler benefit, print as an existing neutral consumer, staged worker waits, common dispatch gate, independent native plans, and a useful MCP compatibility witness. Product/operational reports distinguish their static review from the root's separate CLI probes. Current feature/proposal/experimental/unproven distinctions are generally maintained. No product/contract/test/build/network work was performed during this review.

## F1 — The first-client milestone accidentally defers its own control-safety requirements

**Location:** `implementation.md:34`, D row: `trusted local client vertical 우선, 후속 WS remote/controller/intent/artifact`.

This wording postpones controller and intent semantics together with remote transport. But the same row requires two clients on one Session, and critical-review-service S7 requires explicit control transfer and submit ACK loss with one execution. S7 correctly says durable intent/receipt semantics precede mutation. A trusted local transport does not remove two-controller races or delivery uncertainty.

**Required correction:** State that first D includes controller/operation identity and the applicable intent/receipt subset for its supported mutations. Only WS/browser transport and later artifact/optional capability breadth move to subsequent delivery. Preserve the existing exact-control/unknown/no-resend invariants. No requirement to build every future control-plane API in D0.

## F2 — F1a is introduced, but live dependency/owner references still bind Plan to Goal/parent state

**Locations:** `README.md:521` (Goal/Plan/TaskLink/BudgetReservation authority all described as parent worker), `README.md:548` (`F1/F2` control surface); `review-index.md:106,113` (R17 `F1/G`, R24 `F1/F2` and no native-plan work); `implementation.md:39` (`F1/F2` shorthand).

The new table correctly separates F1a native Plan, F1b hard Goal budget, and F2 funded children. The remaining normative text still reads as if every Plan needs a parent/Goal and the old F1 stage still exists. This weakens the very dependency removal being adopted. Historical reviewer findings can retain old F1 wording as historical evidence; active routing and ownership should not.

**Required correction:** Define a standalone native Plan as owned by its own Session worker/Journal, without a Goal or parent prerequisite. For F2-funded tasks, parent worker owns the parent Goal/task link/budget, while each Session's plan has its explicitly chosen owner. Change Goal/task control references to F1b/F2, include F1a's separate native plan mutation/projection in R24, and map R17's hard-permit dependency to F1b. Replace current-stage ambiguous F1 shorthand or explicitly define it as the aggregate F1a+F1b.

## F3 — One source link points at the wrong owner despite existing on disk

**Location:** `critical-review-architecture.md:60`, link labeled `backend.rs::TurnState` targets `../../../crates/yo-core/src/backend.rs`.

The cited singular tool/approval TurnState is in `crates/backends/managed/src/backend.rs`. The current link lands at the neutral core backend boundary and does not substantiate the claim. A path-existence check cannot catch this.

**Required correction:** Link to `../../../crates/backends/managed/src/backend.rs`. No source change is required.

## F4 — Settings evidence blurs hiding reasoning/images with changing model capability

**Location:** `critical-review-product.md` F7, current-state sentence and acceptance (`이미지 off·reasoning off`).

The cited actual `crates/yo-cli/src/state/config/parse.rs::TuiConfig` has `show_images` and `show_reasoning` (lines 78/80), mapped to OutputPreferences. These are presentation switches. They do not establish a config.yaml switch disabling model reasoning computation or image input capability. The report combines this with model capability/settings discussion without that distinction.

**Required correction:** Name the implemented settings `이미지 표시` / `추론 표시` and use display-off in the acceptance. If changing model reasoning effort/image capability is also proposed, name that as a separate admitted model-profile feature with its own current/default/replacement semantics. Do not count display settings as existing model execution control.

## Selected process route and completion limits

The selected first route is sufficiently concrete: implementation C says versioned structured process tool; its body identifies exec_process-style executable/literal argv/cwd, and the main review explicitly chooses it. Existing fixed-wrapper language is acceptable as a documented alternative only; it must not silently replace that selected route or retain its broader automation acceptance claim. Existing shell remains explicit approval and exclusive when effect independence cannot be proven.

The pending main §8 final-review status may now record this completed independent correction recheck. No broad parity, OS sandbox, provider auth, performance or installation success claim can be inferred from this document review.

검토한 최종 설계와 실행 근거는 [전체 비판 검수](./critical-review.md)에 있다.
