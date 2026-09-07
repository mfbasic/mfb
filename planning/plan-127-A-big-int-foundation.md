# plan-127-A: the `big` package — the `Int` value, its native seams, and comparison

Last updated: 2026-09-06
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

`big` is a new built-in package providing arbitrary-precision signed integers as a
copyable value record with natively-lowered arithmetic. This letter lands the package,
the `big::Int` type, its canonical form, the native access foundation every later
letter builds on, and the eleven members that need no multi-limb carry: four conversion
seams and seven comparison/sign members.

Behavioral outcome for this letter: a program can `IMPORT big`, build a `big::Int` from
an `Integer` or from a `List OF Byte` in either byte order, convert it back, and compare
two of them — with every operation except `toInteger` total (no `TRAP`, no `AS Error`).

References — read these first:

- `src/docs/spec/memory/05_collections.md` — the kind-2 fixed-width list layout.
- `src/docs/spec/memory/03_heap-values.md` — record slot layout and field inlining.
- `src/docs/spec/language/04_types.md` — comparability, orderability, defaultability.
- `src/codegen/builtins/net/mod.rs` — the package-owned value-record precedent.
- `src/codegen/builtins/bits/func_clz.rs` — the arch-neutral native lowering precedent.
- `.ai/codegen-invariants.md` — arch-neutral codegen/IR/regalloc invariants.
- `.ai/arch-abi.md` — per-architecture traps the `abi::` layer does not hide.
- `.ai/resources-packages.md` — builtin-package authoring seams.
- `.ai/man-content.md` — the man-page content standard.

## Prerequisites

This feature is self-contained by design. It depends on no other planning document, no
bug fix, and no in-flight work; it cites only shipped source and the embedded spec.

| Must be true | Command | Status |
|---|---|---|
| The `big` package name is unused | `grep -c '"big"' src/codegen/registry/mod.rs` → `0` | MET |
| No `src/codegen/builtins/big/` exists | `ls src/codegen/builtins/ \| grep -c '^big$'` → `0` | MET |
| Tree builds and tests green at HEAD | `cargo test --no-fail-fast` | UNMEASURED — run before Phase 1 |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites** — not only the one
> that blocked you.

## 1. Goal

- `IMPORT big` resolves; `big::Int` is an exported, defaultable value record whose
  default value is canonical zero.
- `big::fromInteger`, `big::fromBytes`, `big::toBytes`, `big::compare`, `big::equals`,
  `big::isZero`, `big::sign`, `big::abs`, `big::negate` are **total** — none declares
  an error.
- `big::toInteger` is this letter's only fallible member (`ErrOverflow`).
- `big::toBytes(big::fromBytes(b, n, e), e)` returns `b` for every canonical `b`, in
  both byte orders.

### Non-goals (explicit constraints)

Guardrails; violating one means the implementation is wrong.

- **No `big::Dec`**, no BigFloat / BigFixed / BigMoney. One type ships here.
  Arbitrary-precision decimal is not designed in this feature and must not be
  half-built.
- **No operator support.** `+`, `-`, `*`, `/`, `<`, `>`, `=`, `<>` do not work on a
  `big::Int`, now or later. This follows from the language, not a preference:
  `is_comparable_seen` rejects a `ParameterType::ListOf(_)` field
  (`src/ir/verify/values.rs:1174-1181`) and records are never orderable
  (`src/docs/spec/language/04_types.md:473`).
- **No change to the numeric tower.** `Integer`, `Byte`, `Float`, `Fixed`, `Money`
  semantics, promotion rows, literal grammar and range checks are untouched. `big::Int`
  is not a numeric type and gains no promotion row and no literal syntax.
- **No new lowering variant.** Members use `abi_inline` / `abi_function` only.
- **No MFBASIC-source implementation.** The arithmetic is native. A helper written as
  MFB source is a defect in this feature, not a shortcut.
- **No `LINK` binding and no vendored library.**
- **No change to `packages/`.** No package is edited by any letter of this feature.
- **`big` is not a cryptographic primitive.** Nothing here is constant-time. That is a
  documented property enforced by advisory prose, not a gap to quietly close later.

## 2. Current State

