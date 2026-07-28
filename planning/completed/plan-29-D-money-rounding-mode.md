# plan-29-D: Money — configurable rounding mode & the `money::` package

Last updated: 2026-07-07
Effort: medium (1h–2h)

This sub-plan adds a **runtime-configurable rounding mode** for Money **arithmetic** and
the `money::` package that controls it. The default is round-half-away-from-zero
(commercial rounding); a program may switch to round-half-to-even (banker's rounding)
and query the current mode. Every **arithmetic** rounding operation in plan-29-E/F (and
the *conversion* kernels of plan-29-G) consults this mode through one shared codegen
helper. **Note:** `toString(Money)` presentation rounding is *decoupled* — it uses a
fixed half-away-from-zero rule and does **not** call this helper (plan-29-G §4.1
decision), so `toString` stays a pure function of its inputs. The mode governs how
arithmetic settles cents, not how values are displayed. It depends on plan-29-A (the Money type) and is a
prerequisite of plan-29-E/F/G (the arithmetic and conversion kernels that round). It has
no dependency on plan-29-C's storage work, but the three of C, D, E land together (per
the "land C and D together" decision) so `builder_numeric`'s Money dispatch is never
half-built.

It complements:

- `./mfb spec memory` (a new per-arena state field for the rounding mode, mirroring the RNG state)
- `./mfb man money` (the new package)

## 1. Goal

- A per-execution-context **rounding mode** flag lives in the arena state region,
  initialized to `Commercial` (round-half-away-from-zero).
- `money::Rounding` is an enum with members `Commercial` and `Banker`.
- `money::setRounding(mode AS Rounding)` sets it; `money::getRounding() AS Rounding`
  reads it. `money::setRounding(money::Rounding.Banker)` then `money::getRounding()`
  returns `Banker`.
- `money::round(value AS Money, decimals AS Integer) AS Money` — **approved
  (2026-07-11)** — explicitly settles an amount to `decimals` places **under the
  current mode** ("compute at 5 places, book at 2"): `money::round(x, 2)` settles to
  whole cents. `decimals` 0..5 valid (`5` is the identity); outside →
  `ErrInvalidArgument`. This is *arithmetic* settling (mode-aware), distinct from
  `toString`'s fixed presentation rounding.
- A shared codegen helper `emit_apply_rounding(...)` branches on the mode
  (Commercial → half-away-from-zero, Banker → half-to-even) — the single site every
  Money rounding kernel (plan-29-E/F/G) calls, so the two modes are implemented once.

### Non-goals (explicit constraints)

- No arithmetic kernels here (they live in plan-29-E/F and consume this). This sub-plan
  ships the mode storage, the package API, and the shared rounding helper — the helper is
  unit-tested but has its first live callers in E.
- No modes beyond `Commercial` (half-away) and `Banker` (half-even) in v1. (Truncate /
  ceil / floor modes can be added later as enum members + helper branches.)
- No change to non-Money rounding anywhere (`Fixed`/`Float` `toString`, `toFixed`, etc.
  keep their current fixed behavior). This mode governs **Money** rounding only.
- No new runtime error codes.

## 2. Current State

Precedent for a runtime-global getter/setter builtin: `io::isBuffered`/`io::setBuffered`
(`src/builtins/io.rs:8-9,58,67,71,87`) — a `Nothing`-returning setter and a value getter
backed by runtime state. Precedent for a **source companion package + runtime
intrinsics**: `datetime::` (`src/builtins/datetime.rs`, registered in
`src/builtins/mod.rs:5,33,281,316,362,485`; a `.mfb` source package plus libc
runtime-helper intrinsics — see the datetime memory). Per-execution-context state lives
in the arena state region with typed byte offsets in
`src/target/shared/code/error_constants.rs` (`TERM_STATE_*_OFFSET` 0–40,
`ARENA_CLEANUP_FAILURE_*` 64–80, `ARENA_RNG_STATE_LO/HI_OFFSET` 88/96,
`ARENA_QUICK_BIN_BASE_OFFSET` 104, `ARENA_STATE_SIZE` derived at `:209`); the RNG state is
seeded at arena/thread init (math-rng memory: OS-entropy main seed, parent-drawn thread
seed). `ARENA_STATE_REGISTER` addresses the region in codegen
(`builder_codegen_primitives.rs`, `builder_emit_helpers.rs:382-405`, including the
thread-transfer copy of parent arena state).

## 3. Design Overview

Three pieces:

1. **Mode storage** — one arena-state field (`ARENA_ROUNDING_MODE_OFFSET`, a free slot
   such as 48/56 or an extension of the region), holding `0 = Commercial` / `1 = Banker`,
   zero-initialized at arena init (so the default is Commercial with no extra init code).
   At thread spawn the child inherits the parent's mode (copied like the RNG seed
   derivation), then diverges independently — consistent with thread isolation.

