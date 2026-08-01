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

- [ ] Build a matrix: rows = the 18 timeout functions, columns = omit / `0` / `>0`
      / `<0` / expiry-error, values from each man page + descriptor. Confirm each
      cell equals plan-73-A §1 (readiness query vs producing call as appropriate).
- [ ] For any mismatch, open a Correction and route it to the owning family
      sub-plan; do not fix behavior in E.
- [ ] Record the completed matrix in Corrections.

Acceptance: the matrix is complete and every cell conforms (or a routed Correction
exists for each exception). Commit: —

### Phase 2 — Citation + dead-symbol sweep

- [ ] Confirm every timeout man page cites the canonical section; add missing
      cross-links (run `scripts/update_man.sh`).
- [ ] Confirm the section's function list names all 18.
- [ ] Grep to zero: `grep -rn -E 'DEFAULT_CONNECT_TIMEOUT_MS|77070005|77070006|ErrReadTimeout|ErrWriteTimeout' src tests src/docs`
      and any `ErrNotFound`-at-`0` thread wording.
- [ ] Run man_citations_resolve (strict, symbol-level) and the spec file-level
      citation tests; fix any dangling `[[path:symbol]]` in both man and spec.

Acceptance: retired-symbol grep is empty; man_citations + spec-citation tests
green; the section lists all 18 functions. Commit: —

### Phase 3 — Tree-wide gate + archive

- [ ] Full `cargo test` (whole suite, never one module).
- [ ] `scripts/artifact-gate.sh` (debug) diffs=0, with `.ncodesum` regenerated for
      all four targets on the macOS host (per `fast-codegen-gate`).
- [ ] Acceptance golden harness for all touched fixtures (`scripts/sync-goldens.sh`),
      avoiding the non-deterministic full `test-accept.sh` perf-table trap
      (`perf-goldens-break-execution-acceptance`).
- [ ] Move plan-73-A..E to `planning/completed/` (per `completed-plans-go-to-old-plans`).

Acceptance: all gates green tree-wide; plan-73 archived. Commit: —

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

<Filled during execution — the conformance matrix lives here.>

## Summary

The completeness gate for plan-73: turns "timeouts are unified" from a claim into a
cell-by-cell verified matrix, sweeps every retired symbol to zero, and runs the full
tree-wide gates before archiving. No new behavior; it exists so the unification is
provably total, not merely mostly-done.
