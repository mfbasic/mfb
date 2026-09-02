# plan-118-C: Out-of-line string concat, toString, and print marshalling

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-118-A (metrics), plan-118-B (lands first per family order; no design dependency)

Convert the three fattest per-site inline lowerings into synthesized
`runtime.*` functions called from each site. Together they are
**4,764,178 builder-emitted instructions (36 % of all attributed)**:
`binop:&` string concat 2,907,604 over 17,221 sites (169/site),
`call:toString` 1,030,128 over 5,826 sites (177/site), and `rtcall:io.print`
marshalling 826,446 over 3,193 sites (259/site — emitted at the call site
*even though* a shared `runtime.io.print` function already exists; the
marshalling/error-loc/String-build around the call is what's inline). A call
site should cost ~15 instructions (marshal + `bl` + null/error check), an
~85–90 % reduction on these categories.

References:

- plan-118-A §2 (the attribution table; all counts there).
- Emitters this letter rewrites:
  `src/codegen/memory/value/builder_value_semantics.rs:688`
  (`lower_string_concat` — the inline alloc + two byte-at-a-time copy loops +
  inline alloc-failure error block seen verbatim in the micro-fixture:
  `RETURN a & b` = 300 instructions);
  `src/codegen/string/repr/builder_strings.rs:768` (`lower_to_string` — inline
  digit/float formatting, 2 allocs + 2 inline error paths = 315 instructions
  for `RETURN toString(n)`);
  `src/codegen/engine/value/builder_values.rs:2016`
  (`lower_runtime_helper_call` — the per-site print marshalling).
- The synthesis mechanism (precedent): `RuntimeHelper` specs
  (`src/target/shared/runtime/usage.rs:124` `required_helpers`) + the
  `runtime.{call}` `CodeFunction` builders (`src/codegen/engine/builder/mod.rs:2180`,
  and the map helpers `runtime.mapProbe`/`runtime.mapBuildBuckets` at
  `:2578`/`:2366` — maps already went through exactly this out-of-lining).
- Runtime gate: `benchmark/run.sh` (mfb vs c vs python suite).
- Memory/lore: `abi_function` family routing and `.ncodesum` drift notes
  (`.ai/testing-gates.md`; the fs/http/thread golden-churn lesson).

## Prerequisites

Family gate in plan-118-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-118-A landed (attribution tally in `-vv`) | run `-vv`, see "costliest expansion" | NOT MET (A pending) |
| plan-118-B landed (family order; goldens already re-baselined once) | `ls planning/plan-118-B* → planning/completed/` | NOT MET |
| Runtime benchmark suite runs on this box | `benchmark/run.sh` (see `benchmark/README.md`) completes | UNMEASURED — verify before Phase 1 |

## 1. Goal

- Over `tests/acceptance`, the `binop:&` + `call:toString` + `rtcall:io.print`
  attribution rows drop from 4,764,178 to ≤ 700,000 combined, total module
  instructions drop ≥ 3.5 M, and the `benchmark/` string-heavy rows regress
  ≤ 5 % (expected: neutral or faster — the out-of-line loops can use
  word-width copies instead of the current byte-at-a-time loops).

### Non-goals (explicit constraints)

- **No observable behavior change**: same results, same error codes/locations
  on allocation failure (the `ErrorLoc` must still carry the *call site's*
  line/column — passed as arguments to the helper, not lost).
- No change to the String representation (len-prefixed, NUL-terminated,
  inline in records) or arena semantics (`.ai/collections.md` /
  `mfb spec` §14).
- `toString`'s constant-folding fast paths (static values folded at compile
  time) stay: only the runtime-value paths move out of line.
- No regression of the in-place self-append optimization
  (`prescan_string_self_appends` / capacity shadows,
  `function_lowering.rs:990`): `s = s & t` keeps its in-place path — census
  which concat sites that path claims BEFORE moving the general path
  (UNMEASURED; Phase 1 task).

