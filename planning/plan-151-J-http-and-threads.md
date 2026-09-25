# plan-151-J: Protocols, worker lifecycle and retained callbacks

Last updated: 2026-09-23
Effort: large (3h–1d); planning estimate for source review, metadata and rendered verification.
Depends on: plan-151-I; complete and verify it before starting this letter.

Populate reviewed per-overload costs for http, thread. The outcome is complete,
accurate Cost output for this package group, using the final schema in
[plan-151-A](plan-151-A-cost-schema-and-rendering.md). This letter does not implement
optimizations or silently expand a builtin's guarantees.

## Prerequisites

All A prerequisites apply. The preceding letter [plan-151-I-network-transports.md](plan-151-I-network-transports.md)
must have no unfinished tasks and all Commit/acceptance records filled. Check its
live or `planning/completed/` path with `rg -n '^- \[ \]|^- \[~\]|^Commit:'`;
no unchecked/partial task and no `Commit: —` may remain. At authoring this dependency
is NOT MET because the preceding letter is unimplemented. Recheck before executing;
statuses are snapshots. No fallback implementation of the preceding letter here.

## 1. Goal and non-goals

Every public overload in this group's freshly compiled census has a reviewed cost
case set; actual single-page and package-all output exposes the correct cases.
Do not alter signatures, bodies, error sets, overload ordering, runtime behavior or
the settled schema. Per-function evidence lives with its cost entry; timings stay
out of the registry. Numerical costs must follow current code, not planned work.

## 2. Current state and measured scope

Files: `src/codegen/builtins/http/`, `src/codegen/builtins/thread/`, plus helpers reached from their registrations. Evidence population:
21 literal `RegistryFunction {` occurrences in this group, split
http=19, thread=2. These are EDITING-SCOPE counts,
not expanded public function counts; helper-built functions require registry census.
Reproduce this row (same algorithm used when authoring):

```sh
python3 - <<'COUNT'
from pathlib import Path
for pkg in ['http', 'thread']:
    print(pkg, sum(p.read_text().count('RegistryFunction {')
                   for p in Path('src/codegen/builtins', pkg).rglob('*.rs')))
COUNT
```

Before annotating, run `MFB_COST_PACKAGES=http,thread MFB_COST_REQUIRE_COMPLETE=1 cargo test --bin mfb cost_census -- --nocapture`; it must identify missing rows, not pass an empty
selection. Record current expanded rows and any growth since A in the review ledger.
Bounds are UNVERIFIED at authoring; auditing them is this letter's work.
Relevant spec contracts (read alongside implementation):

- src/docs/spec/stdlib/05_http.md
- src/docs/spec/language/16_threads.md
- src/docs/spec/threading/08_queue-semantics.md

## 3. Design and risk

Use A's schema and work boundary unchanged. Review every overload, conditional type,
source shape and platform path; common helper use alone does not prove equal cost.
Provider-dependent work requires a named actual provider. Arbitrary input length is
not NoFiniteBound. A callback/worker can run indefinitely without making the local
builtin loop itself computationally unbounded. Do not omit copying or cleanup from
the local work. Cost claims may be conservative, but must remain useful and explain
what controls the bound.

Man output changes are intentional. Generated program artifacts are expected not
to change. No per-letter full-suite or byte-identity differential; L runs the final
regression gates. Any unexpected generated diff triggers localization and repair.

## 4. Review record and benchmark relationship

Append one row per public overload/case to `planning/plan-151-cost-review.md`:
package, member, full signature (not only ordinal), condition, work/space/wait/callback
conclusion, source file:symbol evidence, command/probe and result, and benchmark
coverage or an explicit gap. A row with an unresolved cost is unfinished, not a
ProviderDependent placeholder. Record why fast-path and general cases cover all calls.

Consult benchmark/mfb/src/main.mfb and existing local HTTP/thread runtime fixtures. Existing measurements can challenge a claim but cannot prove an
asymptotic bound. No blanket benchmark rerun and no benchmark format change. If a
claim needs a one-off scaling/copy-count probe, place it in `/tmp`, record the full
repro in the ledger, and cap inputs/wall time. Instrumentation is a probe, not a
production change. Keep internal proof prose out of rendered man descriptions.

## Phases

Keep task boxes current with implementation commits; record evidence and corrections.

