# plan-58-C: carry `CBuffer` across the `.mfp` package boundary

Last updated: 2026-07-20
Effort: small (<1h) — **but see §2.1: the version bump churns 126 committed
`.mfp` files, which is the real cost of this sub-plan.**
Depends on: plan-58-A (`IrBuffer`, the position rules), plan-58-B
(`IrLinkExpr::{Mul,Add,Sub}`, the `LENGTH` expression)
Produces: `.mfp` encode/decode for `buffers` + `LENGTH` + the three new
`IrLinkExpr` opcodes, `BINARY_REPR_VERSION` 6, decode-path bounds. Consumed by D
(`libsnd` is a binding package and reaches its wrappers only across this
boundary).

`bindings/libsnd` is a **binding package**: its `LINK` block is compiled into an
`.mfp` and its wrappers are called by importers who never see the block. So every
piece of state plan-58-A and plan-58-B added to `IrLinkFunction` — the `buffers`
table and the `LENGTH` expression, plus the three new `IrLinkExpr` variants —
must survive encode/decode, and must be validated on the way in exactly as source
is.

The single behavioral outcome: a program that `IMPORT`s a binding package
declaring an `OUT CBuffer` wrapper calls it and gets the same bytes as if the
`LINK` block were in its own source; and a crafted `.mfp` carrying a malformed
buffer declaration is rejected with the same diagnostic the source path gives.

References (read first):

- `src/ir/binary.rs:255-296` (`encode_project`; the LINK trailer is written only
  when non-empty at `:273-295`), `:307-321` (`encode_link_state_trailer`),
  `:348` (`encode_link_function`; slots as `(name, ctype, direction.code())` at
  `:361-365`, `abi_return_*` at `:366-367`), `:460-469` (the decode branch),
  `:503-536` (`decode_cstructs`, with `MAX_CSTRUCTS = 256` /
  `MAX_CSTRUCT_FIELDS = 64` at `:497-498`), `:538` (`decode_link_function`),
  `:244-249` (the **exact-match** version gate), `:32`
  (`BINARY_REPR_VERSION = 5`).
- `src/binary_repr/mod.rs:48` (`SECTION_BINARY_REPR = 16`) — LINK data is an
  append-only trailer inside the IR payload, **not its own section**.
- `src/ir/coverage_tests.rs:372` (`full_project()`; its LINK tables at
  `:490-509`), `:527-560` (`binary_round_trip_over_full_surface`), `:563`
  (the bare-trailer case), `:580` (`rejects_a_previous_format_version`),
  `:612-636` (`rejects_an_invalid_slot_direction` — the byte-diff trick for
  locating a field), `:638-662` (`binary_round_trip_link_expr_variants`).
- `src/ir/verify/mod.rs:3042-3086` (the package-path ctype gate) and plan-58-A's
  `check_buffer_slots` call site there.
- `src/target/shared/nir/lower.rs:89` (`verify_package`), `:107`
  (`verify_semantics`) — the two points where a decoded package is checked.
- `planning/old-plans/plan-50-C-cstruct-package-format.md` — the precedent: the
  sub-plan that carried CSTRUCTs across the same boundary, including its
  `BINARY_REPR_VERSION` bump.
- `src/docs/spec/package/` — the `.mfp` format spec that must be updated with it.

## 1. Goal

- `IrBuffer` (`slot`, `size: IrLinkExpr`) and the optional `LENGTH` expression
  round-trip through `.mfp` encode/decode with full fidelity.
- The three new `IrLinkExpr` variants (`Mul`/`Add`/`Sub`) round-trip, and an
  unknown expression opcode is rejected on decode rather than defaulted.
- `BINARY_REPR_VERSION` goes **5 → 6**, and a package written by the previous
  compiler is rejected with the existing version diagnostic.
- A crafted `.mfp` declaring a malformed buffer (bad direction, missing clause,
  unknown slot name, `LENGTH` without `CBuffer`) is rejected by `ir::verify` with
  the *same* rule code the source path produces.
