# MFBASIC `d`-Register-Native Float Value Model Plan

Last updated: 2026-06-28

Native `Float` is GP-register-native: a value's canonical home is its **bit pattern
in a general-purpose register / stack slot**, and arithmetic `fmov`s it into a
`d`-register only for the op, then `fmov`s the result back to a GPR. plan-16 Piece B
added a peephole that deletes the `fmov`/`str` shuttle where the GPR is provably
dead, but the non-dead cases remain: float-nbody's advance loop still issues ~500
`fmov`s and ~5,400 stack loads/stores per iteration — ~50% of the loop is moving
floats between GPRs, stack slots, and `d`-registers. That memory traffic, not the
arithmetic (~3%) or the finiteness checks (post-plan-17), is now the dominant cost
of the float-arithmetic benchmarks (nbody 4.1×, mandelbrot 3.6× vs `c -O2`).

This plan makes the `d`-register the **canonical** location for a `Float` value:
float locals are stored and loaded as doubles (`str d`/`ldr d`), arithmetic keeps
results resident in `d`-registers, and a GPR copy is materialized **only on demand**
for the few consumers that genuinely need the bits. The single behavioral outcome a
correct implementation produces: **identical program output, with the per-value
`fmov`-to-GPR/stack round-trips gone** so a working set of floats stays in
`d`-registers across a loop body the way `c -O2` keeps it.

It complements:

- `./mfb spec architecture native` (`src/docs/spec/architecture/06_native.md` — the value
  model + calling convention this changes; canonical specs under `src/docs/spec/**`)
- `./mfb spec memory` (`06_native-calling-convention.md` — how `Float` arguments,
  returns, and the finiteness check use a GPR today; the boundary this plan moves)

## 1. Goal

- Represent a `Float` `ValueResult` by a **`d`-register** (or a double-typed stack
  slot), not a GPR bit pattern. Arithmetic results stay in `d`-registers; float
  locals/fields/elements store and load as doubles (`str d`/`ldr d`).
- Materialize a GPR copy (`fmov x, d`) **lazily**, only at the consumers that need
  raw bits — and audit those exhaustively so none is missed.
- Net effect (no behavior change): float working sets stay register-resident; the
  `fmov` shuttle and the stack-as-int-bits traffic that dominate nbody/mandelbrot
  disappear. Plan-16 Piece B's peephole becomes mostly redundant (kept as a backstop).

### Non-goals (explicit constraints)

- **No observable behavior change.** Byte-identical output, identical traps/codes/
  locations. A float slot still holds the same 8 bytes (`str d` and `str x` of the
  same value write identical bytes), so copy/transfer/golden output is unaffected.
- **No value-layout change** (`mfb spec memory`): a `Float` is still an 8-byte f64
  everywhere it is stored. Only the *register class that carries it in flight*
  changes, plus possibly the float arg/return register convention (§4.3).
- **No change to `Integer`/`Byte`/`Fixed`**, to the finiteness rule (plan-17), or to
  the kernels (plan-03). This plan is purely the float *carrier*.

## 2. Current State

- `emit_float_binary` (src/target/shared/code/builder_numeric.rs:858) computes in a
  `d`-register `d_res`, then `float_move_x_from_d(dst, d_res)` makes the GPR `dst`
  the value's home; `float_residents[dst] = d_res` records the `d`-register copy so a
  *parent* float op (`operand_as_double`) reuses it without a re-`fmov`. So chained
  ops within one expression already avoid the round-trip; the gap is the
  **store-to-slot / reload-from-slot** boundary and every non-float consumer.
- A `Float` `ValueResult.location` is a GPR (or a stack slot read as `ldr x`),
  consumed as a GPR in a dozen-plus generic sites — every `store_u64(value.location,…)`
  in builder_bits / builder_collection_* / builder_conversions / builder_control /
  builder_inplace_assign / builder_emit_helpers, plus returns, args, comparisons,
  `toString`, map keys, thread transfer.
- plan-16 Piece B (`peephole::remove_fp_shuttles`) deletes `fmov xN,dM; str xN,[slot]`
  (xN dead) → `str dM,[slot]` and the `ldr`+`fmov` inverse — but only the
  provably-dead-GPR, adjacent case. The ~500 surviving `fmov`s in nbody are the
  reused / non-adjacent cases a peephole cannot reach.

## 3. Design Overview

Make the `d`-register the canonical carrier and materialize GPRs lazily, behind one
choke point. Lowest-risk first; the audit gates the switch.

- **The choke point.** A single accessor `float_value_as_gpr(value) -> reg` that
  every consumer needing bits calls. While the model is still GP-native it just
  returns `value.location`; after the switch it `fmov`s from the resident
  `d`-register on demand. Routing all GPR consumers through it **first** (a no-op
  refactor) is what makes the later switch safe.
- **The carrier switch.** `emit_float_binary` (and the other float producers) stop
  eagerly `fmov`-ing to a GPR; the `ValueResult` carries the `d`-register. Float
  stores/loads use `str d`/`ldr d`. Consumers that are float-aware (another float op,
  `fcmp`, a double-slot store) use the `d`-register directly; the rest hit the choke
  point.
- **The ABI boundary (§4.3).** Float arguments/returns currently travel in GPRs;
  decide whether to move them to `d`-registers (cleaner, fewer `fmov`s at calls) or
  keep the GPR convention and `fmov` at call boundaries (smaller blast radius).