2. **`money::` package** — a source companion package (`src/builtins/money_package.mfb`,
   mirroring `datetime`) declaring `ENUM Rounding { Commercial, Banker }` and thin
   `setRounding`/`getRounding` wrappers over two runtime intrinsics
   (`_mfb_rt_money_set_rounding`, `_mfb_rt_money_get_rounding`) that write/read the
   arena-state field. Registered in `src/builtins/mod.rs` alongside `datetime`. The enum
   discriminants (`Commercial = 0`, `Banker = 1`) are exactly the stored values.

3. **Shared rounding helper** — `emit_apply_rounding`, in a new codegen module
   `src/target/shared/code/builder_money_math.rs` created here (the module the
   plan-29-E/F **arithmetic** kernels call into, mirroring how `builder_numeric`'s
   `emit_fixed_binary` calls `builder_fixed_math`) that, given a truncated quotient, the
   remainder, and the divisor magnitude, loads the mode field and emits the correct
   half-adjustment: Commercial rounds away from zero when `2*|rem| >= |div|`; Banker does
   the same **except** it rounds to even when `2*|rem| == |div|`. For Float-domain kernels
   (plan-29-F) the same choice maps to `llround` (away) vs `nearbyint`/round-half-even.
   (`toString` does **not** call this helper — its presentation rounding is fixed
   half-away-from-zero, decoupled from the mode; plan-29-G §4.1.)

Correctness risk: the mode field must be read at the point of each rounding (not cached
across a `setRounding`), and the Banker tie case (`2*|rem| == |div|`, round to even) must
be exactly right. Both are covered by the shared helper + its unit tests.

## 4. Detailed Design

### 4.1 Mode storage (`error_constants.rs` + arena init)
- Add `pub(crate) const ARENA_ROUNDING_MODE_OFFSET: usize = <free slot>;` (place it in an
  unused gap or extend `ARENA_STATE_SIZE`; keep every existing offset unchanged).
- Zero-init at arena init (Commercial = 0 needs no explicit store if the region is
  zeroed; otherwise store 0). At thread spawn, copy the parent's field into the child
  (beside the RNG-seed derivation in the thread-init/arena-state-copy path,
  `builder_emit_helpers.rs:382-405`).

### 4.2 `money::` package (`src/builtins/money_package.mfb` + `money.rs` + `mod.rs`)
```
ENUM Rounding
  Commercial      ' round half away from zero (default)
  Banker          ' round half to even
END ENUM

SUB setRounding(mode AS Rounding)          ' -> _mfb_rt_money_set_rounding(discriminant)
FUNC getRounding() AS Rounding             ' -> Rounding from _mfb_rt_money_get_rounding()
FUNC round(value AS Money, decimals AS Integer) AS Money   ' mode-aware settle (§4.4)
```
- `src/builtins/money.rs`: package metadata (name `money`, the two callables' arities /
  return types), mirroring `io.rs`/`datetime.rs`; register in `src/builtins/mod.rs`
  (module decl + the `is_builtin`/return-type/param-name dispatch arms at the datetime
  sites).
- Runtime intrinsics: `_mfb_rt_money_set_rounding(i64 mode)` stores `mode & 1` into
  `ARENA_ROUNDING_MODE_OFFSET`; `_mfb_rt_money_get_rounding() -> i64` loads it. Emit these
  as vreg-able helpers (or inline the load/store, since it is a single arena-state
  access — simpler than a call). Enum member `money::Rounding.Commercial` /
  `money::Rounding.Banker` resolves through the normal package-qualified-enum path
  (surface spelling uses `.` for the member, per the enum-access grammar).

### 4.3 Shared rounding helper (`emit_apply_rounding`, new `builder_money_math.rs`)
Signature (conceptual): given `quotient` (truncated toward zero), `remainder`, and
`abs_divisor`, plus the result sign, emit code that:
- loads `ARENA_ROUNDING_MODE_OFFSET`;
- computes `twice = 2 * |remainder|`;
- if `twice > |div|` → round away (quotient ± 1 toward sign);
- if `twice == |div|` → **Commercial**: round away; **Banker**: round toward even
  (increment only if the truncated quotient is odd);
- if `twice < |div|` → keep quotient.
Return the adjusted quotient. Unit-test all three branches × both modes × both signs.

