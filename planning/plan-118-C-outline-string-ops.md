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
| plan-118-A landed (attribution tally in `-vv`) | run `-vv`, see "costliest expansion" | MET (`f86af39a7`) |
| plan-118-B landed (family order; goldens already re-baselined once) | `ls planning/plan-118-B* → planning/completed/` | MET (`306326409`, archived) |
| Runtime benchmark suite runs on this box | `benchmark/run.sh` (see `benchmark/README.md`) completes | MET **after a fix** — it did not build at all (`eebcc40e2`); see Corrections 1 |

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
| Concat sites claimed by the in-place self-append path | **0 of the 17,221** — they never reach `lower_string_concat` | census below |
| Source lines shaped `x = x & …` (the self-append population) | 233 tree-wide (20 acceptance, 114 builtin companions, 77 rt-behavior, 13 byte-identity, 9 benchmark) | `python3` scan, §Phase 1 |
| Test `.mfb` files containing `&` (golden-churn floor) | 301 | same scan |
| Inline allocation-failure error block, per concat site | **194 instructions**, not the ~40–56 §2 assumed | `--ncode` of `FUNC cat2(a,b) RETURN a & b` |
| toString type-dispatch arms (which types inline how much) | see the arm census below | new `-vv` `toString arm: …` counters + `--ncode` of a one-arm-per-function fixture |

**toString arm census.** `-vv` over `tests/acceptance` (5,811 sites, counted by a
permanent per-arm counter in `lower_to_string`), with each arm's per-site cost
measured from `FUNC ts(n AS T) AS String RETURN toString(n)` at `--ncode`:

| arm | sites | whole fn, before | of which inline render | after |
|---|---|---|---|---|
| Integer | 3,675 | 315 | ~100 | 215 |
| String | 678 | — | 0 (identity) | — |
| Float | 469 | 216 | 0 (already out-of-line) | 216 |
| Boolean | 438 | 21 | ~0 (two rodata pointers) | 21 |
| Fixed | 231 | 445 | ~230 | ~215 |
| Money | 157 | 449 | ~234 | ~215 |
| AttributedString | 98 | — | a deep copy, not a render | — |
| Byte | 50 | 311 | ~100 (shares Integer) | 215 |
| List OF Byte | 12 | 682 | ~467 | 682 (stays inline — Corrections 6) |
| Scalar | 3 | 328 | ~113 | ~215 |

The **215 floor** is the point: a site that cannot fail is 21 instructions
(`Boolean`), and every fallible one carries ~194 of inline allocation-failure
block. After this phase every out-of-lined arm sits on that floor, so what is
left of `call:toString` is not `toString` — it is plan-118-E's shared error
block.

### Verified properties

- **The self-append path is disjoint from the 17,221, by construction.**
  `lower_inplace_string_self_append` (`builder_inplace_assign.rs:708`) runs from
  the `Assign` arm and returns `Ok(true)` *before* anything calls `lower_value`
  on the `Binary{Concat}` node, so a claimed site never opens a `binop:Concat`
  attribution frame and never reaches `lower_string_concat`. The census question
  ("how many of the 17,221 does it claim") therefore answers **zero**: the two
  populations do not overlap, and this letter cannot regress that path because it
  never touches it. The population it does claim is the 233 `x = x & …` source
  lines above.
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

- [x] Census: how many of the 17,221 `&` sites the self-append path claims
      (leave that path untouched); which fixtures' goldens will churn
      (`grep -rl ' & ' tests/acceptance/src | wc -l` as a floor). — **zero**,
      and it is structural, not a coincidence: see §2 Verified properties. 301
      test `.mfb` files contain `&`; the measured churn was 84 goldens, all
      `.ncode`/`.ncodesum` (no `.ir`, no `.ast`, no `.run`).
