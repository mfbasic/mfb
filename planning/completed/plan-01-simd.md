# MFBASIC SIMD Math Array Overloads Plan

Last updated: 2026-06-26

This plan adds **array (vectorized) overloads** to the `math::` package so that a
single call processes a whole `Float[]` / `Integer[]` / `Fixed[]` at once on AArch64
NEON hardware and returns a freshly allocated result array. A correct
implementation makes `math::ceil(values AS Float[]) AS Integer[]` (and the ~30 other
overloads listed in §1) emit genuine NEON vector instructions that compute every
lane in parallel, allocate one new homogeneous numeric `List`, and raise a single
`ErrOverflow` / `ErrInvalidArgument` when *any* lane is out of range — with scalar
`math::` behavior, value/copy/transfer semantics, and golden output otherwise
untouched.

The defining constraint, discovered during the compiler review: **the AArch64
backend cannot encode a single NEON/vector instruction today.** `reg` maps only
`x0..x28`, `lr`, `sp`, and `d0..d7`; the `CodeOp` enum is entirely scalar GPR + scalar
double FP (`src/arch/aarch64/ops.rs`, `src/arch/aarch64/encode.rs:reg`). Scalar
`math::sqrt` uses the scalar `fsqrt_d`; transcendentals call libm via `branch_link`
(`src/target/shared/code/builder_math.rs:lower_external_math`). There is no second
backend (`src/arch/mod.rs` exposes only `aarch64`), so this is single-target work —
but the vector encoder must be built from zero. That is Phase 1 and the bulk of the
risk.

It complements:

- `./mfb spec architecture aarch64-instruction-set` (the `CodeOp` repertoire and
  encodings this plan extends; canonical source under `src/spec/architecture/14_*`)
- `./mfb spec memory collections` and `./mfb spec memory arenas` (the 40-byte
  List header + 40-byte-per-entry lookup table + packed data region this plan
  reads and allocates; `src/spec/memory/**`)
- `./mfb spec memory native-calling-convention` (register classes / clobber sets —
  the arena-alloc clobber hazard below)
- `./mfb spec diagnostics error-codes` (`ErrOverflow` `77050010`,
  `ErrInvalidArgument` `77050002`; `src/spec/diagnostics/02_error-codes.md`)
- `./mfb spec language builtin-functions` (§18.2 math member list; must gain the
  array overloads)

## 1. Goal

Add the following **array overloads** to `math::` (scalar overloads stay exactly as
they are; selection is by argument type via `resolve_call`):

| Function | Overloads (array element type → result) | Error |
|---|---|---|
| `ceil` | `Float[]→Integer[]`, `Fixed[]→Integer[]` | `ErrOverflow` (Float only) |
| `floor` | `Float[]→Integer[]`, `Fixed[]→Integer[]` | `ErrOverflow` (Float only) |
| `round` | `Float[]→Integer[]`, `Fixed[]→Integer[]` (ties away from zero) | `ErrOverflow` (Float only) |
| `abs` | `Float[]→Float[]`, `Integer[]→Integer[]`, `Fixed[]→Fixed[]` | `ErrOverflow` (Integer/Fixed min value) |
| `clamp` | `(Float[],Float,Float)→Float[]`, `(Integer[],…)→Integer[]`, `(Fixed[],…)→Fixed[]` | — |
| `min` / `max` | `(Float[],Float[])→Float[]`, `(Integer[],Integer[])→Integer[]`, `(Fixed[],Fixed[])→Fixed[]` | `ErrInvalidArgument` if lengths differ |
| `sqrt` | `Float[]→Float[]`, `Fixed[]→Fixed[]` | `ErrInvalidArgument` (negative lane) |
| `exp` | `Float[]→Float[]` | `ErrOverflow` |
| `log` / `log10` | `Float[]→Float[]`, `Fixed[]→Fixed[]` | `ErrInvalidArgument` (lane ≤ 0) |
| `pow` | `(Float[],Float[])→Float[]` | `ErrInvalidArgument` if lengths differ |
| `sin` / `cos` / `tan` | `Float[]→Float[]` | — |
| `asin` / `acos` | `Float[]→Float[]` | `ErrInvalidArgument` (lane outside [-1,1]) |
| `atan` | `Float[]→Float[]` | — |
| `atan2` | `(Float[],Float[])→Float[]` | `ErrInvalidArgument` if lengths differ |

Concrete checkable outcome: each overload compiles, lowers to NEON code, runs, and
produces a new array whose lanes equal the scalar function applied element-wise;
each error overload exits 255 with the specified code when a lane is out of range;
acceptance passes; every overload has `tests/func_math_<fn>_*_valid/**` and
`_invalid/**` coverage.

This plan also unifies the **scalar** Float transcendentals onto the same kernels
(§4.7), so that after Phase 6 there is **one deterministic math surface**:
`math::sin(x)` and `math::sin([x])[0]` produce the *bit-identical* result, the same
on macOS and Linux (glibc and musl), with no libm dependency for those functions.

**Accuracy oracle: macOS libm, within ≤1 ULP.** The kernels are tuned and validated
against the `f64` values produced by macOS's system libm (the current scalar
backend): each kernel result must be within **≤1 ULP** of the macOS-libm result for
that input. macOS libm is the reference of record; the kernel's own deterministic
output is the golden. Two invariants still hold exactly (0 ULP), independent of the
libm tolerance: `math::f(x) == math::f([x])[0]` (scalar and array share one kernel)
and identical results across all targets (same kernel on macOS and Linux). Because
the bar is ≤1 ULP rather than bit-exact, Phase 6's scalar re-point off libm onto the
kernel **may shift scalar values by up to 1 ULP on macOS** — an expected, validated
golden update, not a regression.