### 4.4 `money::round(value, decimals)` — explicit mode-aware settle
The first live caller of `emit_apply_rounding`. Kernel (own-lowering builtin or a
vreg-able helper — implementer's choice; the mode must be read at call time):
- `decimals` outside `0..5` → `ErrInvalidArgument`; `decimals == 5` → identity.
- `divisor = 10^(5 - decimals)` (from a 5-entry constant table); truncating signed
  divide `raw / divisor` with remainder; `emit_apply_rounding(quotient, rem, divisor,
  sign)`; result × `divisor` back to raw. Exact integer arithmetic throughout; the
  re-multiply cannot overflow (|quotient| ≤ |raw|/divisor + 1 keeps the product within
  one `divisor` of the original raw, well inside i64).
- Tests: the tie case (`money::round(0.125m, 2)`) settles differently under
  `Commercial` (0.13) vs `Banker` (0.12), proving the mode is consulted; both signs;
  `decimals` 0 and 5; invalid `decimals` fails.

## Layout / ABI Impact

`mfb spec memory` gains one arena-state field (`ARENA_ROUNDING_MODE_OFFSET`) and, if the
region is extended, a new `ARENA_STATE_SIZE`. Document it beside the RNG-state fields.
No change to any Money value layout or `.mfp` format. Because the field is zero-init and
only read by Money kernels, existing non-Money goldens are byte-identical (the arena-state
size change, if any, is internal and not observable in program output — but re-run the
artifact gate to confirm determinism).

## Phases

### Phase 1 — mode storage + `money::` package API
The rounding mode is settable/gettable and defaults to Commercial.

- [ ] `error_constants.rs`: `ARENA_ROUNDING_MODE_OFFSET`; zero-init + thread-spawn copy.
- [ ] `money_package.mfb` (Rounding enum + setRounding/getRounding); `money.rs` metadata;
      register in `builtins/mod.rs`; `_mfb_rt_money_set_rounding`/`_get_rounding` codegen.
- [ ] Tests: `func_money_setRounding_{valid,invalid}/**`, `func_money_getRounding_valid/**`.

Acceptance: an executed program — `money::getRounding()` returns `Commercial` by default;
after `money::setRounding(money::Rounding.Banker)`, `getRounding()` returns `Banker`;
round-trips through the runtime field on both backends.
Commit: —

### Phase 2 — shared rounding helper + `money::round`
`emit_apply_rounding` implements both modes; `money::round` is its first live caller.

- [ ] New `src/target/shared/code/builder_money_math.rs` with `emit_apply_rounding` per
      §4.3 (the module plan-29-E/F's kernels call into).
- [ ] Unit tests: all three magnitude branches × {Commercial, Banker} × {+, −}, e.g.
      half-case `2.5`-equivalent → Commercial rounds away, Banker rounds to even.
- [ ] `money::round(value, decimals)` per §4.4 (package registration beside
      setRounding/getRounding; kernel over `emit_apply_rounding`; `decimals` guard).
- [ ] Tests: `func_money_round_{valid,invalid}/**` incl. the Commercial-vs-Banker tie
      divergence from §4.4.

Acceptance: the helper returns the correct adjusted value for every branch/mode/sign in
unit tests; an executed program shows `money::round(0.125m, 2)` differing under the two
modes on both backends.
Commit: —

## Validation Plan

- Function tests: `func_money_{setRounding,getRounding}_*`.
- Runtime proof: the get/set/default round-trip program (Phase 1); the mode-affects-result
  proof lands in plan-29-E (e.g. a `M / k` whose tie rounds differently under each mode).
- Doc sync: `mfb spec memory` (the arena-state field); `mfb man money` (Phase, in
  plan-29-G's doc sweep or here — land the man page here).
- Acceptance: `scripts/test-accept.sh …`; `scripts/artifact-gate.sh` for arena-state
  determinism.

## Open Decisions

- **Mode scope / thread inheritance** — *recommend per-execution-context state, child
  inherits the parent's mode at spawn* (copied like the RNG seed), then independent —
  consistent with thread isolation. Alternative: reset each thread to Commercial
  (simpler, but a worker silently ignores the parent's policy). (§4.1)
- **`Commercial` member name** — the user wrote `commercial`; MFB convention is
  CapitalCamelCase enum members, so *recommend `Commercial`*. (§4.2)
- **`money::round(m AS Money, decimals AS Integer) AS Money` — APPROVED (2026-07-11).**
  Explicit, mode-aware settling of an amount to `decimals` places (§4.4): the mode's
  natural explicit companion to its implicit use inside arithmetic — invoice line
  items, allocation remainders, "compute at 5 places, book at 2." Distinct from
  `toString`'s fixed presentation rounding, which is unchanged. Note the contrast with
  `math::round(Money) → Integer` (plan-29-G §4.7): `math::round` is the dimensionless
  whole-unit count with the fixed half-away rule; `money::round` stays Money and
  follows the mode.

## Summary

A small, self-contained subsystem: one arena-state field, a `money::` package
(`Rounding` enum, `setRounding`/`getRounding`, mode-aware `round(value, decimals)`),
and one shared rounding helper that centralizes the half-away/half-even choice. It
carries all the rounding *policy* plus its one explicit consumer; plan-29-E/F/G carry
the arithmetic kernels that call it. Risk is confined to the Banker tie case and reading the mode at
each rounding site — both unit-tested here.