- The decode path is bounded: a buffer count and an expression depth cap, so a
  crafted package cannot allocate or recurse without limit.

### Non-goals (explicit constraints)

- No change to the LINK trailer's **framing** — buffers append to the existing
  per-function record; the trailer stays inside `SECTION_BINARY_REPR`, not a new
  section.
- No change to how CSTRUCTs, slots, aliases, or `SUCCESS_ON` encode.
- No relaxation of the exact-match version gate (`binary.rs:244-249`). Old
  packages are rejected, not migrated.
- No new validation *rules* — this sub-plan makes plan-58-A's rules fire on the
  package path; it does not invent any.

## 2. Current State

### 2.1 Measured populations — the real cost of this sub-plan

| What | Count | Command |
|---|---|---|
| `BINARY_REPR_VERSION` today | **5** | `rg -n 'BINARY_REPR_VERSION' src/ir/binary.rs` → `:32` |
| **Committed `.mfp` files that churn on a version bump** | **126** | `find . -name '*.mfp' -not -path './target/*' \| wc -l` |
| — of those, bindings | 2 | `bindings/libsnd/libsnd.mfp`, `bindings/sqlite3/sqlite3.mfp` |
| — of those, tool packages | 17 | `tools/thread-package-sources/*/` |
| — of those, test goldens | 107 | `tests/**/golden/*.mfp` and `tests/rt-behavior/**/packages/` |
| `IrLinkExpr` variants today | **6** (`Var`, `Int`, `Compare`, `And`, `Or`, `Not`) | `sed -n '515,534p' src/ir/link.rs` |
| `MAX_CSTRUCTS` / `MAX_CSTRUCT_FIELDS` (the bounding precedent) | 256 / 64 | `rg -n 'MAX_CSTRUCTS\|MAX_CSTRUCT_FIELDS' src/ir/binary.rs` → `:497-498` |
| External int arg registers (bounds `MAX_LINK_BUFFERS`) | 6 x86-64 SysV, 8 elsewhere | `x86_64/regmodel.rs:159` → 6; `shared/regmodel.rs:147` → `REGISTER_ARGUMENT_COUNT` = 8 (`abi.rs:30`) |

**The 126 is the headline.** The 2026-07-19 draft said "Every in-tree `.mfp` must
be rebuilt: `bindings/libsnd`, `bindings/sqlite3`, and any committed package
artifact" — naming two and gesturing at the rest. The real number is 126, and 107
of them are **byte-compared goldens** under `tests/`, so every one churns in the
acceptance run. That is what makes this "small" sub-plan expensive, and it is why
the version bump gets its own phase.

### 2.2 How the LINK trailer encodes

The LINK tables are an **append-only trailer** inside the IR payload
(`SECTION_BINARY_REPR = 16`, `src/binary_repr/mod.rs:48`), written only when
non-empty (`binary.rs:273-295`). `encode_link_function` (`:348`) writes slots as
`(name, ctype, direction.code())` (`:361-365`) then `abi_return_name` /
`abi_return_ctype` (`:366-367`). Appending a `buffers` vector and an optional
`LENGTH` expression after the existing fields is the same move plan-50-C made for
CSTRUCTs.

The version gate is **exact-match** (`binary.rs:244-249`): `if version !=
BINARY_REPR_VERSION`. There is no forward or backward compatibility window, by
design — so a bump means every artifact is regenerated, never migrated.

### 2.3 Where the decode path is currently unguarded

`decode_link_function` (`binary.rs:538`) reads slot ctypes with `r.string()?`,
which accepts **any UTF-8** (`:555-568` for slots, `:570` for
`abi_return_ctype`). The only decoder-level validator is
`AbiDirection::from_code` (`src/ir/link.rs:488-495`), which rejects codes `> 2`.

So a crafted `.mfp` can carry an arbitrary ctype string today, and it is caught
only later by `ir::verify`'s gate. That is the correct layering — but it means
plan-58-A's `check_buffer_slots` **must** be wired into `ir::verify`, because
nothing at the decode layer will catch a malformed buffer.

