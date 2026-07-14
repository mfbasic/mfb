# MFBASIC Register-Native SIMD Vector Types Plan

Last updated: 2026-07-06

> **STATUS (2026-07-06): core DONE + verified (3 commits, 848 acceptance green,
> builtin-vector suite byte-identical, no leaks/double-frees).** Register-native
> carrier (Phases 1–2) for **all 9** vector types (Float/Fixed/Integer 2/3/4) —
> lanes in per-lane scalar carriers, no `arena_alloc`, materialized at every
> storage/escape boundary via a fail-loud `%%vecnative:` marker. Hot ops inlined
> (Phase 3): `scale`/`dot`/`cross`/`lerp_unclamped`/`length`/`distance` (Float),
> bit-identical to the FUNC bodies. Float2/4 parity + Fixed/Integer carrier
> (Phase 5). `normalize` is **kept as a FUNC by design** — its `len==0` FAIL is
> control flow and keeping it a FUNC preserves the domain-error *location* this
> plan's validation requires (it still gets native args/returns from the carrier).
> **Deferred: Phase 4 (register-passing ABI, §4.3)** — the one remaining phase,
> explicitly optional ("only if §4.3 lands"); a high-risk cross-arch calling-
> convention change with marginal benefit (residual FUNC calls pass a correct
> block pointer). Left for a dedicated, box-validated effort.

The `vector::` package's small vector types (`Float2`/`Float3`/`Float4`) are ordinary
**records**: a `Float3` is `{x,y,z}` heap-allocated in the arena, every operation is an
out-of-line FUNC call (`__vector_normalize_float3`, …) that takes vectors **by value**
(record copy in) and `RETURN`s a freshly **allocated** `Float3[…]` (record copy out).
The arithmetic inside those functions is already NEON SIMD and fine — but it is swamped
by the carrier overhead. The vector-math benchmark's hot loop is **75 `bl` calls + 531
memory ops per iteration** for ~8 vector ops: ~1.6M heap allocations + calls + record
copies, putting it at **10.1× `c -O2`** (72 ms vs 7.1 ms) — the worst ratio in the suite.

This plan makes `Float2`/`Float3`/`Float4` **register-native SIMD value types**: a vector
in flight lives in a NEON `v`-register (or `d`-register lanes), constructed without
allocation, with its operations **inlined** as NEON sequences, and passed/returned in
registers. The single behavioral outcome a correct implementation produces: **identical
`vector::` results, with the per-op heap allocation, FUNC-call, and record-copy overhead
gone** so a vector expression compiles to a few NEON instructions the way `c -O2` (or a
GLSL `vec3`) does.

This is the SIMD/representation work deferred from `plan-06-vector` (its "Phase 7").

It complements:

- `./mfb spec language types` and the `vector::` man pages (`src/man/builtins/vector/**`
  — the package API/semantics this must preserve exactly; canonical specs under `src/spec/**`)
- `./mfb spec memory` (record layout — a vector's *stored* form is unchanged; only the
  *in-flight* carrier becomes a register)

## 1. Goal

- Represent an in-flight `Float2`/`Float3`/`Float4` as a **NEON register** (lanes), not a
  heap record. Construction (`Float3[a,b,c]`) loads lanes — **no `arena_alloc`**.
- **Inline** the hot `vector::` ops (`add`/`sub`/`scale`/`dot`/`length`/`normalize`/`cross`/
  `lerp`/`distance`/…) as NEON instruction sequences at the call site — no FUNC call, no
  argument record copy, no allocated result.
- Pass/return vectors in registers (a small-vector calling convention) so calls that do
  remain don't re-marshal through memory.
- Net (no behavior change): the vector-math loop drops from ~75 calls + ~531 memory ops
  to a handful of NEON ops per vector expression.

### Non-goals (explicit constraints)

- **No change to `vector::` API, semantics, or accuracy.** Same results bit-for-bit
  (the inlined NEON must match the current FUNC bodies — they already use the same
  instructions). Same domain errors/codes (`length 0` normalize, etc.).
- **No change to the *stored* layout of a vector** (`mfb spec memory`): when a `Float3`
  is put in a collection, a record field, a map, or transferred to a thread, it is still
  the 3-float block it is today. Only the **register carrier for a value in flight**
  changes (like plan-01-dnative did for scalar `Float`).
- **No change to scalar `Float`/`Integer`/`Fixed`**, the finiteness rule (plan-17), or
  the transcendental kernels (plan-03).
- `Fixed*`/`Integer*` vector variants are a later phase; start with the `Float*` types
  that the benchmark and graphics code exercise.

## 2. Current State

- `src/builtins/vector_package.mfb`: `EXPORT TYPE Float3 { x,y,z AS Float }` (a record);
  ops are MFBASIC FUNCs `__vector_<op>_float3(... AS Float3) AS Float3` that compute
  componentwise and `RETURN Float3[cx,cy,cz]` — each return **constructs (allocates) a
  record**, each call **copies** its vector arguments.
- The arithmetic in those bodies is NEON (the binary has ~1,996 `.4s`/`.2d` lane ops),
  so the math is not the problem; the call/alloc/copy carrier is.