There is no arbitrary-precision integer anywhere in the language or the built-in
packages. `Integer` is 64-bit and its arithmetic traps on overflow
(`src/docs/spec/language/04_types.md:9`); `Money` is a 64-bit carrier scaled to five
decimal places, bounded at `±92233720368547.75807`
(`src/docs/spec/language/04_types.md:50`). A program needing a value past those bounds
has no option, and `crypto::randomInt` documents a hard 64-bit span ceiling as a result
(`src/codegen/builtins/crypto/func_random_int.rs:17-21`) — the gap plan-127-D closes.

Two existing precedents shape the design:

- **`net::Address` / `net::Url`** (`src/codegen/builtins/net/mod.rs:186-215`) — a
  package-owned, exported, copyable value record registered with `add_record`,
  documented through its `description` and per-prop `description`, with copy semantics
  and no `RES` handle. `Url` also establishes that such a record may carry a canonical
  contract its fields cannot enforce (`scheme` is documented "lowercased" with nothing
  checking it) — exactly `big::Int`'s position.
- **`bits`'s native lowerings** (`src/codegen/builtins/bits/func_clz.rs:67-84`) — a
  lowering is a single arch-neutral function taking `&mut CodeBuilder`, `&[ValueResult]`
  and `&AbiCtx`, emitting through the `abi::` vocabulary. **Written once, not per
  target.** This is the shape every `big` member takes.

Naming follows `datetime`, which spells its comparison members
`compare(a, b) AS Integer` (`src/codegen/builtins/datetime/func_compare.rs:81,104`) and
`equals(a, b) AS Boolean` (`func_equals.rs:76,99`) — the latter existing for the same
reason `big` needs it.

### Measured populations

| What | Count | Command |
|---|---|---|
| Built-in packages registered | 31 | `grep -c "::register(&mut r)" src/codegen/registry/mod.rs` → 31 |
| `bits` members (all fixed-width `Integer`→`Integer`) | 17 | `ls src/codegen/builtins/bits/func_*.rs \| wc -l` → 17 |
| `crypto` members | 21 | `ls src/codegen/builtins/crypto/func_*.rs \| wc -l` → 21 |
| Acceptance fixtures under `tests/acceptance/src/` | 20 | `ls tests/acceptance/src/ \| wc -l` → 20 |
| Members this letter adds | 11 | §4.4 |
| Members the whole `big` feature adds | 30 + 1 record | letters A/B/C member lists |

### Verified properties

Each claim below was established by reading the named code, not by citation alone.

- **A native lowering is arch-neutral and written once.** VERIFIED by reading
  `lower_bits_clz` (`src/codegen/builtins/bits/func_clz.rs:67-84`): it type-checks its
  argument, allocates a register from the builder, and emits one `abi::` operation. No
  per-target branch. Per-arch divergence lives below `abi::`, documented in
  `.ai/arch-abi.md`. **This sets the effort for all four letters.**
- **A `List OF Byte` is a kind-2 fixed-width list: one byte per element, no
  `LookupEntry` array, element `i` at `Data[i * 1]`.** VERIFIED:
  `src/docs/spec/memory/05_collections.md:57-72` — the payload-width table gives
  `Boolean`/`Byte` width 1, and kind 2 "carries **no `LookupEntry` array at all**". The
  magnitude is therefore a packed contiguous byte array a lowering can address directly.
- **The collection header is 40 bytes**, laid out `U8 kind, U8 keyType, U8 valueType,
  U8 flagsVersion, U8 bucketsReady, U8[3] reserved, U64 count, U64 capacity, U64
  dataLength, U64 dataCapacity` — so `count` is at `+8`, `dataLength` at `+24`, and for
  kind 2 the packed data begins at `+40`. VERIFIED:
  `src/docs/spec/memory/05_collections.md:24-56`, which also warns that implementations
  "must not derive the runtime entry stride from the sum of field sizes without
  accounting for padding and alignment" — Phase 1 confirms the offsets against the
  layout builder rather than trusting this reading.
- **The magnitude field inlines into the record's own block.** VERIFIED:
  `src/docs/spec/memory/03_heap-values.md:63-66` — a `List` whose payloads are all flat
  is a flat composite, inlined by block-relative offset into the record's data region,
  and the field read recovers `recordBase + offset`. A whole-record copy is one
  `memcpy`.
- **A record with a `List OF Byte` field is not comparable.** VERIFIED by reading
  `is_comparable_seen` (`src/ir/verify/values.rs:1163-1205`): `ParameterType::ListOf(_)`
  returns `false` at 1174-1181, and a record's verdict is the AND over its field types
  at 1201-1203. This is why `compare`/`equals`/`isZero`/`sign` are functions.