There is currently **no coverage test for a crafted unknown ctype string** on the
decode path. The direction byte has one (`rejects_an_invalid_slot_direction`,
`coverage_tests.rs:612`); the ctype string does not. This sub-plan adds it.

### 2.4 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The version gate is exact-match, no migration window | **CONFIRMED** | `binary.rs:244-249` |
| `rejects_a_previous_format_version` pins a **literal** version | **CONFIRMED** | `coverage_tests.rs:580` does `bytes[4..6].copy_from_slice(&4u16.to_le_bytes())` and asserts `"version 4 unsupported"` — so the bump requires editing this test, it is not automatic |
| No decode-path test exists for a crafted ctype string | **CONFIRMED** | `rg -n 'CBogus\|unknown_ctype' src/ir/coverage_tests.rs` → 0 matches |
| No `IrLinkExpr` depth cap exists | **CONFIRMED** | no depth guard in the recursive decoder, `binary.rs:538+`. Pre-existing hole, widened by adding three recursive variants |
| Offsets are derived, never transported (the security argument) | **CONFIRMED** | `src/ir/binary.rs:282-284` and `src/ir/link.rs:139-142`. **Not** `link.rs:279-281` — the draft cited that, which is the *shared-validation* comment plan-58-A cites for a different purpose |
| The `.mfp` blast radius is 2 files | **FALSE** | 126 — see §2.1 |
| `MAX_LINK_BUFFERS` / `MAX_LINK_EXPR_DEPTH` exist | **FALSE** | `rg -n 'MAX_LINK_BUFFERS\|MAX_LINK_EXPR_DEPTH' src/` → 0. Both are proposals in this sub-plan |

## 3. Design Overview

Three pieces, all mechanically small; the cost is in the churn, not the code.

1. **Encode/decode the new state** — `buffers: Vec<IrBuffer>` and the optional
   `LENGTH` expression, appended to `encode_link_function` / `decode_link_function`;
   three new opcodes in the `IrLinkExpr` encoding.
2. **Bound the decode path** — `MAX_LINK_BUFFERS` and `MAX_LINK_EXPR_DEPTH`,
   mirroring `MAX_CSTRUCTS` / `MAX_CSTRUCT_FIELDS` (`binary.rs:497-498`).
3. **Bump the version and regenerate** — 5 → 6, then 126 artifacts.

**Where design uncertainty concentrates:** nowhere. plan-50-C did exactly this
for CSTRUCTs and its shape is known-good. Every premise in §2.4 is verified.

**Where correctness risk concentrates:** the **`ir::verify` wiring**, not the
codec. If `check_buffer_slots` is not called on the package path, a crafted
`.mfp` reaches plan-58-B's marshaler with a malformed buffer — the position rules
plan-58-A wrote would be enforced on source only, which is precisely the drift
`src/ir/link.rs:279-281` exists to prevent. The negative package-path fixture is
the guard.

**Rejected alternative:** *a new section for LINK buffer data.* Rejected: the
trailer is append-only by design and adding a section changes framing for every
reader. plan-50-C established the append precedent.

**Rejected alternative:** *tolerate old `.mfp` versions and default `buffers` to
empty.* Rejected: the gate is deliberately exact-match, and a silently-defaulted
buffer table is how a wrapper would decode as "no buffers" and then lower a
`CBuffer` slot with no capacity.

## 4. Detailed Design

### 4.1 The encoding

Appended to each function record, after `abi_return_ctype` (`binary.rs:366-367`):

```
buffers      : vec<(slot: str, size: IrLinkExpr)>
result_length: opt<IrLinkExpr>
```

`IrLinkExpr` gains three opcodes for `Mul`/`Add`/`Sub`, each `(lhs, rhs)`. An
unknown opcode must **error**, not default — mirror the existing unknown-variant
handling rather than falling through.

### 4.2 The bounds

```rust
const MAX_LINK_BUFFERS: usize = 16;
const MAX_LINK_EXPR_DEPTH: usize = 32;
```