### Non-goals (explicit constraints)

- **No change to scalar *algebraic* `math::` overloads.** Existing
  `math::ceil(Float)`, `abs`, `min`/`max`, `clamp`, `round`, `floor`,
  `sqrt(Float)` (already hardware `fsqrt_d`) keep their exact codegen and golden
  output. Array overloads are *added* paths in `resolve_call` / `lower_math_call`,
  selected only when an argument is a `List`. **Exception (deliberate, §4.7):** the
  scalar Float *transcendentals* (`exp, log, log10, sin, cos, tan, asin, acos, atan,
  atan2, pow`) are re-pointed from libm onto the shared deterministic kernels — their
  runtime values (last ULP) and emitted code change, and their goldens are
  regenerated against a validated reference. This is the "one deterministic surface"
  goal, not incidental drift.
- **No change to the List/collection memory layout or ABI** (`mfb spec memory
  collections`): 40-byte header, 40-byte lookup entries, packed data region, the
  capacity-derived data base. Result arrays are allocated *tight* (`capacity ==
  count`, `dataCapacity == dataLength`) like every other freshly built list.
- **No change to value/copy/move/freeze or thread-transfer semantics.** A result
  array is an ordinary arena-owned `List` subject to scope-drop frees
  (`[[scope-drop-frees]]`); inputs are read-only and not mutated.
- **No new element types and no SIMD-only value type.** Arguments and results are
  ordinary `List OF Integer|Float|Fixed`. There is no user-visible "vector" type.
- **No second architecture.** AArch64 only. The vector encoder is new but lives
  behind the same `CodeOp`/encoder pipeline as every other instruction.
- **No change to Fixed's Q32.32 representation** or to the deterministic-Fixed
  policy: Fixed results stay platform-independent. Float transcendentals **become**
  deterministic (the old "platform libm, last-ULP may vary" caveat is retired by
  §4.7); this is an intended behavior change, documented in the spec, not a silent
  one.

## 2. Current State

**Builtin metadata** (`src/builtins/math.rs`). Pure name + type tables, no registry:
`is_math_call` (names), `arity`, `call_param_names`, `expected_arguments`, and the
overload resolver `resolve_call(name, arg_types) -> ResolvedCall { return_type }`
(`math.rs:142`). `arg_types` are stringified type names from `typecheck::type_name`.
Today every helper assumes scalar names (`"Float"`, `"Integer"`, `"Fixed"`);
`all_same_numeric` / `one_float_or_fixed` / `two_same_float_or_fixed` (`math.rs:186`)
are the gates to extend. `call_return_type_name` (`math.rs:84`) returns an
*arg-independent* type and is consulted by `builtins/mod.rs:141` — note it returns a
single static type, which the array overloads cannot satisfy (floor scalar→Integer
vs floor `Float[]`→`Integer[]`), so the array return type must flow through
`resolve_call` (which sees `arg_types`), and any `call_return_type_name` consumer
must be confirmed to fall back to `resolve_call` for List arguments (§4.2).

**Typecheck** (`src/typecheck.rs`). `is_math_call` → `check_math_builtin_call`
(`typecheck.rs:5491`): infers each arg type via `type_name`, checks `arity`, calls
`resolve_call`, returns `parse_type(resolved.return_type)`. List types print as
`"List OF Float"` etc. (`Type::List(Box<Type>)`, `typecheck.rs:17`); the array
overloads key off exactly these strings — the implementer must confirm the precise
spelling `type_name` produces for `Type::List(Type::Float)` and match it.

**Codegen dispatch** (`src/target/shared/code/builder_values.rs:547`):
`target.strip_prefix("math.")` → `lower_math_call(function, args)`
(`builder_math.rs:8`). Scalar precedents to mirror per family:
- `lower_math_abs` (`builder_math.rs:37`) — sign-mask for Float, min-int overflow
  check (`emit_overflow_return`) for Integer/Fixed.
- `lower_math_rounding` (`builder_math.rs:245`) — `frintp/frintm` + `fcvtzs`
  (Float), with `emit_float_rounding_integer_range_check`.
- `lower_math_clamp` (`builder_math.rs:150`) — `emit_invalid_argument_return` when
  `low > high`.
- `lower_math_sqrt` (`builder_math.rs:435`) — scalar `fsqrt_d`, negative→invalid.
- `lower_external_math` (`builder_math.rs:472`) — spills args, `branch_link` to a
  libm symbol resolved from `platform_imports` (`external_math_symbol`,
  `builder_math.rs:664`); Fixed routes to deterministic Q32.32
  (`lower_fixed_external_math`, `builder_fixed_math.rs`).
- `lower_math_rand` (`builder_math.rs:320`) — precedent for calling an **internal
  runtime symbol** (`_mfb_rng_next`, `mod.rs:177`) with a relocation, and for the
  arena/scratch register-lifetime discipline.

**Collection layout** (`mfb spec memory collections`; constants in
`builder_collection_layout.rs` / `mod.rs:280`): 40-byte header
(`kind,keyType,valueType,flagsVersion` then `count@8, capacity@16, dataLength@24,
dataCapacity@32`), then `count` × 40-byte lookup entries (`flags@0, …,
valueOffset@24, valueLength@32`), then the **packed, 8-byte-aligned data region** at
`base + 40 + capacity*40`. For a homogeneous `Integer|Float|Fixed` list the data
region is a contiguous array of 8-byte lanes — exactly the SIMD-friendly shape: read
`count` + data base once, stream the data region in 128-bit (2-lane) chunks. Type
codes: Integer 3, Float 4, Fixed 5. New lists are built by `lower_collection_values`
(`builder_collection_layout.rs:1134`) via `_mfb_arena_alloc`.

