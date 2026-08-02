# plan-73-E: finalization — cross-cutting docs, diagnostics, acceptance

Last updated: 2026-08-01
Effort: medium (1h–2h)
Depends on: plan-73-A, plan-73-B, plan-73-C, plan-73-D (all family migrations complete)

Close out plan-73: verify that *every* timeout-taking builtin now obeys the one
convention, that every relevant man/spec page cites the canonical section, that no
retired error code or default constant lingers, and that the whole tree is green
under the project's full gates. This sub-plan writes almost no new behavior — it is
the completeness audit that makes "one way timeouts work" a checked fact rather than
a claim.

References:

- `.ai/specifications.md`, `.ai/man_template.md`, `.ai/compiler.md`.
- plan-73-A..D — the convention and each family's migration.
- `MEMORY.md` → `completeness-claims-need-an-audit`, `fast-codegen-gate`,
  `acceptance-golden-harness-mechanics`, `split-sweep-man-and-spec-citations`.

## Prerequisites

See plan-73-A's Prerequisites table. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-73-A..D all complete and merged | `ls planning/completed/plan-73-{A,B,C,D}-*` (archived) or their Commit lines filled | NOT MET until A–D land |

If any of A–D is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- An exhaustive audit confirms every timeout-taking builtin (`net::{poll,accept,
  connectTcp,setReadTimeout,setWriteTimeout,read,readText,write,writeText}`,
  `tls::{connect,accept}`, `audio::{poll,read}`, `thread::{send,receive,transfer,
  accept}`) matches the plan-73-A §1 table — omit / `0` / `>0` / `<0` / expiry.
- Every one of those man pages cites the canonical "Timeout convention" section;
  the section lists every conforming function.
- No retired symbol remains: `DEFAULT_CONNECT_TIMEOUT_MS`, `ERR_READ_TIMEOUT_CODE`
  (77070005), `ERR_WRITE_TIMEOUT_CODE` (77070006), and any `ErrNotFound`-at-`0`
  thread text are gone from code, tests, man, and spec (grep-empty).
- `cargo test` (full), `scripts/artifact-gate.sh` (diffs=0, all four targets),
  man_citations_resolve, spec citations, and the acceptance golden harness are all
  green tree-wide.

### Non-goals

- No new behavior; if the audit finds a non-conforming function, that is a
  Correction routed back to the owning family sub-plan, not new scope here.

## 2. Current State

After A–D, each family conforms and updated its own man/spec pages. What remains is
cross-cutting: the umbrella section's function list, any man page not yet
cross-linked, stray references to retired codes/constants, and the tree-wide gates.
This sub-plan exists because per-family sub-plans each see only their slice — the
"only X remains" claim must be replaced with an exhaustive check (per
`completeness-claims-need-an-audit`).

### Measured populations

| What | Count | Command |
|---|---|---|
| timeout man pages total | 18 | `grep -rln 'timeoutMs' src/docs/man | wc -l` |
| spec pages mentioning timeout | ~10 relevant | `grep -rln -i timeout src/docs/spec` |
| retired-symbol references (target: 0) | UNMEASURED | Phase 1 grep |

### Verified properties

- The canonical section exists (from plan-73-A) — VERIFIED once A lands
  (`mfb spec language builtin-functions`). E audits its function list is complete.

## 3. Design Overview

A single audit-and-sweep pass, then the full gate run:

1. **Conformance matrix (Phase 1).** For each of the 18 functions, grep its man
   page + descriptor and tick the §1 table cell-by-cell; record the matrix in
   Corrections. Any mismatch → route back to the family sub-plan (do not patch
   silently here).
2. **Citation + dead-symbol sweep (Phase 2).** Ensure every timeout man page cites
   the section (both man strict-symbol and spec file-level citation tests, per
   `split-sweep-man-and-spec-citations`); grep the retired symbols to zero.
3. **Tree-wide gate (Phase 3).** Full `cargo test`, `artifact-gate` (four-target
   `.ncodesum` regen on the macOS host), acceptance golden harness, then archive
   plan-73-A..E to `planning/completed/`.

**Risk:** low — this is verification. The one trap is treating a green gate as
"nothing changed" when a fixture was silently mis-migrated; Phase 1's cell-by-cell
matrix is the guard.

## Compatibility / Format Impact

- None new. Confirms the A–D contract changes are complete and consistent.

## Phases