- **A `List OF T` field is always defaultable to the empty collection.** VERIFIED:
  `src/docs/spec/language/04_types.md:465`. With `Boolean` defaulting to `FALSE`, the
  default `big::Int` is `{[], FALSE}` — canonical zero, with no initialization seam.
- **A record named `Integer` collapses onto the built-in scalar.** VERIFIED:
  `ParameterType::parse("Integer")` returns `ParameterType::Integer`
  (`src/types.rs:666`), and the comment at `src/types.rs:339-350` states a declared
  `TYPE Integer` must denote the same type an `AS Integer` annotation denotes. Hence
  `Int`, not `Integer`.
- **Overload resolution selects on concrete named types.** VERIFIED by reading
  `match_overload` (`src/codegen/registry/mod.rs:514-542`): it returns the first
  implementation whose params `unify` with the call's argument types. Unused by this
  letter; load-bearing for plan-127-D's `crypto::randomInt` overload.
- **UNVERIFIED — the sign convention of MFBASIC's `MOD` for negative operands.** Not
  needed here. plan-127-C measures it before `big::remainder` is specified; do not
  assume truncated semantics.

## 3. Design Overview

Three layers, bottom-up:

1. **The value.** `big::Int` is `{ magnitude AS List OF Byte, negative AS Boolean }` —
   little-endian magnitude, no trailing zero bytes, empty exactly when zero, `negative`
   false when the magnitude is empty.
2. **The native access foundation** — a small set of shared emitters in `gen_big.rs`
   that every member's lowering calls: resolve a `big::Int` argument to
   `(dataBase, count, negative)`, and allocate + populate a result `Int` from a
   scratch buffer. This letter lands them; later letters only consume them.
3. **The members**, each an `abi_function` lowering plus its registry prose.

**Where design uncertainty concentrates: the runtime layout the lowerings address.**
Every member in every letter reads a magnitude through the same two offsets. If the
header offsets or the record's block-relative field convention are misread, all 30
members are wrong in the same way, and the failure is silent memory misreads rather
than a compile error. So Phase 1 is a single narrow probe that pins the layout against
the real layout builder before any member exists.

**Where correctness risk concentrates: not in this letter.** There is no carry
propagation and no division here. Risk lands in plan-127-B (carry/borrow) and
plan-127-C (Knuth Algorithm D), which is why they are later letters behind tests.

**Byte-identity is not this plan's correctness gate.** This is new surface; behavior
changes by construction and new goldens are created. Byte-identity is used in one
narrow valid role — a **non-disturbance** check: a program that does not `IMPORT big`
must produce byte-identical `.ncode` before and after each phase. A diff there is a
registration bug, to be root-caused by objdumping one fixture and fixed; it is never a
signal the design is dead. `.ir` goldens for packages whose descriptors render **are**
expected to churn when the registry gains a package; that churn is the plan working.

### Rejected alternatives

Recorded so they are not re-litigated mid-implementation.

- **A single `String` field holding decimal digits.** The default record is `{""}`, and
  `""` is not `"0"`, so the language's zero value would not be a canonical value.
  Base-10 also forces an O(n²) conversion at every native boundary.
- **`magnitude AS List OF Integer` (64-bit limbs).** An MFBASIC `Integer` is signed, so
  a full-width unsigned limb requires specifying what the sign bit means; storage is
  the same 512 bytes for a 4096-bit value either way
  (`05_collections.md:66-72` gives width 8 vs width 1). Byte limbs keep every partial
  product ≤ 65025.
- **Two's-complement bytes in one field, no `negative`.** Canonicalization becomes
  "strip redundant sign-extension bytes honouring the next byte's high bit" instead of
  "strip trailing zeros", for no gain: the invalid state it removes (`{[], TRUE}`) costs
  one branch, and canonicity is not load-bearing for equality here because the record is
  not comparable at all.
- **A `RES` resource handle.** Every intermediate would need a `close`, making
  accumulation unusable, and a resource can never be promoted to a value later.
- **Bare `List OF Byte` with the arithmetic under `bits::`.** `List OF Byte` is the
  universal binary type in this tree (crypto hashes, keys, signatures, file contents),
  so overloading it to also mean "bignum" removes the compiler's ability to reject
  passing a digest to a multiply. All 17 `bits` members are fixed-width
  `Integer`→`Integer`; arbitrary-precision arithmetic does not belong under that name.