- vector-math hot loop (`/tmp` build): 1,207 instrs/iteration, **75 `bl`**, 531 `ldr`/
  `str`, almost no arithmetic (it is all inside the called funcs). 200K iterations ×
  ~8 ops ⇒ ~1.6M allocations/calls/copies.
- Precedent to mirror: plan-01-dnative made scalar `Float` register-native behind a
  lazy-materialization choke point; this is the same move one type up (a 3–4-lane value),
  plus op inlining.

## 3. Design Overview

Two coupled pieces; the representation gates the inlining.

- **Vector value carrier.** A `Float2`/`Float3`/`Float4` `ValueResult` in flight is a NEON
  register (`vN`, lanes `.2s`/`.4s` for f32? — **no**: MFBASIC `Float` is f64, so a
  `Float3` is three f64 lanes = a `v`-register pair / `.2d`+scalar, or three `d`-registers;
  see Open Decisions on the lane width). Construction loads the component values into the
  lanes; component read (`v.x`) extracts a lane; storing to a record/collection writes the
  3-float block (unchanged memory form). A GPR/memory copy is materialized lazily only at
  a storage/transfer/FFI boundary — the plan-01-dnative choke-point pattern.
- **Op inlining.** Recognize the `vector::` ops on a register-carried vector and emit the
  NEON sequence inline (the same instructions the FUNC body uses today), with no call and
  no allocated result. `dot`/`length`/`distance` reduce lanes to a scalar `Float`;
  `normalize`/`cross`/`scale`/`lerp` produce a register vector. Ops that stay out-of-line
  (rare/large) use the register-passing ABI.

Correctness risk: the carrier must be **bit-identical** to the record path (same NEON, same
reductions, same `length==0` handling), and the storage-boundary audit (where a vector
becomes a stored record / crosses a call/thread/FFI) must be complete — a missed boundary
leaves a register vector where a record block is expected.

## 4. Detailed Design

### 4.1 Carrier + construction (no allocation)

`Float3[a,b,c]` lowers to loading `a`,`b`,`c` (already `Float` values) into the vector
register's lanes instead of `arena_alloc` + three field stores. A `Float3` `ValueResult`
carries the vector register. Component access extracts a lane. A `Float3` *stored* into a
named binding that escapes (collection element, record field, map, thread, FFI, or simply
a `MUT`/`LET` whose address is taken) is written as the existing 3×f64 block via the
choke point `vector_value_as_block(value)` — identity when already a block, `st`-lanes
when a register.

### 4.2 Inlined ops (the speed)

Each `vector::` op gets an inline NEON lowering matching its FUNC body bit-for-bit:
componentwise `add`/`sub`/`scale`/`lerp` are lane ops; `dot`/`length`/`distance` are a
multiply + horizontal add (+ `fsqrt` for length/distance/normalize); `cross` is the lane
shuffle + multiply-subtract. These already exist in `vector_package.mfb`/the SIMD seam —
this phase relocates them to inline emission keyed on the recognized vector type + op,
gated so a non-register (block) operand falls back to the existing call.

### 4.3 Register-passing ABI (the residual calls)

Vector arguments/returns that still cross a real call travel in NEON registers (a small-
vector convention) rather than a copied record block — eliminating the marshal-through-
memory at the boundaries op-inlining doesn't remove. (Mirrors plan-01-dnative §4.3's
arg/return decision, for vectors.)

## Layout / ABI Impact

No change to a vector's **stored** record layout (`mfb spec memory`) — copy/transfer/
collection/golden behavior of a stored vector is unchanged. New **internal** register
carrier + a small-vector register-passing convention (documented in
`mfb spec architecture native-calling-convention` if §4.3 lands). Native-code goldens
change; `.run`/`.ir`/`.ast` must not.

## Phases

1. **Carrier + construction.** `Float3` (then `Float2`/`Float4`) in flight is a register;
   `Float3[…]` constructs without `arena_alloc`; component access extracts lanes; the
   `vector_value_as_block` choke point handles every storage/escape boundary. Acceptance:
   suite byte-identical `.run` (it is a representation change only).
2. **Storage-boundary audit.** Enumerate every site a vector becomes a stored record /
   crosses a call/thread/FFI; route through the choke point. The gate for Phase 3.
3. **Inline the hot ops** (`dot`/`length`/`normalize`/`cross`/`scale`/`lerp`/`distance`/
   `add`/`sub`) — bit-identical NEON, no call, no alloc. Acceptance: vector func tests
   green, vector-math output unchanged, ins-count + `c -O2` ratio measured vs `run.log`.
4. **Register-passing ABI** for the residual vector calls (§4.3).
5. **`Float2`/`Float4` parity, then `Fixed*`/`Integer*` variants.**

## Implementation Notes (2026-07-06 codegen survey — for the implementer)

A concrete, lower-risk design worked out against the current tree:

- **Carrier = per-lane scalar `Float`, not a NEON `v`-register.** A `Float3`
  in flight is 3 scalar `Float` `ValueResult`s (each already a `d`-register via
  the plan-01-dnative carrier). This *reuses the entire tested, cross-arch scalar
  float carrier* (`operand_as_double`/`materialize_float`/`store_value_at`/
  `emit_float_binary` in `builder_numeric.rs`), so it works on x86-64 for free and
  is **bit-identical by construction** — the `vector_package.mfb` op bodies are
  plain scalar float expressions (`a.x*b.x + a.y*b.y + a.z*b.z`), not hand-NEON, so
  replicating that exact scalar order reproduces the same rounding.
- **Side-table, not a ValueResult field.** `ValueResult { type_, location, text }`
  is constructed in hundreds of sites; adding a field is too invasive. Instead keep
  a `vector_natives: HashMap<String, Vec<ValueResult>>` on the builder (like
  `float_residents`), keyed by a **deliberately un-encodable marker** location
  (e.g. `%%VECNATIVE:3:N`). A missed boundary then *hard-errors at the encoder*
  (fail-loud) instead of silently miscompiling.
- **Choke point** `vector_value_as_block(value)`: if native, `arena_alloc` a
  `8*dim` block and `store_value_at` each lane (mirror `emit_build_inlined_record`
  for the alloc/spill/register-lifetime safety), return the block pointer; else
  identity. Route **every** `materialize_float` boundary through a combined
  `materialize_value` (constructor args, call args, append/prepend/insert, map
  key/value ×3, state-assign, return, global store, union-wrap, list/map literal
  element, thread transfer, `LocalRef`). This census is the correctness gate.
- **Interception points** (`builder_values.rs`): `NirValue::Constructor{type_}`
  for a vector type → build lanes, no alloc; `NirValue::MemberAccess` on a native
  vector → return the lane; `NirValue::Call{target}` matching `#vector_<op>_<suf>`
  → inline via `emit_float_binary` when both operands resolve to lanes, else fall
  back to the existing package-FUNC call (the safe default).

**The correctness surface that makes this a multi-session, dedicated effort** (not
safely crammable alongside unrelated work): (1) the **finiteness-observation**
semantics must match per op — `vector::scale` of `1e200` values traps
`ErrFloatOverflow 77050015` today, so each inlined lane product must observe
finiteness at the same boundary with the same code/location; (2) the exact
per-op operation order for `dot`/`length`/`normalize`/`cross`/`lerp`/`distance`
must match `vector_package.mfb` for bit-identical results and identical
`length==0`/domain errors; (3) the boundary census must be **exhaustive** (a
single missed `materialize_value` site is a fail-loud build break for that path);
(4) Phases 4 (register-passing ABI) and 5 (`Fixed*`/`Integer*` variants) remain.
Verification is behavioral (no vector `.ncode` goldens exist), so the gate is the
full `tests/builtin-vector/**` suite staying byte-identical plus the vector-math
benchmark output unchanged — cross-validated on an x86 box per the project's
codegen practice.

## Validation Plan

- Behavior: every `tests/func_vector_*_valid/_invalid` stays green; vector-math output
  (`acc: …`) byte-identical; the `length==0`/domain errors unchanged in code + location.
- Runtime proof: vector-math correct output, and its median + ins-count before/after.
- Metric: vector-math `bl`/`arena_alloc`/memory-op counts in the loop → near zero; `c -O2`
  ratio from 10.1× toward the float-loop band (~2–3×).
- Doc sync: `vector::` man pages unchanged (API stable); `mfb spec architecture native`
  for the carrier, and `native-calling-convention` only if §4.3 lands.
- Acceptance: full unfiltered `scripts/test-accept.sh`; native goldens regenerated.

## Open Decisions

- **Lane width / register form.** `Float` is f64, so a `Float3` is three f64 lanes —
  a `v`-register holds two (`.2d`); options: a `v`-register pair, three `d`-registers, or
  (if a reduced-precision vector path is ever wanted) f32 `.4s`. Recommend three
  `d`-registers / a `.2d`+`d` split (exact f64, no precision change); decide against the
  NEON op availability for `cross`/horizontal-add at f64.
- **Inline vs. keep-callable.** Recommend inlining the hot ops and keeping the existing
  FUNC bodies as the fallback for block-carried operands (and for `pkg doc`), rather than
  deleting them.
- **Scope of first cut.** Recommend `Float3` end-to-end first (the benchmark), then
  `Float2`/`Float4`, then `Fixed*`/`Integer*`.

## Non-Goals

- Auto-vectorizing scalar user loops (separate).
- Changing the `vector::` math/accuracy or API.
- The scalar `Float` carrier (plan-01-dnative, done) or the kernels (plan-03).

## Summary

The gap is pure carrier overhead: small vectors are heap records passed by value through
FUNC calls, so a vector expression is ~75 calls + ~531 memory ops where `c -O2` is a few
NEON instructions. Make the in-flight vector a register (lazy block-materialization at
storage boundaries, exactly like plan-01-dnative for scalars) and inline the ops, which
already emit the right NEON. The math, accuracy, API, and stored layout are all untouched;
only the in-flight representation changes — which is the entire 10.1× gap.