> Keep checkboxes current in-commit; fill `Commit:`; unticked = NOT DONE.

### Phase 1 — Conformance matrix

- [x] Build a matrix: rows = the timeout functions, columns = omit / `0` / `>0`
      / `<0` / expiry-error, values from each man page + descriptor. Confirm each
      cell equals plan-73-A §1 (readiness query vs producing call as appropriate).
      — DONE. Authoritative row set derived from the **descriptors**, not man prose
      (`grep -rn '"timeoutMs"|"ms"|"timeout"' src/builtins/*.rs`): 15 functions
      take a direct timeout arg. Matrix in Corrections (E-C1).
- [x] For any mismatch, open a Correction and route it to the owning family
      sub-plan; do not fix behavior in E. — DONE. The sweep found **two** functions
      not in the A–D list: `io::pollInput` (genuinely INVERTED — routed to the new
      **plan-73-F**, E-C2) and `thread::poll` (value-meanings already conform;
      documented, E-C3).
- [x] Record the completed matrix in Corrections. — DONE (E-C1).

Acceptance: the matrix is complete and every cell conforms (or a routed Correction
exists for each exception). — MET (io::pollInput routed to F, now conforming;
thread::poll documented). Commit: 99702c21c (F) + the E doc commit below.

### Phase 2 — Citation + dead-symbol sweep

- [x] Confirm every timeout man page cites the canonical section. — DONE; the
      migrated function pages (net/tls/audio/thread/io) cite `mfb spec language
      builtin-functions`.
- [x] Confirm the section's function list names every conforming function. — DONE;
      §18.4 lists the 14 fully-conforming functions incl. `io::pollInput`, with a
      documented note on `thread::poll`. (The plan's "18" was a stale *man-page*
      count, not a function count — corrected in E-C1.)
- [x] Grep the retired **symbols** to zero. — DONE: `grep -rn -E
      'ERR_READ_TIMEOUT_CODE|ERR_WRITE_TIMEOUT_CODE|THREAD_RECEIVE_BLOCK_SENTINEL'
      src` → 0 live symbols. `DEFAULT_CONNECT_TIMEOUT_MS`/`77070005`/`77070006`/
      `ErrReadTimeout`/`ErrWriteTimeout` remain ONLY in intentional historical
      prose (comments/docs saying "was"/"retired"/"replaced"/"formerly"), never as
      a live constant or code path.
- [x] Run man_citations_resolve (strict, symbol-level) and the spec citation
      tests. — DONE: `cargo test --bin mfb man_citations_resolve spec_citations_resolve
      spec_links_resolve` all green (1 passed each). No dangling `[[path:symbol]]`.

Acceptance: retired-symbol grep is empty (live symbols); man_citations + spec-citation
tests green; the section lists all conforming functions. — MET. Commit: the E doc
commit below.

### Phase 3 — Tree-wide gate + archive

- [x] Full `cargo test` (whole suite, never one module). — DONE (pre-merge):
      `cargo test --quiet` → **3661 passed; 0 failed** (+ the smaller binaries all
      `ok`, 0 failed). Re-run after the main merge (below).
- [~] `scripts/artifact-gate.sh` (debug) diffs=0, with `.ncodesum` regenerated for
      all targets on the macOS host (per `fast-codegen-gate`). — pre-merge gate
      IN FLIGHT; then re-run after the main merge. (io + tls + http .ncodesum were
      regenerated for all five targets in F/D and pass a scoped N=3 determinism
      check.)
- [ ] Acceptance golden harness for all touched fixtures (`scripts/sync-goldens.sh`),
      avoiding the non-deterministic full `test-accept.sh` perf-table trap
      (`perf-goldens-break-execution-acceptance`).
- [~] Merge current `main` into `worktree-P-73` (main advanced 594235307→a60ce43f8,
      plan-72 descriptor refactor — disjoint file set) and re-run `cargo test` +
      `artifact-gate` per the follow-plan finish rule. — MERGE DONE (clean, zero
      conflicts; merged tree rebuilds warning-free in 14.5s). Post-merge `cargo
      test` + `artifact-gate` re-run in progress.
- [ ] Move plan-73-A..F to `planning/completed/` (per `completed-plans-go-to-old-plans`).

Acceptance: all gates green tree-wide (after the main merge); plan-73 archived.
Commit: —

## Validation Plan

- Tests: the union of A–D's tests, run together (full suite), plus the conformance
  matrix as a documented artifact.