`MAX_LINK_BUFFERS = 16` is generous: a wrapper cannot have more buffer slots than
the target's external integer argument registers (6 on x86-64 SysV, 8 elsewhere,
`link_thunk.rs:659-675`), so 16 is already unreachable — it exists to bound
allocation on a crafted file, not to constrain real code.

`MAX_LINK_EXPR_DEPTH = 32` closes a **pre-existing** hole: the recursive
`IrLinkExpr` decoder has no depth cap today, and `And`/`Or`/`Not` already allowed
unbounded nesting. Adding three more recursive variants widens it. Fixing it here
is in scope because this sub-plan is the one touching that decoder.

### 4.3 The `ir::verify` wiring

`check_buffer_slots` (plan-58-A) is called from `ir::verify`'s
`check_link_functions` (`verify/mod.rs:3042-3086`) with function-level spans. The
faults map to the same rule codes the source path emits.

Two incidental notes on that function, found while reading it:

- Lines `3053-3059` and `3060-3066` are a **verbatim duplicated CSTRUCT-skip
  block** (identical `if project.link_cstructs.iter().any(...) { continue; }`).
  Harmless but dead. Remove it while wiring in `check_buffer_slots`.
- Its ctype gate extends to ~`:3086`, not `:3079` as the 2026-07-19 draft said.

## Compatibility / Format Impact

- **Changes:** `BINARY_REPR_VERSION` 5 → 6. Every `.mfp` written by an older
  compiler is rejected with the existing version diagnostic. **126 committed
  `.mfp` files must be regenerated** (§2.1), of which 107 are byte-compared test
  goldens.
- **Unchanged:** the trailer's framing and section id; CSTRUCT, slot, alias and
  `SUCCESS_ON` encoding; the exact-match gate's behavior.

## Phases

Ordered so the expensive, mechanical churn is isolated from the logic.

### Phase 1 — codec, bounds, and the package-path gate

- [ ] `src/ir/binary.rs`: encode/decode `buffers` and `result_length` in
      `encode_link_function` (`:348`) / `decode_link_function` (`:538`).
- [ ] `src/ir/binary.rs`: three new `IrLinkExpr` opcodes; unknown opcode errors.
- [ ] `src/ir/binary.rs`: add `MAX_LINK_BUFFERS = 16` and
      `MAX_LINK_EXPR_DEPTH = 32` near `:497-498`, enforced on decode.
- [ ] `src/ir/verify/mod.rs:3042-3086`: call `check_buffer_slots`; delete the
      duplicated CSTRUCT-skip block at `:3053-3066`.
- [ ] Tests: extend `full_project()` (`coverage_tests.rs:372`, LINK tables at
      `:490-509`) with a buffer + `LENGTH` + each new operator, so
      `binary_round_trip_over_full_surface` (`:527`) covers them.
- [ ] Tests: **the missing negative** — a crafted `.mfp` with an unknown ctype
      string, using the byte-diff trick from `rejects_an_invalid_slot_direction`
      (`:612`). This gap is called out in §2.3 and exists today.
- [ ] Tests: crafted `.mfp` negatives for each malformed-buffer shape (bad
      direction, missing `BUFFER` clause, unknown slot name, `LENGTH` without a
      `CBuffer`), each asserting the **same rule code** the source path gives.
- [ ] Tests: over-limit `buffers` count and over-depth expression both rejected.

Acceptance: round-trip is byte-identical for a project carrying buffers, `LENGTH`
and all three operators; every crafted-`.mfp` negative produces the same rule
code as its source-path twin; over-limit and over-depth inputs are rejected
rather than allocated or recursed. `BINARY_REPR_VERSION` is still 5 at this
point — no golden churn yet.
Commit: —

### Phase 2 — the version bump and the 126-file regeneration (largest blast radius last)

Isolated deliberately: this phase changes no logic and produces ~126 changed
files, so it must be reviewable as pure churn.