**Arena/register hazard** (`[[arena-alloc-clobbers-x14-x15]]`, `.ai/compiler.md`):
`bl _mfb_arena_alloc` clobbers `x0,x1,x9,x10,x14,x15,x16,x20-x28`. The input array
pointer, `count`, and any loop state must be spilled to stack across allocation and
reloaded — this is the layout-sensitive class of bug that only shows past a length
threshold.

**Encoder** (`src/arch/aarch64/{ops.rs,encode.rs,abi.rs}`). Adding an instruction =
`CodeOp` variant + `mnemonic` + `from_mnemonic` + `emit_*` (base word + operand bit
fields) + `instruction_size` + an `abi::` fluent constructor. `reg` currently has
**no V/Q register names and no arrangement suffixes** — this is the gap Phase 1
fills.

**Tests** (`scripts/test-accept.sh`). Per-function dirs `tests/func_<pkg>_<fn>_<flavor>/`
with `project.json`, `src/main.mfb`, `golden/{*.ast,*.ir,*.run,build.log}`. Valid =
golden match incl. `.run` stdout; invalid = `build.log` diagnostics; runtime-error =
successful build then `Code: <n> Message: …` + `[exit 255]`. Precedent runtime-error
dir: `tests/func_math_exp_fixed_overflow_rt/`.

## 3. Design Overview

Four layers, built bottom-up so each phase is independently landable and the
highest-risk codegen lands last behind tests:

1. **Vector encoder (Phase 1).** Teach `reg`/the encoder a V-register naming scheme
   (`v0.2d`, `v0.2s`, `v0.16b`, plain `q0`/`d0` for load/store) and add the closed
   set of NEON `CodeOp`s the kernels need. Unit-tested against known-good machine
   words. No language behavior; acceptance unchanged.
2. **List-build runtime helper + SIMD loop scaffold (Phase 2).** One internal
   runtime symbol `_mfb_simd_alloc_list(count, typeCode) -> ptr` that allocates a
   tight homogeneous numeric list (header + uniform lookup table) and returns its
   base, plus reusable codegen helpers for the 2-lane chunk loop, the scalar tail,
   and the per-lane error-mask reduce. This isolates the arena-clobber discipline in
   one place and makes the per-op lowerings small.
3. **Algebraic overloads (Phase 3).** The overloads that map to *direct* NEON
   instructions — `abs`, `min`, `max`, `clamp`, `ceil`, `floor`, `round`,
   `sqrt(Float[])` — for Float/Integer/Fixed. These are the genuine, deterministic
   inline-SIMD win and carry the bulk of the value.