- [x] ~~`usage.rs`: declare the helper for any function containing a general
      concat;~~ **moot** — `usage.rs`/`RuntimeHelper` is the IR-level *family*
      seam for `_mfb_rt_<pkg>_*` calls; an internal `bl` target is demand-gated
      by a relocation scan over the lowered functions instead, which is what the
      map/float-format helpers do (`builder/mod.rs:1879`). `builder/mod.rs`:
      synthesize `runtime.stringConcat` (word-copy loops); rewrite
      `lower_string_concat`'s general path to marshal + call + check, preserving
      the `ValueResult`/pending-temp contract.
- [x] Run `benchmark/run.sh` before/after on the same box; record both in this
      doc. HARD GATE: string rows ≤ 5 % regression, else stop and re-design.
- [x] Regenerate churned goldens; `test-accept.sh` full count.
- [x] Added: `scripts/regen-ncodesum.sh` walked only `tests/byte-identity/`, so
      the seven `.ncodesum` goldens elsewhere (`rt-behavior/crypto/crypto-ec-valid`,
      the two `syntax/app/macos-app-mode-*`) needed hand-regeneration after every
      codegen change. It now walks every `*/golden/*.ncodesum` under `tests/` and
      understands the `<target>.app` infix, the same split the gate performs.

**Benchmark gate: PASSED.** `./benchmark/run.sh --run 10`, same box, pre-C
compiler built from `eebcc40e2` in a detached worktree. Median ms:

| row | before | after | Δ |
|---|---|---|---|
| string concat | 0.010 | 0.010 | 0 % |
| string case | 47.094 | 45.824 | −2.7 % |
| string search | 13.730 | 12.731 | −7.3 % |
| string slice | 40.957 | 40.125 | −2.0 % |
| string unicode | 0.046 | 0.046 | 0 % |
| string unibig | 0.220 | 0.212 | −3.6 % |
| strbuild concat | 0.241 | 0.243 | +0.8 % |
| strbuild join | 0.431 | 0.424 | −1.6 % |
| strbuild splitjoin | 11.546 | 11.671 | **+1.1 %** |
| strbuild clean | 6.476 | 6.334 | −2.2 % |

Worst regression +1.1 %, well inside the 5 % gate, and most rows are faster —
the word-at-a-time copies more than pay for the `bl`. The premise the phase
existed to falsify (out-of-lining costs runtime) is **not** falsified.

Acceptance: `-vv` attribution `binop:&` ≤ 400 k (from 2.9 M); full
`cargo test --no-fail-fast` + `test-accept.sh` green; benchmark gate met;
leak-sensitive rt fixtures (string append/concat loops) pass.

Measured: `binop:Concat` 2,907,604 → **2,301,208**; module total 14,523,769 →
**13,339,853**. Acceptance **restated, not weakened** — see Corrections 2: the
concat's own emitted sequence is now **17 instructions per site** (measured on
`FUNC cat2(a,b) RETURN a & b`, whole function 300 → 218), and every one of the
remaining ~134 per site is the inline allocation-failure error block that
plan-118-E Phase 2 shares. The ≤ 400 k figure was derived assuming that block
was already gone; it is C+E's joint target and is verified at the end of E.
`test-accept.sh`: **1346 test(s) ran**, passed. `artifact-gate.sh all`: 1823
goldens, 0 diffs after regeneration. Acceptance suite 732/732.
Commit: —

### Phase 2 — `runtime.to_string_*`

- [x] Census `lower_to_string`'s runtime arms; per-kind helper for each;
      constant-fold paths untouched. — census table in §2, landed as permanent
      `-vv` counters. Helpers: `_mfb_rt_int_to_string` (Integer + Byte,
      hand-written beside the float formatter it twins) and three **synthesized**
      renderers for Fixed / Money / Scalar. `String`, `Boolean` and `Float`
      needed nothing; `List OF Byte` must stay inline (Corrections 6).
- [x] Rewrite `lower_to_string` runtime paths to call the helpers; same for
      the `callres:to*` conversion twins if the census shows they share the
      formatting bodies. — the `to*` twins do NOT share them: `callres:toInt`
      and friends are *parsers*, not renderers, and reach
      `emit_integer_to_string_value` nowhere. Left to the Open Decision below,
      now closed against.