Correctness risk concentrates in the **consumer audit**: a missed GPR consumer reads
an un-`fmov`'d register → silent miscompile. Same audit shape as plan-17, but the
trigger is "reads the float as bits," not "observes the float."

## 4. Detailed Design

### 4.1 `ValueResult` carrier + the choke point

A `Float` `ValueResult` gains a notion of "lives in `d`N" vs "lives in GPR/slot"
(naming convention `%fN`/`dN` vs `xN`, mirroring the existing vreg classes, or a
flag). `float_value_as_gpr(&value)`: if already a GPR/slot → return as-is; if a
`d`-register → `fmov x, d` into a fresh GPR and return that. Every site that today
reads `value.location` for a `Float` and feeds it to a GPR consumer is rewritten to
call this. Phase 1 lands this as a **pure refactor** (the accessor is the identity
while the model is unchanged) so the audit can be reviewed in isolation.

### 4.2 The consumer audit (the gate)

A `Float` needs a **GPR** at exactly: integer/bitwise reinterpretation, `toString`/
print formatting (reads bits/sign), `toInt`/`toByte`/`toFixed` conversion that uses
the bit pattern, map-key bitwise hashing/compare, storing into a *heterogeneous*
slot read elsewhere as `x`, thread-transfer marshalling, and native-FFI `CDouble`
(which loads `d` anyway — verify). A `Float` stays in a `d`-register for: float
arithmetic, `fcmp` comparisons, finiteness checks (plan-17 already FP-domain), and
double-slot store/load. Deliverable: a checklist mapping every consumer to "GPR via
choke point" or "d-register direct"; **completeness is the gate** for Phase 3.

### 4.3 Float arg/return ABI (open decision)

Today `Float` arguments and returns travel in GPRs (`x`), so a `d`-native value
`fmov`s to a GPR at every call/return. Two options: **(a)** move the `Float` arg/
return convention to `d`-registers (eliminates those `fmov`s, but touches the
calling convention spec, all call sites, and thread/FFI marshalling), or **(b)**
keep GPR args/returns and `fmov` at the boundary (no ABI change; calls still pay one
`fmov` per float operand). Recommend **(b) first** (smaller, still wins the loop
body), **(a)** as a follow-on once (b) proves out.

## Layout / ABI Impact

No value-layout change (a `Float` slot is still 8 bytes of the same f64 bits).
Possible **internal** calling-convention change for float args/returns under §4.3
option (a) — documented in `mfb spec architecture native-calling-convention` if
taken; option (b) changes nothing observable. Native-code goldens change (register
classes / `str d` vs `fmov`+`str x`); `.run`/`.ir`/`.ast` goldens must not.

## Phases

1. **Choke-point refactor (no behavior change).** Add `float_value_as_gpr`; route
   every float GPR consumer through it (still the identity). Acceptance: suite +
   acceptance byte-identical (`.run` and native goldens unchanged — it is a no-op).
2. **Consumer audit (§4.2).** Produce the checklist; classify each site GPR-vs-`d`.
   Gate for Phase 3.
3. **Carrier switch.** `emit_float_binary`/producers carry the `d`-register; float
   local store/load become `str d`/`ldr d`; the choke point `fmov`s on demand
   (ABI option (b), §4.3). Acceptance: full suite byte-identical `.run`; native
   goldens regenerated; nbody/mandelbrot `fmov` + stack-traffic counts drop;
   ins-count + `c -O2` ratio measured vs `benchmark/run.log`.
4. **(Optional) `d`-register float ABI** (§4.3 option a) if the call-boundary `fmov`s
   are material after Phase 3.

## Validation Plan

- Behavior: full unfiltered `scripts/test-accept.sh` byte-identical `.run`
  (float output must not change — `str d` and `fmov`+`str x` write identical bits);
  float `_valid`/`_invalid` and the trap-location tests unchanged.
- Runtime proof: float-nbody still `-0.169079859`, leibniz `pi: 3.14159`, mandelbrot
  `in-set: 61852`.
- Metric: per-loop `fmov` count and stack ldr/str count (nbody/mandelbrot), plus
  ins-count and `c -O2` ratio vs `benchmark/run.log` — target: the ~500 `fmov`s and
  much of the 5,400 memory ops in nbody's loop gone.
- Doc sync: `mfb spec architecture native` (the value carrier) and, only if §4.3(a)
  is taken, `native-calling-convention`.

## Open Decisions

- **Float arg/return register class** — keep GPR + boundary `fmov` (recommended V1)
  vs. move to `d`-registers (bigger, cleaner). (§4.3)
- **Carrier representation** — a new `%fN`-style d-vreg for the `ValueResult` carrier
  (reuses the FP vreg class from plan-03) vs. a flag on the existing `ValueResult`.
  Recommend the d-vreg, since the FP register allocator already colors `%fN`.

## Non-Goals

- The finiteness rule (plan-17) and the transcendental kernels (plan-03) — separate.
- pow's hand-written stack-slot working set — it does not go through this value
  model, so it is plan-03's problem, not this one.

## Summary

The risk is the consumer audit (§4.2): the change is "a `Float` lives in a
`d`-register unless a consumer asks for its bits," and a missed consumer is a silent
miscompile. Route every GPR read through one choke point first, audit it in
isolation, then flip the carrier. Value layout, copy/transfer, traps, and output are
all untouched — only the register class carrying an in-flight `Float` changes, which
is exactly the memory traffic that now dominates the float-arithmetic loops.