4. **Transcendentals (Phases 4–5).** Fixed transcendentals (`sqrt`, `log`,
   `log10`) via the deterministic Q32.32 path (Phase 4); Float transcendentals
   (`exp`, `log`, `log10`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`,
   `pow`) via **hand-written NEON polynomial kernels — the same code on macOS and
   Linux** (Phase 5), with **no external math-library dependency**. Highest risk:
   kernel accuracy and per-lane domain/overflow detection.

The hand-rolled kernels are deliberate, not a fallback: there is no system vector
math library available across the compiler's targets (see § Open Decisions #1 —
macOS Accelerate/vForce exists, but Linux `libmvec` is glibc-only and AArch64
vector symbols only landed in glibc ≈2.40, and musl has none). Since Linux forces a
hand-rolled kernel regardless, using that same kernel on macOS gives one
implementation, one dependency-free link, and **bit-identical Float results across
all targets**. Because the kernel is now the only accurate Float transcendental
implementation in the compiler, the **scalar** overloads are re-pointed onto it too
(§4.7), collapsing scalar and array onto **one deterministic math surface** and
dropping the libm dependency for those functions. The array path's per-lane tail and
the scalar overload share the identical instruction sequence + coefficients, so
`sin([x])[0] == sin(x)` by construction.

Correctness risk concentrates in (a) the encoder bit-fields (Phase 1 — wrong
encoding = silent corruption), (b) the arena-clobber register lifetimes across the
alloc helper (Phase 2), (c) per-lane error detection + tail handling (Phases 3–5),
and (d) the polynomial-kernel accuracy + coefficient provenance (Phase 5).

## 4. Detailed Design

### 4.1 Vector encoder (Phase 1)

Extend operand decoding and the op set. Keep it the **minimal closed set** the
kernels in Phases 3–5 actually use — no speculative instructions.

- **Register naming.** Add V-register parsing to `encode.rs:reg` (or a sibling
  `vreg`/arrangement decoder): names `v0..v31` carrying an arrangement suffix
  (`.2d` 2×i64/f64, `.2s`, `.4s`, `.16b`, `.8b`) plus `q0..q31` and reuse `d0..d31`
  for scalar tail. The encoder needs the register number (0–31) and, for
  arranged ops, the `size`/`Q` bits. Decide a string convention (e.g.
  `"v3.2d"`) and a helper returning `(num, arrangement)`. Document in
  `mfb spec architecture aarch64-instruction-set`.
- **Loads/stores:** `ldr_q` / `str_q` (128-bit), reusing existing `d`-form scalar
  load/store for the tail.
- **FP vector ops (`.2d`):** `fabs`, `fsqrt`, `fadd`, `fsub`, `fmul`, `fdiv`,
  `fmla`/`fmls` (fused multiply-add — Horner polynomial evaluation in the
  transcendental kernels), `fmin`, `fmax`, `frintp`, `frintm`, `frinta`, `frintn`,
  `fcvtzs` (→i64), `fcvtas` (→i64 ties-away), `scvtf` (i64→f64, kernel exponent
  reconstruction), `fcmgt`/`fcmlt`/`fcmge`/`fcmeq` (domain masks), `dup` (broadcast
  scalar→both lanes).
- **Integer vector ops (`.2d`/`.16b`):** `add`, `sub`, `mul`, `shl`/`sshr`/`ushr`
  (immediate Q32.32 shifts and kernel exponent-field assembly), `ushl`/`sshl`
  (register, **per-lane variable** shift — each Fixed lane normalizes by its own
  amount in the `log`/`sqrt` kernels), `clz` (leading-zero count — Q32.32
  normalization / exponent extraction), `abs`, `neg`, `smin`, `smax`,
  `cmgt`/`cmlt`/`cmeq` (overflow/domain masks), `dup` (broadcast), `and`/`orr`/`eor`
  (mask combine, sign handling), `bsl`/`bit` (lane-wise select for special-case
  blending in the kernels).
- **Horizontal reduce:** `umaxv`/`uminv` (or `addv`) over a `.16b`/`.4s` mask to
  collapse a per-lane error mask to a single GPR for one branch to the error
  return. (Alternative: `umaxp` + `fmov` to GPR.)

Each gets `CodeOp` + `mnemonic` + `from_mnemonic` + `emit_*` + `instruction_size`
(all 4 bytes) + `abi::` constructor. **Validation:** add `#[cfg(test)]` encoder unit
tests asserting the exact 32-bit little-endian word for each new op against
references assembled out-of-band (e.g. `llvm-mc`/known opcodes) — the encoder is the
one place a wrong constant silently corrupts every caller.

### 4.2 Builtin metadata + typecheck (begins Phase 3)

In `src/builtins/math.rs`, generalize the gates so an argument may be a homogeneous
numeric `List`:

- `resolve_call`: add array arms. The result type is **arg-dependent**, so it must
  be produced here (not in `call_return_type_name`):
  - `ABS|MIN|MAX|CLAMP` over `List OF T` (T numeric) → `List OF T`.
  - `FLOOR|CEIL|ROUND` over `List OF Float|Fixed` → `List OF Integer`.
  - `SQRT` over `List OF Float|Fixed` → same list type.
  - `EXP|SIN|COS|TAN|ASIN|ACOS|ATAN` over `List OF Float` → `List OF Float`.
  - `LOG|LOG10` over `List OF Float|Fixed` → same list type.
  - `POW|ATAN2` over two `List OF Float` → `List OF Float`.
  - `MIN|MAX` two-array forms: both args same `List OF T`.
- `arity` is unchanged (array overloads have the same arities as their scalar
  siblings). `call_param_names` / `expected_arguments` extended only for the message
  text ("Float | Float[] | …").
- `call_return_type_name` stays scalar/None; **confirm its consumer**
  (`builtins/mod.rs:141`) defers to `resolve_call` when arguments are Lists, so the
  static return hint never overrides the arg-dependent array result. If a consumer
  relies on `call_return_type_name` for these names, route List args through
  `resolve_call` there.
- Match List type strings to **exactly** what `typecheck::type_name(Type::List(…))`
  emits — verify empirically before hard-coding `"List OF Float"`.

`check_math_builtin_call` needs no structural change (it already infers arg types
and delegates to `resolve_call`), but confirm length/`arity` errors and the
`TYPE_CALL_ARGUMENT_MISMATCH` path read correctly for the new arms.

### 4.3 List-build runtime helper + loop scaffold (Phase 2)

**`_mfb_simd_alloc_list(count: x0, typeCode: x1) -> ptr: x0`** — a new internal
runtime routine emitted alongside `_mfb_rng_*` (precedent: `mod.rs` RNG symbols +
`runtime.rs`). It computes `alloc = 40 + count*40 + count*8`, calls
`_mfb_arena_alloc(alloc, 8)`, writes the tight header (`kind=0, valueType=typeCode,
flagsVersion=1, count, capacity=count, dataLength=count*8, dataCapacity=count*8`),
and fills the `count` lookup entries (`flags=1, valueOffset=i*8, valueLength=8`).
Returns the list base; the data region base is then `base + 40 + count*40`. This
keeps the per-element scalar bookkeeping out of every op lowering and contains the
`_mfb_arena_alloc` clobber set behind a single, register-disciplined helper.

**Codegen scaffold** (in a new `src/target/shared/code/builder_simd_math.rs`):

- `lower_simd_unary(op_kind, in_type, out_type, &arg)` and
  `lower_simd_binary` / `lower_simd_clamp` helpers that:
  1. Lower the input list(s) to pointers; read `count@8`; for binary forms compare
     the two `count`s and `emit_invalid_argument_return` on mismatch.
  2. **Spill** input data-base pointer(s) and `count` to stack slots (arena-clobber
     discipline) and call `_mfb_simd_alloc_list(count, outTypeCode)`; reload.
  3. Run the **2-lane chunk loop**: `ldr q` from input data base + offset, apply the
     op-specific NEON sequence (a closure/enum the caller supplies), accumulate any
     per-lane error mask in a fixed V register, `str q` to output data base.
  4. **Tail** (`count & 1`): one scalar iteration reusing the existing scalar
     instruction sequence for that op.
  5. **Error reduce:** horizontal-reduce the error mask to a GPR; branch to
     `emit_overflow_return` / `emit_invalid_argument_return` if nonzero. Errors are
     reported *after* processing all lanes (no side effects; result is discarded on
     error), giving a deterministic single error regardless of which lane failed.
  6. Return `ValueResult { type_: "List OF …", location: result_ptr }`.
- **Vector register bank:** hard-code a fixed set (`v0..v7`, caller-saved) inside the
  kernel exactly as scalar lowering hard-codes `d0..d2` — the GPR allocator (no
  spilling, no V tracking) is **not** modified. Loop-index/pointer GPRs come from the
  normal allocator but must be spilled across the alloc call only (already handled in
  step 2; the loop runs entirely after allocation).
- **Empty array:** `count == 0` allocates an empty list and skips the loop — no
  error, returns the empty list.

### 4.4 Algebraic overloads (Phase 3)

Direct NEON, deterministic, no library calls:

- **`abs`**: Float → `fabs v.2d`. Integer/Fixed → `abs v.2d`; detect min-value
  overflow with `cmeq` against a broadcast `INT64_MIN` (Integer) / min-Fixed lane,
  OR-accumulate into the error mask → `ErrOverflow`.
- **`min`/`max`** (two arrays): Float → `fmin/fmax v.2d`; Integer/Fixed →
  `smin/smax v.2d`. Length mismatch → `ErrInvalidArgument`.
- **`clamp`**: `dup` the scalar `min`/`max` to both lanes; Float `fmax(fmin(x,hi),lo)`;
  Integer/Fixed `smax(smin(x,hi),lo)`.
- **`ceil`/`floor`/`round` (Float[]→Integer[])**: `frintp`/`frintm`/`frinta` then
  `fcvtzs` (ceil/floor) or `fcvtas` directly (round = ties-away). Per-lane overflow:
  compare the rounded double against the representable i64 range (mirror
  `emit_float_rounding_integer_range_check`) before/after convert, accumulate mask →
  `ErrOverflow`.
- **`ceil`/`floor`/`round` (Fixed[]→Integer[])**: integer SIMD on Q32.32 — `floor =
  sshr #32`; `ceil = sshr((x + (ONE-1)), 32)` with sign care; `round` = add/sub
  half-ONE by sign then `sshr #32`. Never overflows (integer part fits i64); no error
  path.
- **`sqrt(Float[]→Float[])`**: `fsqrt v.2d`; `fcmlt` lanes `< 0` → error mask →
  `ErrInvalidArgument`.

### 4.5 Fixed transcendentals (Phase 4)

`sqrt(Fixed[])`, `log(Fixed[])`, `log10(Fixed[])` must stay deterministic Q32.32.
These are implemented as **NEON-vectorized Q32.32 integer kernels** (Open Decision
#2) — the existing deterministic scalar Q32.32 algorithm (`builder_fixed_math.rs`)
re-expressed op-for-op over `v*.2d` integer lanes so two lanes advance in parallel.
Because integer ops are exact and the algorithm is mirrored step-for-step, each lane
is **bit-identical to the scalar Fixed result** (`f([x])[0] == f(x)`), which a
dedicated test asserts. The shared kernel definition (vector `.2d` for the loop,
scalar form for the tail and the existing scalar overload) is what preserves that
identity — the same factoring used for the Float kernels (§4.6). Looping the scalar
routine per lane is explicitly rejected: it is not SIMD. Domain check: lane ≤ 0
(`log`/`log10`) or `< 0` (`sqrt`) → `ErrInvalidArgument`, via the same per-lane
mask-reduce as the Float path.

### 4.6 Float transcendentals (Phase 5)

`exp, log, log10, sin, cos, tan, asin, acos, atan, atan2, pow` over `Float[]`. No
hardware vector instruction exists and no system vector math library is available
across the targets (Open Decision #1, now decided), so these are implemented as
**hand-written NEON `f64` polynomial kernels emitted inline by codegen — one
implementation used unchanged on macOS and Linux**, with **no external library
import** (no Accelerate, no libmvec, no scalar libm). The kernels run over the data
region in 2-lane (`v*.2d`) chunks via the Phase 2 loop scaffold, with the scalar
tail evaluating the same polynomial on a single `d`-lane.

Kernel construction (standard, well-trodden double-precision algorithms):

- **`exp`**: range-reduce `x = n·ln2 + r` (`n = round(x/ln2)`, reuse the `ln2`
  constant), evaluate `e^r` by a minimax polynomial (Horner via `fmla`), then scale
  by `2^n` constructed by adding `n` to the IEEE-754 exponent field with integer
  vector ops (`shl`/`add`). Overflow (`n` past the exponent range) → error mask.
- **`log`/`log10`**: decompose `x = 2^k · m` (extract exponent/mantissa with integer
  ops), polynomial for `log(m)`, recombine `k·ln2 + log(m)`; `log10 = log·log10(e)`.
- **`sin`/`cos`/`tan`**: Cody-Waite / Payne-Hanek range reduction to `[-π/4,π/4]`
  (reuse `twoOverPi`), minimax sine/cosine polynomials, quadrant select via integer
  mask + `bsl`; `tan = sin/cos` (`fdiv`).
- **`asin`/`acos`/`atan`/`atan2`**: polynomial `atan` core with the standard argument
  reduction and quadrant/sign assembly (integer masks + `bsl`); `asin`/`acos` via the
  `atan` identity with the `[-1,1]` domain pre-check.
- **`pow`**: `x^y = exp(y·log(x))` reusing the `exp`/`log` kernels, with special-case
  blending (`x≤0`, `y=0`, integer `y`) handled by lane-wise `bsl`.

**Accuracy target: within ≤1 ULP of macOS libm.** The reference oracle is macOS's
system libm; each kernel must land within **1 ULP** of the macOS-libm value over the
supported domain. This is the achievable, standard "faithfully rounded" bar — a
clean minimax kernel (Sollya/Remez) reaches it without having to reproduce Apple's
exact algorithm, so the coefficients can be derived independently. Coefficients are
generated offline and committed as named `f64` constants with their generator inputs
recorded beside them — auditable, not magic numbers.

**Reference capture.** A small generator run **once on the reference macOS** (this
project's Darwin 24.6 / aarch64) feeds a representative + boundary input set through
macOS libm and commits the captured `(input, expected_bits)` vectors as a data file
(precedent: the committed generated Unicode table, `[[regex-package-impl]]`). Tests
read that committed file and assert `ulp_diff(kernel, reference) ≤ 1`, so Linux/CI
validate against the macOS-libm oracle without needing a Mac, version-pinned
regardless of the runner's libm.

**≤1 ULP is the bar.** Per function, every reference input must be within 1 ULP. Any
input exceeding 1 ULP is a **tracked blocker** — resolved by improving the kernel, or
escalated for an explicit signed-off exception; never silently shipped
(`.ai/compiler.md`: prove the value, don't baseline the drift). The kernel's own
output (not libm's) is what the golden `.run` encodes, identical on every target.

**Per-lane error detection** is folded into the kernel: domain pre-checks on the
input (`asin`/`acos` lane outside `[-1,1]`, `log`/`log10` lane ≤ 0 → mask), and
overflow checks (`exp`/`pow` result past finite range → mask), reduced once to a
single `ErrInvalidArgument` / `ErrOverflow` after all lanes (§4.3).

**Factoring:** define each kernel once as a shared evaluator over a lane operand,
parameterized only by the instruction *form* (vector `v*.2d` for the loop body,
scalar `d` for the tail and for the scalar overload in §4.7). The coefficient
constants and the operation sequence are shared verbatim — that shared definition is
what guarantees `sin([x])[0] == sin(x)` and cross-platform determinism.

### 4.7 One deterministic math surface — re-point scalar transcendentals (Phase 6)

Once the kernels exist and are accuracy-validated, replace the scalar Float
transcendental lowering so scalar and array share one implementation:

- In `lower_math_call` (`builder_math.rs`), route `exp, log, log10, sin, cos, tan,
  asin, acos, atan, atan2, pow` over scalar `Float` to the **scalar-lane kernel**
  (the same evaluator the array tail uses) instead of `lower_external_math` →
  `branch_link` to libm.
- Drop the now-unused libm `platform_imports` for exactly those symbols on each
  target (the `native_call_imports` mapping at `linux_aarch64/plan.rs:370` and the
  macOS equivalent). **Keep** any libm import still needed by other functions (e.g.
  `math.fmod` if it remains libm-backed) — remove only what these transcendentals
  used. Scalar `sqrt` is already hardware `fsqrt_d` and is untouched.
- Fixed scalar transcendentals already use the deterministic Q32.32 path
  (`lower_fixed_external_math`) and are untouched; this only moves the **Float**
  scalars onto the new kernels.

This changes existing shipping behavior, so it is the **highest-risk phase** and
lands last (before docs), behind the Phase 5 accuracy tests:

- **Golden regen.** Both outputs change. The `.ir` / native output changes (the `bl
  <libm>` + relocation disappears, replaced by the inline kernel), and the `.run`
  **value may shift by up to 1 ULP** on every platform (off each platform's libm onto
  the single kernel, which sits within ≤1 ULP of macOS libm). These goldens are
  regenerated, and every new `.run` value is validated `≤1 ULP` against the committed
  macOS-libm reference before being accepted — never blindly re-baselined
  (`.ai/compiler.md`). After Phase 6, all targets emit the *same* kernel value.
- **Determinism test.** A program asserting `math::f(x) == math::f([x])[0]` for each
  function over a sample of inputs, run on both macOS and Linux flavors, proving the
  single surface holds.

## Layout / ABI Impact

- **No collection layout change.** Result arrays use the standard tight homogeneous
  numeric `List` layout (`mfb spec memory collections`); copy/transfer/scope-drop and
  golden output for existing programs are unaffected.
- **`mfb spec architecture aarch64-instruction-set`** gains the new vector `CodeOp`s,
  the V-register/arrangement operand convention, and the `_mfb_simd_alloc_list`
  internal symbol — additive; existing scalar encodings unchanged.
- **`mfb spec language builtin-functions` §18.2** math member list gains the array
  overload signatures (the names already exist; only signatures expand).
- **No new error codes.** Reuses `ErrOverflow` `77050010` and `ErrInvalidArgument`
  `77050002` (`mfb spec diagnostics error-codes`) — so no `errorCode::` registry /
  `build.rs` change.
- **External library dependency shrinks.** The Float transcendental kernels (§4.6)
  are emitted inline — no Accelerate, no `libmvec`, no added `platform_imports`; the
  only new symbol is the internal `_mfb_simd_alloc_list`. Phase 6 (§4.7) further
  **removes** the scalar libm imports for `exp/log/log10/sin/cos/tan/asin/acos/atan/
  atan2/pow` (keeping any libm symbol still used elsewhere, e.g. `fmod`). After this
  plan, Float transcendentals — scalar and array — are cross-platform deterministic
  and link no math library.
- **Behavior/golden change to existing scalar transcendentals.** Both `.ir`/native
  (libm `bl` → inline kernel) and runtime `.run` (values may shift up to 1 ULP, on
  every platform, onto the single kernel) change — an expected, validated update, not
  a regression. The determinism-policy change ("Float transcendentals are
  deterministic, equal across all targets, and within ≤1 ULP of macOS libm") is
  reflected in the spec (Doc sync below). Algebraic scalar overloads and all Fixed
  behavior are byte-unchanged.

## Phases

1. **NEON encoder.** V-register/arrangement operand decoding + the closed vector
   `CodeOp` set (§4.1), `emit_*`, `abi::` constructors, `instruction_size`. Encoder
   unit tests vs known-good words. Update `mfb spec architecture
   aarch64-instruction-set`. *Acceptance unchanged (no callers).*
2. **List-build helper + loop scaffold.** `_mfb_simd_alloc_list` runtime symbol
   (§4.3) + `builder_simd_math.rs` chunk-loop / tail / error-reduce helpers, proven
   end-to-end by wiring the single simplest op (`abs(Integer[])`) through it with a
   runtime test. *Establishes the arena-clobber discipline once.*
3. **Algebraic overloads** (§4.4): `abs`, `min`, `max`, `clamp`, `ceil`, `floor`,
   `round`, `sqrt(Float[])` for all listed element types — metadata (§4.2) +
   typecheck + lowering + full `_valid`/`_invalid`/`_rt` tests.
4. **Fixed transcendentals** (§4.5): `sqrt(Fixed[])`, `log(Fixed[])`,
   `log10(Fixed[])` as NEON-vectorized Q32.32 integer kernels (mirroring the scalar
   Q32.32 algorithm) + a `f([x])[0] == f(x)` bit-identity test vs the scalar Fixed
   routine + `_valid`/`_invalid`/`_rt` tests.
5. **Float transcendental kernels** (§4.6): `exp, log, log10, sin, cos, tan, asin,
   acos, atan, atan2, pow` over `Float[]` via hand-written NEON `f64` polynomial
   kernels (same code both platforms, no external library) + per-lane error
   detection, validated **within ≤1 ULP of the committed macOS-libm reference
   vectors**. Can land incrementally per-function (e.g. `exp`/`log` first, then trig,
   then inverse-trig/`pow`). *Array overloads only — scalar path untouched until
   Phase 6.*
6. **Unify scalar transcendentals onto the kernels** (§4.7): re-point scalar
   `exp/log/log10/sin/cos/tan/asin/acos/atan/atan2/pow` (Float) off libm onto the
   shared scalar-lane kernel, drop the now-unused libm imports, regenerate the
   affected scalar goldens against the validated reference, and add the
   `scalar == array` determinism test. *Highest risk — changes existing shipping
   behavior; lands last, behind Phase 5's accuracy tests.*
7. **Docs + acceptance sweep.** `mfb spec language builtin-functions` §18.2 array
   signatures + the Float-transcendental determinism-policy update (retire the
   "platform libm, last ULP varies" caveat), `mfb man math` regen, DOC blocks if
   applicable, full `scripts/test-accept.sh` green, runtime proofs recorded. Remove
   this plan doc in the commit that lands Phase 7 (precedent `34e526c9`).

## Validation Plan

- **Function tests** — for every overload in §1, `tests/func_math_<fn>_<elem>_valid/**`
  (e.g. `func_math_ceil_floatarray_valid`, `func_math_abs_intarray_valid`) asserting
  the result array equals element-wise scalar output, plus `_invalid/**`
  (wrong arg count, non-numeric element type, mismatched two-array lengths as a
  compile or runtime error per design, scalar-where-array-expected) and `_rt`
  runtime-error dirs for each error overload (`ceil`/`floor`/`round` Float overflow,
  `abs` Integer/Fixed min-value overflow, `sqrt`/`asin`/`acos`/`log` domain, `exp`
  overflow). Empty-array and odd-length (tail-path) cases included.
- **Runtime proof:** a program that builds a known `Float[]`/`Integer[]`/`Fixed[]`,
  calls each overload, and prints results whose values prove real per-lane
  computation (not zeros/defaults) — including an odd length to exercise the scalar
  tail and a length that crosses several 2-lane chunks to catch the
  arena-clobber/length-threshold class of bug (`.ai/compiler.md`).
- **Encoder proof:** Phase 1 `#[cfg(test)]` tests asserting each new instruction's
  exact little-endian word.
- **Kernel accuracy (Phase 5):** the oracle is **macOS libm**. A generator run once
  on the reference macOS captures `(input, expected_bits)` vectors over a
  representative + boundary input set and commits them as a data file; per-function
  tests assert `ulp_diff(kernel, reference) ≤ 1` for every input. Any input exceeding
  1 ULP is enumerated as a tracked blocker, not absorbed into a re-baselined golden.
  Tests run on macOS and both Linux flavors against the same committed reference.
- **One-surface determinism (Phase 6):** a test asserting `math::f(x) ==
  math::f([x])[0]` **bit-identically** for every re-pointed transcendental (scalar and
  array share one kernel — this is exact, 0 ULP), run on macOS and both Linux flavors
  to prove the single cross-platform surface. The re-pointed scalar `.run` goldens
  shift up to 1 ULP off libm and are validated `≤1 ULP` against the macOS-libm
  reference, never blindly re-baselined.
- **Doc sync:** `mfb spec language builtin-functions` §18.2 (array signatures) and
  the Float-transcendental determinism-policy change; `mfb spec architecture
  aarch64-instruction-set`; `mfb man math` regenerated.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  green after each phase that can affect AST/IR/native output.

## Open Decisions

All resolved; each is recorded with its rationale so the choice is auditable during
implementation. Where a tie was close, the deciding rule was **most-correct over
easiest** — notably #2 (real SIMD, not a scalar loop) and #6 (no out-of-bounds read).

1. **Float transcendental strategy** — **DECIDED: hand-written NEON `f64` polynomial
   kernels, the same code on macOS and Linux, no external library.** Rationale: no
   system vector math library spans the targets — macOS has Accelerate/`vForce`, but
   Linux `libmvec` is glibc-only with AArch64 vector symbols only in glibc ≈2.40
   (mid-2024), and musl has none (the compiler links scalar `libm.so.6`/`libm.so.1`,
   `plan.rs:22`). Since Linux must hand-roll regardless, sharing that kernel with
   macOS yields one implementation, a dependency-free link, and bit-identical Float
   results everywhere. *Rejected:* platform vector libraries (don't span Linux/musl);
   scalar-libm loop (not parallel — contradicts "SIMD hardware"). Cost accepted: the
   polynomial-coefficient accuracy/test work in Phase 5. (§4.6)
2. **Fixed transcendentals** — **DECIDED: NEON-vectorized Q32.32 integer kernels.**
   This is the most-correct choice against the directive ("all use SIMD low-level
   asm"): looping the existing scalar routines is *not* SIMD — it processes one lane
   at a time and contradicts the feature's premise. The vectorized kernel mirrors the
   existing deterministic Q32.32 integer algorithm op-for-op over `v*.2d` integer
   lanes, so it stays **bit-identical to the scalar Fixed result** (`f([x])[0] ==
   f(x)`) while actually running lanes in parallel. *Rejected:* scalar loop (simpler
   but not SIMD). Cost accepted: more codegen + a determinism test vs the scalar
   Fixed routine. (§4.5)
3. **Result construction** — **DECIDED: the `_mfb_simd_alloc_list` runtime helper**
   (§4.3). Most correct on safety grounds: it confines the `_mfb_arena_alloc`
   register-clobber discipline (the layout-sensitive bug class in `.ai/compiler.md`)
   to one audited routine instead of duplicating it across ~20 lowerings, minimizing
   the surface where a live pointer/count can be silently corrupted. *Rejected:*
   fully inline construction (more duplication, more clobber surface).
4. **Error-on-any-lane reporting** — **DECIDED: process all lanes, reduce a per-lane
   error mask, raise one error.** Most correct: it yields a **deterministic** single
   error independent of which lane failed and keeps the hot loop branch-free; it is
   safe because NEON FP ops don't hardware-trap under the default FPCR (out-of-range
   lanes produce NaN/Inf/wrapped values that the mask detects) and the result array is
   discarded on error, so there are no side effects to unwind. *Rejected:*
   short-circuit at first failing lane (data-dependent error identity, branchy loop).
   (§4.3)
5. **Two-array length mismatch** (`min`/`max`/`pow`/`atan2`) — **DECIDED: runtime
   `ErrInvalidArgument`.** This is the only correct option: `List` lengths are dynamic
   and not part of the type, so the check cannot live at compile time. Confirmed the
   language has no compile-time length typing. (§4.4)
6. **Tail handling** (odd final lane) — **DECIDED: evaluate the final lane with the
   shared scalar-form kernel.** Most correct, not merely simpler: the masked-vector
   alternative would load a full 128-bit chunk on the last iteration and thus **read
   one element past the data region** (an out-of-bounds read) unless inputs are
   over-allocated/padded — a real memory-safety hazard. The scalar tail reads exactly
   the valid element and reuses the identical kernel (the same one Phase 6's scalar
   overload uses), so its result is bit-identical to a vector lane. *Rejected:* masked
   single-lane vector op (OOB read risk; needs padding to be correct). (§4.3)
7. **Accuracy bar = within ≤1 ULP of macOS libm** (DECIDED). macOS libm is the
   reference oracle; kernels must land within 1 ULP of it. This is the standard
   faithfully-rounded bar, reachable with an independent minimax kernel — no need to
   reproduce Apple's exact algorithm. Trade-off accepted: Phase 6's scalar re-point
   shifts macOS scalar `.run` values by up to 1 ULP (expected, validated golden
   churn), and `f(x) == macOS-libm(x)` holds only to ≤1 ULP. The two exact (0 ULP)
   invariants are retained regardless: `f(x) == f([x])[0]` and identical results on
   every target. *Rejected:* bit-exact/0 ULP vs libm (would avoid macOS scalar churn
   but demands mirroring Apple's libm algorithm — disproportionate effort). (§4.6,
   §4.7)

## Non-Goals

- No SIMD for non-numeric lists (String/Byte/record/nested) — element type must be
  `Integer|Float|Fixed`.
- No auto-vectorization of scalar loops or of `collections::transform` — only the
  explicit `math::` array overloads.
- No user-visible vector/SIMD type, no FMA/dot-product/reduction builtins beyond the
  listed signatures, no `f32`/lane-width selection surface.
- No x86/AVX backend (single-target compiler).

## Summary

The real engineering risk is front-loaded into **Phase 1**: there is no NEON encoder
today, so the vector instruction set, V-register operand model, and their exact bit
encodings must be built and unit-proven before any overload can lower — a wrong
encoding constant corrupts silently. The second risk is the **arena-clobber register
lifetime** across result allocation (Phase 2), contained behind one runtime helper
and one scaffold. Phase 3 (algebraic overloads) is where genuine, deterministic
inline SIMD delivers most of the value with direct NEON instructions. Phases 4–5
(transcendentals) have no hardware vector instruction, so they are hand-written NEON
`f64` polynomial kernels for Float (the same code on macOS and Linux, no external
library — because no system vector math library spans the targets) and deterministic
Q32.32 for Fixed; the residual risk there is polynomial accuracy and coefficient
provenance, not platform availability. The kernels are validated to **within ≤1 ULP of
macOS libm** (the current scalar backend), captured once on the reference macOS into
a committed reference file — a concrete oracle and an achievable faithfully-rounded
bar. Phase 6 then collapses scalar and array onto **one deterministic math surface**
by re-pointing the scalar Float transcendentals off libm onto the same kernels. That
re-point shifts macOS scalar values by up to 1 ULP (expected, validated golden
churn) and brings Linux onto the same kernel. It is the highest-behavioral-risk step
— it touches shipping output — which is why it lands last, behind the Phase 5
accuracy proof. The payoff: `sin(x)` and `sin([x])[0]` are bit-identical on every
target and within ≤1 ULP of macOS libm, and the compiler links no math library for
those functions. Untouched throughout: scalar *algebraic* `math::` behavior and
goldens, all Fixed behavior, the List layout/ABI, value/copy/transfer semantics, the
error-code registry, and the single-architecture backend shape.