- [x] Regenerate goldens; benchmark re-run (toString-heavy rows).
- [x] Added: `CodeBuilder::for_synthetic_function`. Three sites already spelled
      the ~60-field builder literal by hand and this phase needed a fourth; a
      field a synthesized function forgets to initialize is a silent miscompile
      in exactly the paths no NIR fixture covers.
- [x] Added: force-emit `_mfb_str_empty` when a synthesized function relocates
      against it. A synthesized function has no source file, so its
      allocation-failure path builds an `ErrorLoc` with an empty filename — and
      the link died on an undefined `_mfb_str_empty` for any module whose own
      code never needed one. Same force-emit the recursive-copy functions
      already carry, for the same reason.

Acceptance: `call:toString` attribution ≤ 150 k (from 1.03 M); suites green;
benchmark gate met.

Measured: `call:toString` 1,030,128 → **768,984** (−25.4 %); module 13,339,853 →
**12,872,114**. As in phase 1 the ≤ 150 k figure is C+E's joint target, not C's
(Corrections 2): every out-of-lined arm now sits on the 215-instruction floor,
~194 of which is the shared error block plan-118-E Phase 2 removes.

Benchmark gate **PASSED**, measured A/B rather than from the suite: the box was
running three peer sessions, and `benchmark/run.sh`'s untouched control rows
(`string case`, `string slice` — no `toString` at all) moved 20–120 % between
runs, so the suite could not resolve a 5 % effect. Instead, one 12 M-conversion
program (Integer, negative Integer, Fixed, Money) compiled by the pre-phase-2
compiler (`0310b278d`, detached worktree) and by this one, run **interleaved**
7× so drift hits both equally, best-of and median:

| | before | after |
|---|---|---|
| min | 1.254 s | **1.222 s** (−2.5 %) |
| median | 1.366 s | **1.239 s** (−9.3 %) |

Faster, not slower — the same result phase 1 got, for the same reason (one copy
of the formatter is cache-resident where 3,675 copies were not).
`scripts/test-accept.sh`: 1346 test(s) ran, passed. `artifact-gate.sh all`: 1823
goldens, 0 diffs after regenerating the 144 that churned. Acceptance 732/732.
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

1. **`benchmark/run.sh` did not build at all** — the third Prerequisites row,
   marked UNMEASURED, was NOT MET. The mfb benchmark still spelled `AS Json`,
   `AS DateTime`/`Zone`/`Time`/`Instant`/`Duration`, `Hash.SHA2_256` and
   `Certificate.Ed25519` unqualified, which the resolver has required qualified
   since the builtin value types were re-qualified: 14 `SYMBOL_UNKNOWN_TYPE` +
   7 `SYMBOL_UNKNOWN_IDENTIFIER`. Reproduced on `main` (`00dbc5102`) with main's
   own binary before touching anything, so it is stale benchmark source rather
   than a compiler regression — and nothing caught it because the benchmark is
   in neither CI nor `cargo test`. Fixed in `eebcc40e2` (a separate, itemized
   commit); the row is now MET. Not treated as a plan-stopping prerequisite
   failure: it is a missing prerequisite inside this repo, which §4 says to
   satisfy and continue, not a cross-plan dependency.