## 2. Current State

- `lower_string_concat` emits per site: length loads, `_mfb_arena_alloc` call,
  a ~45-instruction inline error-result construction on failure, header store,
  and two byte-at-a-time copy loops whose loop bodies round-trip every pointer
  through a stack slot (13 instructions per copied byte — read the
  `string_concat_left_loop` block in any `.ncode` dump).
- `lower_to_string` inlines type-dispatched formatting per site.
- `io.print` sites inline argument marshalling + `_mfb_build_error_loc` +
  result checking around the `bl` to the already-shared `_mfb_rt_io_io_print`.
- The synthesis path (`RuntimeHelper` → `runtime.{call}` `CodeFunction`) is
  demand-driven from an IR usage scan and validated by an
  unused-runtime-helper check (`usage.rs:169` comment) — new helpers must be
  declared exactly where used or validation fails.

### Measured populations

| What | Count | Command |
|---|---|---|
| `binop:&` | 2,907,604 instrs / 17,221 sites | plan-118-A §2 attribution |
| `call:toString` | 1,030,128 / 5,826 | ditto |
| `rtcall:io.print` | 826,446 / 3,193 | ditto |
| toString variants also inline (`callres:toInt` etc.) | ~0.2 M across `to*` rows | attribution dump, `to*` rows |
| Micro: `RETURN a & b` | 300 instrs | `--ncode` fixture (A §2) |
| Concat sites claimed by the in-place self-append path | UNMEASURED | Phase 1 census |
| toString type-dispatch arms (which types inline how much) | UNMEASURED | Phase 1: read `lower_to_string` + per-type attribution |

### Verified properties

- A shared runtime function per module is already how maps, io, arena work —
  read `builder/mod.rs:2180-2620`; the unused-helper validation exists and
  passes today.
- The error path can be shared: `_mfb_make_error_result` /
  `_mfb_build_error_loc` are already out-of-line calls; what's inline is the
  register staging around them (~40 instrs/site).

## 3. Design Overview

One new synthesized function per operation, demand-declared like the map
helpers:

- `runtime.string_concat(left, right, loc) → ptr | error` — allocates,
  word-copies both payloads (fixing the byte-at-a-time loops in the same
  stroke), returns the new block or routes the allocation error. Call sites:
  load two operands + loc registers, `bl`, one check.
- `runtime.to_string_<kind>` for the runtime-value kinds `lower_to_string`
  dispatches on (integer, float, fixed, money, boolean, byte/scalar — exact
  set from the Phase 1 census). Per-kind functions keep each body small and
  monomorphic; sites shrink to marshal + `bl` + check.
- `runtime.io_print_str(ptr, loc)` — the marshalling wrapper around the
  existing `_mfb_rt_io_io_print`, hoisting the per-site error-loc build.

**Design uncertainty (schedule FIRST):** does out-of-lining regress runtime
perf? Phase 1 does concat ONLY and runs `benchmark/run.sh` before/after — the
cheapest falsification. Expectation: faster (word copies, I-cache); if a
string-heavy row regresses > 5 %, stop and re-design (helper inlining
threshold for tiny constant operands) BEFORE Phases 2–3.

**Correctness risk:** ownership/temp registration — `lower_string_concat`'s
result feeds `register_pending_temp` and the self-append machinery; the
helper's returned block must thread through the exact same `ValueResult` shape
(fresh-block, freeable-flat) so statement-scope frees stay correct. A mistake
here is a leak or double-free, invisible to byte diffs — covered by the rt
fixtures + leak checks, not goldens.

Byte-identity is NOT the gate (all concat-touching goldens are EXPECTED to
diff — that is the plan working). Gates: behavior suites + the attribution
numbers + benchmarks.

Rejected: an MIR-level "outline" pass that factors common sequences
automatically (far larger, unpredictable); making `&` an `abi_function`
registry member (it is operator lowering, not a registry call — the
`RuntimeHelper` seam is its native home).