## 4. Detailed Design

### 4.1 The record

```basic
EXPORT TYPE Int
  magnitude AS List OF Byte
  negative  AS Boolean
END TYPE
```

**Canonical form.** `magnitude` is the absolute value, little-endian, with no trailing
zero bytes; it is empty exactly when the value is zero; `negative` is `FALSE` whenever
`magnitude` is empty.

**Field order is contract.** `net/mod.rs:287` records the same rule for `PingResult`
("`gen_ping` builds this record by writing five consecutive 8-byte slots at the offsets
these declarations fix"). Every `big` lowering constructs an `Int` at these offsets;
the `add_record` call carries a comment saying so.

**The decoder is total, deliberately.** An exported record can be hand-constructed
(`big::Int[[9, 0, 0], TRUE]`), so canonical form cannot be enforced. Validating on entry
would make every member fallible and destroy the total-arithmetic property that
motivates the whole design. Therefore:

- a non-canonical `magnitude` (trailing zero bytes) reads as the value it denotes;
- `{[], TRUE}` — negative zero — reads as zero.

Neither is an error. This mirrors `net::Url`'s unenforceable `scheme` contract.

### 4.2 Runtime layout the lowerings address

A `big::Int` argument arrives as a pointer to the record block. Per
`03_heap-values.md:50-66`:

- **slot 0** (`recordBase + 0`) — a `U64` block-relative offset; the magnitude
  collection block is at `recordBase + thatOffset`.
- **slot 1** (`recordBase + 8`) — `negative`, the value inline.

Within the magnitude block, per `05_collections.md:24-72` (kind 2):

- `count` at `+8` — the byte length of the magnitude;
- packed data at `+40` — the bytes themselves, one per element.

Phase 1 pins all four of these against
`src/codegen/collection/layout/builder_collection_layout.rs` rather than against this
reading of the spec.

### 4.3 The native access foundation (`gen_big.rs`)

Three shared emitters, consumed by every member in every letter:

- `emit_load_int(builder, arg) -> (dataBase, count, negative)` — resolves a `big::Int`
  argument to a data pointer, a byte count, and the sign flag, per §4.2.
- `emit_alloc_magnitude(builder, byteCount) -> dataBase` — allocates a kind-2
  `List OF Byte` collection of the given capacity and returns its data base.
- `emit_build_int(builder, dataBase, count, negative) -> ValueResult` — **normalizes**
  (walks down from the high byte skipping zeros, forces `negative` false on a zero
  result) and constructs the record. Every member returns through this; it is the single
  point where canonical form is established.

Placing normalization inside `emit_build_int` rather than in each member is what makes
canonical form a property of the package rather than of thirty separate lowerings.

### 4.4 Members (11) and the enum

```basic
EXPORT ENUM Endian { Little, Big }

big::fromInteger(value AS Integer) AS big::Int                       ' total
big::toInteger(value AS big::Int) AS Integer                         ' ErrOverflow
big::fromBytes(bytes AS List OF Byte, negative AS Boolean,
               endian AS big::Endian = Little) AS big::Int           ' total
big::toBytes(value AS big::Int,
             endian AS big::Endian = Little) AS List OF Byte         ' total
big::compare(a AS big::Int, b AS big::Int) AS Integer                ' total, -1/0/1
big::equals(a AS big::Int, b AS big::Int) AS Boolean                 ' total
big::isZero(a AS big::Int) AS Boolean                                ' total
big::sign(a AS big::Int) AS Integer                                  ' total, -1/0/1
big::abs(a AS big::Int) AS big::Int                                  ' total
big::negate(a AS big::Int) AS big::Int                               ' total
```

`Endian` is load-bearing, not decoration: crypto wire formats are big-endian while the
storage is little-endian, so without it every caller crossing that boundary hand-reverses
a byte list. Registered with `add_enum` as `money::Rounding` is
(`src/codegen/builtins/money/mod.rs:76-93`); variant order fixes the discriminants.

`fromInteger` is total by construction — every `Integer` fits. `toInteger` is the one
fallible member. Note `toInteger(fromInteger(x)) = x` must hold for `Integer`'s minimum,
whose magnitude is one past `Integer`'s maximum — the edge case tests must cover.

## Compatibility / Format Impact

- **Added:** one built-in package (`big`), one exported record (`big::Int`), one
  exported enum (`big::Endian`), eleven members. `IMPORT big` needs no manifest
  dependency, as for every built-in package.
- **Unchanged:** every existing package's public surface; the numeric tower;
  `Integer`/`Money`/`Fixed`/`Float` semantics; the `.mfp` wire format; record layout
  rules; the type grammar; every file under `packages/`.
- **Reserved for later letters:** `big::DivResult` (plan-127-C). No other identifier
  under `big::` is claimed here.

## Phases

> **NOTE — keep the checkboxes current as you go.**
> Tick `- [x]` in the same commit as the work it describes — never batched at the end.
> Use `- [~]` for partial and say what remains. Mark a task moot with
> `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line the moment the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — pin the runtime layout (uncertainty first)

Before any member exists, prove the four offsets every lowering will use. Throwaway
probe; nothing here ships.

- [ ] Read `src/codegen/collection/layout/builder_collection_layout.rs` and confirm
      against it: the kind-2 header size, the offset of `count`, the offset of the
      packed data region, and `record_field_is_inlined`'s treatment of a
      `List OF Byte` field.
- [ ] Write a temporary `abi_function` member that takes a `List OF Byte` and returns
      its element count read through the offsets above; verify it against `len()` for
      lists of length 0, 1, 255, 4096.
- [ ] Record the four confirmed offsets in §4.2, replacing the spec-derived reading.
      Note any divergence in Corrections.
- [ ] Delete the temporary member before commit.

Acceptance: the four offsets are recorded in this file, each citing the layout-builder
symbol that confirmed it, and the count probe agreed with `len()` on all four lengths.
Commit: —

### Phase 2 — package skeleton, the type, and the enum

The package exists and its type is declarable; no arithmetic yet.

- [ ] Create `src/codegen/builtins/big/mod.rs` with `MODULE_INTRO`/`MODULE_DESC`, the
      `INT_TYPE`/`INT_TYPE_ID` constant pair (the `net/mod.rs:112-121` split), the
      `add_record` for `Int` carrying the field-order-is-contract comment, and the
      `add_enum` for `Endian`.
- [ ] Register it: add `crate::codegen::builtins::big::register(&mut r);` to the block
      at `src/codegen/registry/mod.rs:2050-2080`, and declare the module in
      `src/codegen/builtins/mod.rs`.
- [ ] Tests in `big/mod.rs`, following the `money/mod.rs:105-182` shape: the package
      resolves; `Int` and `Endian` are registered types;
      `qualified_builtin_type("big.Int")` is `Some("big.Int")`; the rendered companion
      source contains `EXPORT TYPE Int` and `EXPORT ENUM Endian`.
- [ ] Runtime test: `LET x AS big::Int` compiles, and its default is `{[], FALSE}` —
      the canonical-zero-by-default property.

Acceptance: `mfb man big` renders the package page; a program with `IMPORT big` and a
defaulted `LET x AS big::Int` compiles and runs; a program that does **not** import
`big` produces byte-identical `.ncode` to before this phase.
Commit: —

### Phase 3 — the native access foundation

The three shared emitters, with no public member yet depending on them.

- [ ] `src/codegen/builtins/big/gen_big.rs` — `emit_load_int`, `emit_alloc_magnitude`,
      `emit_build_int` per §4.3, using the Phase 1 offsets.
- [ ] Unit tests: `emit_build_int` normalizes trailing zero bytes; maps `{[], TRUE}` to
      `{[], FALSE}`; leaves a canonical input unchanged.

Acceptance: a temporary public shim calling load→build round-trips a `big::Int` through
the foundation, normalizing three non-canonical inputs (trailing zeros, negative zero,
all-zero magnitude) to their canonical forms; the shim is removed before commit.
Commit: —

### Phase 4 — the conversion seams

- [ ] `func_from_integer.rs`, `func_to_integer.rs` — `abi_function` lowerings.
      `toInteger` declares `ErrOverflow` and nothing else.
- [ ] `func_from_bytes.rs`, `func_to_bytes.rs` — both total, both taking
      `endian AS big::Endian = Little` as a defaulted parameter.
- [ ] Tests: `toInteger(fromInteger(x)) = x` for `0`, `1`, `-1`, `Integer` max,
      `Integer` min; `toInteger` raises `ErrOverflow` one past `Integer` max;
      `toBytes(fromBytes(b, n, e), e) = b` for canonical `b` in both byte orders; a
      non-canonical input (trailing zero bytes) normalizes rather than failing.

Acceptance: the round-trip and overflow tests pass, and
`native_member_declares_error` reports `true` for `toInteger`, `false` for the other
three.
Commit: —

### Phase 5 — comparison and sign

- [ ] `func_compare.rs`, `func_equals.rs`, `func_is_zero.rs`, `func_sign.rs`,
      `func_abs.rs`, `func_negate.rs` — all total. `compare` orders by sign, then by
      magnitude length, then by descending byte index.
- [ ] Tests: `compare` is a total order over a spread covering both signs, zero, and
      differing magnitudes; `equals` agrees with `compare(...) = 0`; `isZero` agrees
      with `equals(x, fromInteger(0))`; `negate(negate(x)) = x`; `negate` of zero is
      canonical zero, not `{[], TRUE}`; `abs` of a negative is its magnitude; `sign`
      returns -1/0/1.
- [ ] Test: `equals` returns `TRUE` for a canonical value against a hand-constructed
      **non-canonical** record denoting the same number — the total-decoder contract.

Acceptance: all six report `native_member_declares_error` → `false`, and the comparison
suite passes including the non-canonical and negative-zero cases.
Commit: —

## Validation Plan

- **Tests:** Rust unit tests in each `func_*.rs` and in `big/mod.rs` for registry facts
  (membership, return types, declared errors, argument types), following
  `money/mod.rs:105-182`. Runtime behavior in a new `tests/rt_big_int.rs` covering every
  acceptance criterion above, including negative/error cases (`toInteger` overflow,
  non-canonical input, negative zero).
- **Coverage check:** `tests/` gives `src/**` zero coverage, so registry facts must be
  asserted from unit tests inside `src/codegen/builtins/big/`, not only from
  `tests/rt_big_int.rs`. Confirm the new module is in the coverage denominator before
  calling the gate green.
- **Runtime proof:** an `.mfb` program that builds a `big::Int` from a 32-byte value,
  round-trips it through `toBytes` in both byte orders, and prints `big::compare`
  results — run on a release binary, output checked against hand-computed expectations.
  Lowering is not runtime proof; this must actually execute.
- **Doc sync:** `mfb man big`, `mfb man big Int`, `mfb man big types` must render. Prose
  lives on the descriptors, so this is verification, not separate authoring. Run
  `scripts/man-census.sh --fill big` and `scripts/man-census.sh --memory-scope` (0
  unclassified hits — the memory-vocabulary ban applies; "copy" and "value" are the
  permitted words for what `big::Int` does). The `mfb spec` chapter is plan-127-D.
- **Acceptance:** `cargo test --no-fail-fast`, then `scripts/test-accept.sh`. Adding a
  package changes the registry, so `.ir` golden churn is expected — regenerate with
  `scripts/sync-goldens.sh` and prove the delta is only `big`'s.
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

1. **Scratch-buffer strategy for intermediate magnitudes.** **Recommend: allocate the
   result collection up front at the maximum possible size, write into its data region,
   then normalize in `emit_build_int`** — one allocation per operation, no separate
   scratch. Alternative: `builder.stack_size` scratch plus a copy, which avoids
   over-allocating for results that normalize far down but costs a second pass. Phase 3
   picks one and records why.
2. **`Endian` as a defaulted parameter vs. separate `*BE` members.** **Recommend: the
   defaulted enum parameter** — four names instead of eight, self-documenting at the
   call site. Alternative: separate members, plainer to read but proliferating.

## Corrections

<!-- Filled in DURING execution. Every place this plan turned out to be wrong: the
     claim, what was actually true, and the evidence. A corrected number also needs a
     check of whether a later letter's scope was derived from the wrong one. -->

## Summary

Engineering risk here is low by construction — no carry, no division, no multi-limb
loop. The letter's real job is to fix the three things that are expensive to change
later (canonical form, field order, the runtime offsets every lowering addresses) and
to prove the last of those against the layout builder before 30 members are written
against a guess.

Left untouched: all arithmetic (plan-127-B), division and everything built on it
(plan-127-C), the `mfb spec` chapter, the acceptance fixture, and the `crypto`
integration (plan-127-D).