### Phase J1 — Review and populate the group

- [ ] Audit thread helper factories and runtime queue paths; record data copy size, queue-retained storage, worker initialization, launch versus execution, transfer and accept. Handle parent/worker overloads and resource-plane variants explicitly.
- [ ] For send/receive/transfer/accept/poll/waitFor/cancel, distinguish a deadline, a cancellation request, worker completion and queue progress. Never describe cancellation as a bounded join.
- [ ] For HTTP clients, include parsing/serialization, response sizes, redirects/retry loops if present, and DNS/connect/TLS/body-read deadline phases. For server helpers, distinguish request processing from long-lived serving.
- [ ] Mark http::route handler as RetainedForLater; inspect func_handle_request.rs and helper_invoke_handler.rs for indirect handlers inside routes, invocation counts and failure behavior. Parameter root validation must accept callable-containing data rather than require the root type itself to be FUNC.
- [ ] Add `Some(Cost { ... })` per public overload in the scoped descriptors;
      pass explicit cost arguments through this group's factories. Reuse static
      records only after proving all reused overloads share their conditions.
- [ ] Fill the per-case ledger; use actual source evidence, including runtime and
      optimizer callees where relevant. Update existing owning spec topics only
      for newly documented internal explanations; do not duplicate cost tables.
- [ ] Remove duplicated performance prose from these functions' Description fields
      when Cost now owns it, preserving semantic statements and useful cross-links.

Acceptance: every currently registered public overload in the group is populated
and schema-valid, with case evidence and no unknown cost disguised as a bound.
Check: `MFB_COST_PACKAGES=http,thread MFB_COST_REQUIRE_COMPLETE=1 cargo test --bin mfb cost_census -- --nocapture` → no missing public overloads, nonzero denominator, exit 0
(estimate 5–10 min including incremental compilation).
Commit: —

### Phase J2 — Real output and ratcheted coverage

- [ ] Add the package names to A's temporary completed-package coverage set; add
      targeted real-registry render assertions under `src/cli/man.rs` with a
      `cost_` name for the group's distinct cases. Existing semantic/golden tests
      remain unchanged; no new language fixture is needed for metadata alone.
- [ ] Build and render each function via `target/debug/mfb man <package> <function>`
      and each package via `--all`; inspect named representative pages: thread send; thread waitFor; http route; http handleRequest.
      Store observations in the ledger. Check all rows via the renderer's registry
      walk, not top-level man --all, which omits general/testing.
- [ ] Review rendered Cost text against `.ai/man-content.md`, including no evidence
      paths, internal symbols, implementation jargon, or allocation vocabulary.
      Existing unrelated page findings do not authorize weakening a test/golden;
      record and resolve relevant prose without changing language implementation.

Acceptance: these real pages render their reviewed costs with correct overload
numbers, case conditions and no internal evidence text.
Check: `cargo test --bin mfb cost_` → schema, coverage-ratchet and rendering tests pass
(estimate 5–10 min).
Commit: —

## Compatibility / Format Impact

Cost sections and any deduplicated performance prose are the only expected user
output changes. Runtime/generated output and package formats remain unchanged.

## Validation Plan

- `cargo build --bin mfb` → current renderer binary (estimate 5–10 min).
- `MFB_COST_PACKAGES=http,thread MFB_COST_REQUIRE_COMPLETE=1 cargo test --bin mfb cost_census -- --nocapture` → complete group with actual expanded denominator (estimate 5 min).
- `MFB=./target/debug/mfb scripts/man-census.sh --memory-scope http thread` →
  no newly introduced unclassified hits; inspect all changed Cost sections and
  document baseline findings separately (estimate 2 min).
- Actual `mfb man` renders above are the runtime proof of the requested feature.
  No remote execution is needed for text-only changes; any platform-specific cost
  claim must cite that platform's implementation, and unsupported timing claims are
  removed rather than extrapolated from the host.
- Full suite and acceptance: once in L. If source helpers/bodies change unexpectedly,
  localize the change; metadata is not permission for runtime refactoring.

## Open Decisions

None. Per-member bound verification is required work under the settled schema.

## Corrections

None at authoring. Record source growth, disproved costs and effects on later letters.

## Summary

This letter turns reviewed source behavior into reader-facing cost contracts for its
package group; the coverage ratchet prevents losing those contracts in later work.