## Phases

### Phase 1 — `runtime.string_concat` + the perf gate

- [ ] Census: how many of the 17,221 `&` sites the self-append path claims
      (leave that path untouched); which fixtures' goldens will churn
      (`grep -rl ' & ' tests/acceptance/src | wc -l` as a floor).
- [ ] `usage.rs`: declare the helper for any function containing a general
      concat; `builder/mod.rs`: synthesize `runtime.string_concat` (word-copy
      loops, shared error path); `builder_value_semantics.rs:688`: rewrite
      `lower_string_concat`'s general path to marshal + call + check,
      preserving the `ValueResult`/pending-temp contract.
- [ ] Run `benchmark/run.sh` before/after on the same box; record both in this
      doc. HARD GATE: string rows ≤ 5 % regression, else stop and re-design.
- [ ] Regenerate churned goldens; `test-accept.sh` full count.

Acceptance: `-vv` attribution `binop:&` ≤ 400 k (from 2.9 M); full
`cargo test --no-fail-fast` + `test-accept.sh` green; benchmark gate met;
leak-sensitive rt fixtures (string append/concat loops) pass.
Commit: —

### Phase 2 — `runtime.to_string_*`

- [ ] Census `lower_to_string`'s runtime arms; per-kind helper for each;
      constant-fold paths untouched.
- [ ] Rewrite `lower_to_string` runtime paths to call the helpers; same for
      the `callres:to*` conversion twins if the census shows they share the
      formatting bodies.
- [ ] Regenerate goldens; benchmark re-run (toString-heavy rows).

Acceptance: `call:toString` attribution ≤ 150 k (from 1.03 M); suites green;
benchmark gate met.
Commit: —

### Phase 3 — print marshalling

- [ ] `runtime.io_print_str` wrapper; rewrite the `rtcall:io.print` site
      emission in `lower_runtime_helper_call` for the print family (census
      first: which `rtcall:` targets share the marshalling shape — `io.write`,
      `io.printErr` likely do).
- [ ] Regenerate goldens; doc sync: `planning/speed.md` note; any spec
      architecture page describing call-site lowering
      (`src/docs/spec/architecture/` — census in-phase).

Acceptance: `rtcall:io.print` attribution ≤ 150 k (from 826 k); module total
over `tests/acceptance` down ≥ 3.5 M vs the plan-118-B baseline; suites green;
benchmark gate met.
Commit: —

## Validation Plan

- Tests: existing string/io unit + rt fixtures; ADD an rt fixture pinning
  concat of empty/1-byte/large strings and allocation-failure error text with
  correct ErrorLoc line (the loc-threading is the subtle part).
- Coverage check: the acceptance corpus's 17,221 `&` sites and 5,826 toString
  sites ARE the denominator; `test-accept.sh` full-count line watched.
- Runtime proof: `benchmark/run.sh` before/after tables recorded in this doc
  per phase.
- Acceptance: full `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all` with regenerated goldens, both-root fmt,
  `cargo check --all-targets`. Cross-arch: the helper bodies go through the
  shared `abi::` layer like the map helpers — prove on the remote boxes per
  `.ai/remote_systems.md` (x86-64 + Windows runs), since lowering changes are
  emission, not runtime proof.

## Open Decisions

- Tiny-operand fast path: keep a short inline path for concat where one side
  is a static string ≤ 8 bytes? Recommended: NO in Phase 1 (measure first;
  add only if the benchmark gate demands it).
- Whether `to*` conversions (toInt/toFixed/…, ~0.2 M combined) ride Phase 2 or
  a follow-up — decide from Phase 2's census of shared formatting bodies.

## Corrections

*(fill during execution)*

## Summary

Biggest single win in the family (36 % of attributed instructions) using an
existing mechanism; the engineering risk is the temp-ownership contract at
each rewritten site, and the one real premise — runtime perf — is tested by
Phase 1's hard benchmark gate before the pattern is repeated.