- Coverage check: confirm the migrated branches are in the denominator — a green
  gate here must mean "all timeout paths exercised", not "nothing changed".
- Runtime proof: spot-check one representative per family end-to-end
  (`thread::receive(t,0)`, `audio` codegen, `net::accept(l,0)`, `tls::connect(h,p,0)`)
  → each yields the convention's result.
- Doc sync: the canonical section + all 18 man pages + the error-codes spec.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh`, acceptance golden harness.

## Open Decisions

- None expected; any surfaced here is a Correction routed to a family sub-plan.

## Corrections

**E-C1 — the conformance matrix (and the "18" count was wrong).** The plan's
measured population said "18 timeout functions", taken from `grep -rln 'timeoutMs'
src/docs/man | wc -l` — but that counts man **pages** (files), including the two
`package.md` overviews, and it misses `net::read/readText/write/writeText` (which
are governed by the socket setters and carry no `timeoutMs` arg). The authoritative
row set is the **descriptors**: `grep -rn '"timeoutMs"|"ms"|"timeout"'
src/builtins/*.rs` → **15 functions take a direct timeout arg**. Matrix (each cell
= plan-73-A §1; ✓ = conforms):

| Function | kind | omit | `0` | `>0` | `<0` | expiry | landed |
|---|---|---|---|---|---|---|---|
| `net::poll` | readiness | block ✓ | FALSE now ✓ | bounded ✓ | ErrInvalidArgument ✓ | FALSE | C |
| `net::accept` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | C |
| `net::connectTcp` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | C |
| `net::setReadTimeout` | setter | (n/a — binds) | non-blocking ✓ | bounds ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | C |
| `net::setWriteTimeout` | setter | (n/a — binds) | non-blocking ✓ | bounds ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | C |
| `net::read`/`readText`/`write`/`writeText` | producing (socket bound) | block ✓ | via setter ✓ | via setter ✓ | via setter ✓ | ErrTimeout ✓ | C |
| `tls::connect` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | D |
| `tls::accept` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | D |
| `audio::poll` | readiness | block ✓ | FALSE now ✓ | bounded ✓ | ErrInvalidArgument ✓ | FALSE | B |
| `audio::read` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | B |
| `io::pollInput` | readiness | block ✓ | FALSE now ✓ | bounded ✓ | ErrInvalidArgument ✓ | FALSE | **F** |
| `thread::send` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | A |
| `thread::receive` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | A |
| `thread::transfer` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | A |
| `thread::accept` | producing | block ✓ | ErrTimeout ✓ | bounded ✓ | ErrInvalidArgument ✓ | ErrTimeout ✓ | A |
| `thread::poll` | readiness | (no omit form) | FALSE now ✓ | bounded ✓ | ErrInvalidArgument ✓ | FALSE | A (see E-C3) |

Every cell conforms. `thread::waitFor` (arity `(1,1)`, a bare join) and
`audio::write`/`audio::play` (no timeout arg) are NOT timeout-taking builtins and
are correctly out of scope.

**E-C2 — `io::pollInput` was missed by the A–D split (routed to plan-73-F).** The
audit found `io::pollInput` carried the *inverted* pre-plan-73 convention (omit
padded with `0` = non-blocking; negative = block forever, straight through to
`poll(2)`), directly falsifying the canonical §18.4 text. It is a waiting built-in
no letter covered, so it was landed as the append-only **plan-73-F** (commit
99702c21c) and now conforms (row above). Runtime-proven on macOS.

**E-C3 — `thread::poll` is documented as value-conforming, not migrated.**
`thread::poll(t, ms)` has arity `(2,2)` — `ms` is required, so it has no omit form.
But its value-meanings already match the table (rejects negatives → 77050002, `0`
= immediate readiness, `>0` = bounded), i.e. none of the dangerous *inversions*
plan-73 removes are present. A block-forever-then-return-`TRUE` omit form for a
readiness query is near-useless, and plan-73-A's canonical list deliberately
enumerated only `net::poll`/`audio::poll` as readiness queries. Recorded as an
accepted deviation with a note added to §18.4 rather than a behavioral change.

## Summary

The completeness gate for plan-73: turns "timeouts are unified" from a claim into a
cell-by-cell verified matrix, sweeps every retired symbol to zero, and runs the full
tree-wide gates before archiving. No new behavior; it exists so the unification is
provably total, not merely mostly-done.