- [ ] `src/ir/binary.rs:32`: `BINARY_REPR_VERSION` 5 → 6.
- [ ] `src/ir/coverage_tests.rs:580`: update `rejects_a_previous_format_version`
      — it pins the literal `4u16` and asserts `"version 4 unsupported"` (§2.4),
      so it does not follow the constant automatically.
- [ ] Regenerate all 126 `.mfp` artifacts: `bindings/` (2),
      `tools/thread-package-sources/` (17), and the test goldens (107) via the
      project's golden-sync path.
- [ ] `src/docs/spec/package/`: document the buffer/`LENGTH` trailer fields, the
      new opcodes, and the version.

Acceptance: the full acceptance suite is green with exactly the expected `.mfp`
churn and **no** non-`.mfp` golden changes; an `.mfp` built before the bump is
rejected with the version diagnostic naming 5.
Commit: —

## Validation Plan

- Tests: round-trip over the full surface, plus a negative per malformed-buffer
  shape and per bound. The crafted-ctype-string negative (§2.3) is a real
  pre-existing gap, not a formality.
- Coverage check: `src/ir/coverage_tests.rs` is a unit suite and is in the
  denominator. The `.mfp` goldens are byte-compared by `scripts/test-accept.sh`
  (`:468-470`), so the churn is visible — confirm it is *only* `.mfp` files.
- Runtime proof: a program importing a binding package whose wrapper declares an
  `OUT CBuffer` calls it and receives the same bytes as the equivalent in-source
  `LINK` block. Mirror `tests/rt-behavior/native/native-link-import-sqlite-rt/`
  and `native-resource-state-import-rt/` (both confirmed to exist).
- Doc sync: `src/docs/spec/package/`.
- Acceptance: the project's full suite. Expect ~126 `.mfp` files changed in
  Phase 2 and nothing else.

## Open Decisions

1. **`MAX_LINK_EXPR_DEPTH` here vs. as its own bug.** The depth hole is
   pre-existing and independent of `CBuffer`. Recommended: fix it here, since
   this sub-plan is already editing that decoder and adding recursive variants to
   it. Alternative: file it separately and keep this sub-plan minimal. (§4.2)
2. **Whether Phase 2's regeneration should be one commit or split by directory.**
   Recommended: one commit, since a half-regenerated tree fails the exact-match
   gate everywhere. (§Phase 2)

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The `.mfp` blast radius is 126 files, not 2.** The draft named
  `bindings/libsnd` and `bindings/sqlite3`. Measured: 126 committed `.mfp` files,
  107 of them byte-compared goldens. The version bump now has its own phase.
- 2026-07-20 — **The security-argument citation was wrong.** The draft cited
  `src/ir/link.rs:279-281` for "offsets are derived, never transported". That is
  the *shared-validation* comment. The offsets argument is at
  `src/ir/binary.rs:282-284` and `src/ir/link.rs:139-142`.
- 2026-07-20 — **`IrLinkExpr` has six variants, not five.** The draft's
  enumeration omitted `Int(i64)`, which already exists.
- 2026-07-20 — **`rejects_a_previous_format_version` pins a literal.** The draft
  said "update it *if* it pins a literal" — it does (`4u16`, and the string
  `"version 4 unsupported"`), so the edit is mandatory, not conditional.
- 2026-07-20 — Line drift corrected throughout: `full_project` is at
  `coverage_tests.rs:372` (draft said `:490-509`, which is its LINK-table slice);
  `binary_round_trip_over_full_surface` `:527-560`;
  `rejects_a_previous_format_version` `:580`;
  `binary_round_trip_link_expr_variants` `:638-662`; the `ir::verify` gate
  extends to ~`:3086`.

## Summary

The code here is genuinely small — three fields, three opcodes, two bounds, one
call site. The engineering risk is concentrated in one place that is not code at
all: **126 regenerated `.mfp` artifacts**, isolated into Phase 2 so the logic
change stays reviewable.

The one correctness risk that is code: forgetting the `ir::verify` call site,
which would enforce plan-58-A's rules on source but not on packages — the exact
drift the shared-checker design exists to prevent.

What is left untouched: the trailer framing, the section id, and every other
encoded structure.