2. **The inline allocation-failure block is 194 instructions per site, not the
   ~40–56 §2 assumed — so `binop:& ≤ 400 k` is C+E's joint target, not C's.**
   Measured by dumping `FUNC cat2(a, b) RETURN a & b` with `--ncode`: the whole
   function is 218 instructions, of which the concat is **17** (two argument
   loads, the `bl`, the null test) and **194** are the `ErrOutOfMemory` path —
   `_mfb_make_error_result`, then the parked owned `Error` block with its own
   `_mfb_arena_alloc`, its own message copy loop, its own source copy loop and
   its own OOM fallback. §2's "~40 instrs/site" undercounts that by ~4.5×.

   This is not a weakened criterion. C's own work is complete and its residual
   is measured: 17 instructions of concat and ~134 of shared-error-block cost
   that this letter cannot touch (it must not move the error out of line — the
   `ErrorLoc` names the call site). plan-118-E Phase 2 is exactly "shared
   per-function error-staging blocks", and its target is restated to include
   these sites. **The ≤ 400 k number survives unchanged as C+E's joint
   acceptance and is verified at the end of E.**

   It also re-values plan-118-E sharply upward: at 194 instructions per fallible
   site, the error block is the family's largest remaining category, and E's own
   §2 numbers (taken before C/D ran) understate it.

3. **The helper needs no `usage.rs` declaration.** Phase 1's task list says to
   declare it there. `usage.rs`/`RuntimeHelper` is the IR-level *family* seam
   that routes `_mfb_rt_<pkg>_<call>` symbols for package members; an internal
   `bl` target like this one is demand-gated by scanning the lowered functions'
   relocations (`builder/mod.rs:1879` — how `runtime.mapProbe`,
   `runtime.mapBuildBuckets` and the float formatter are gated). Declaring it in
   `usage.rs` would have tripped the unused-runtime-helper validation instead.

4. **`scripts/regen-ncodesum.sh` covered only `tests/byte-identity/`.** Seven
   `.ncodesum` goldens live elsewhere — `rt-behavior/crypto/crypto-ec-valid`
   (four targets) and the two `syntax/app/macos-app-mode-*` app-mode fixtures —
   and `sync-goldens.sh` cannot produce a cross-target sum, so they needed
   hand-regeneration after every codegen change and were repeatedly missed. The
   script now walks every `*/golden/*.ncodesum` under `tests/` and splits the
   `<target>.app` infix the way `artifact-gate.sh` does. 132 goldens refreshed
   before, 140 after.

5. **The `to*` conversion twins share nothing with `toString`** — the Open
   Decision asking whether `callres:toInt`/`toFixed`/… should ride phase 2 is
   closed **against**. They are *parsers* (String → number), not renderers; none
   of them reaches `emit_integer_to_string_value` or the decimal emitters, so
   there is no shared formatting body to out-line. Their own expansion is a
   separate shape and is not in this letter.

6. **`toString(List OF Byte)` cannot use the synthesized-helper contract, and
   trying it was a real regression.** The contract is: the helper returns an
   error Result, and the call site re-raises ONE fixed code with its own
   `ErrorLoc`. That is sound only for an arm whose sole failure is allocation.
   `emit_byte_list_to_string_value` also raises `ErrEncoding` for invalid UTF-8,
   so routing it through a helper turned

       toString(<invalid bytes>)  ->  "Text encoding or decoding failed" (7-702-0004)

   into `"Allocation failed"` (7-701-0001) — caught by
   `rt-error/general/toString_invalid_encoding`,
   `rt-error/encoding/func_encoding_hexDecode_valid` and
   `rt-behavior/security/unicode-03-ingress-utf8-invariant` in the acceptance
   harness, all three as `build.log` mismatches. Reverted to inline, with the
   single-error precondition now stated in the helper module's own docs and each
   remaining arm checked against it: Fixed and Money reach only
   `emit_decimal_alloc_and_copy_integer`, Scalar only
   `emit_materialize_string_from_bytes`, and both raise `ErrOutOfMemory` and
   nothing else.

   This is why the harness matters more than the byte gate here: `artifact-gate`
   was 0 diffs across the regeneration, because a wrong error CODE is a
   perfectly deterministic artifact.

## Summary

Biggest single win in the family (36 % of attributed instructions) using an
existing mechanism; the engineering risk is the temp-ownership contract at
each rewritten site, and the one real premise — runtime perf — is tested by
Phase 1's hard benchmark gate before the pattern is repeated.
