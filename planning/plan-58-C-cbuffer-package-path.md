# plan-58-C: carry `CBuffer` across the `.mfp` package boundary

Last updated: 2026-07-19
Effort: small (<1h)
Depends on: plan-58-A (`IrBuffer`, the position rules), plan-58-B (`IrLinkExpr`
arithmetic, the `LENGTH` expression)

`bindings/libsnd` is a **binding package**: its `LINK` block is compiled into an
`.mfp` and its wrappers are called by importers who never see the block. So every
piece of state plan-58-A and -C added to `IrLinkFunction` — the `buffers` table
and the `LENGTH` expression, plus the three new `IrLinkExpr` variants — must
survive encode/decode, and must be validated on the way in exactly as source is.

The single behavioral outcome: a program that `IMPORT`s a binding package
declaring an `OUT CBuffer` wrapper calls it and gets the same bytes as if the
`LINK` block were in its own source; and a crafted `.mfp` carrying a malformed
buffer declaration is rejected with the same diagnostic the source path gives.

References (read first):

- `src/ir/binary.rs:255-296` (`encode_project`; the LINK trailer is written only
  when non-empty at `:273-295`), `:307-321` (`encode_link_state_trailer`),
  `:348+` (`encode_link_function`; slots as `(name, ctype, direction.code())` at
  `:361-365`, `abi_return_*` at `:366-367`), `:460-469` (the decode branch),
  `:503-536` (`decode_cstructs`, with `MAX_CSTRUCTS = 256` /
  `MAX_CSTRUCT_FIELDS = 64` at `:497-498`), `:538+` (`decode_link_function`),
  `:244-249` (the **exact-match** version gate).
- `src/binary_repr/mod.rs:48` (`SECTION_BINARY_REPR = 16`) — LINK data is an
  append-only trailer inside the IR payload, **not its own section**.
- `src/ir/coverage_tests.rs:490-509` (`full_project()` builds the LINK tables),
  `:539-556` (`binary_round_trip_over_full_surface`), `:566-573` (the bare-trailer
  case), `:578+` (`rejects_a_previous_format_version`), `:612-643`
  (`rejects_an_invalid_slot_direction` — the byte-diff trick for locating a
  field), `:645-662` (`binary_round_trip_link_expr_variants`).
- `src/ir/verify/mod.rs:3042-3079` (the package-path ctype gate) and plan-58-A's
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
- `BINARY_REPR_VERSION` is bumped, and a package written by the previous compiler
  is rejected by the exact-match gate (`binary.rs:244-249`) with a clear message.
- Decode is **bounded**: a `buffers` table is capped like `decode_cstructs` caps
  its tables, and expression nesting is depth-capped, so a crafted package cannot
  drive unbounded allocation or recursion.
- A crafted `.mfp` violating any plan-58-A rule (a `CBuffer` slot with no
  `BUFFER` clause, an `IN`-direction `CBuffer`, a `List OF Byte` return with no
  buffer slot, a `SIZE` naming an unknown slot) is rejected by `ir::verify` with
  the same rule the source path emits.
- An importer calling a `CBuffer` wrapper across the package boundary gets the
  right bytes at runtime.

### Non-goals (explicit constraints)

- No change to the section layout. LINK data stays an append-only trailer inside
  `SECTION_BINARY_REPR` (`binary_repr/mod.rs:48`); do not mint a new section id.
- No change to how any existing LINK field encodes. A package containing no
  `CBuffer` must encode **byte-identically** to before, apart from the version
  number.
- No new validation *logic* — plan-58-A's `check_buffer_slots` is already shared
  and already called from `ir::verify`. This sub-plan only ensures the data
  reaches it intact and that the decoder itself is bounded.
- No relaxation of the exact-match version gate.

## 2. Current State

The LINK trailer is written by `encode_project` (`binary.rs:273-295`) only when
the link tables are non-empty, so a package with no `LINK` block carries a bare
trailer (`coverage_tests.rs:566-573` pins this). `encode_link_function`
(`:348+`) writes slots as `(name, ctype, direction.code())`, then
`abi_return_name`/`abi_return_ctype`, then `CONST`s, `SUCCESS_ON`, `RETURN`,
`BIND IN`, `BIND STATE`, `FREE`.

**CSTRUCT layouts are deliberately not transported** (`:282-284`): only field
names and ctypes ride the wire, and `compute_c_layout` recomputes offsets on
decode. The security argument is stated at `src/ir/link.rs:279-281` — a crafted
package can choose ctypes, each of known size, but has no offset to forge. The
same principle applies here: **a `CBuffer`'s size is an expression, never a
number**, so a crafted package cannot forge a size/capacity mismatch either. It
can only supply an expression, which the thunk evaluates and the runtime gate
(plan-58-B §4.2) bounds.

**Nothing validates a ctype string on decode.** `r.string()?` accepts any UTF-8
(`decode_link_function:570` for `abi_return_ctype`, `:555-568` for slots). The
rejection happens downstream in `ir::verify`
(`nir/lower.rs:89`, `:107`), which is exactly why `check_link_functions`
duplicates the syntaxcheck logic. `AbiDirection::from_code` (`link.rs:488-495`)
is the one decoder-level validator, rejecting codes `> 2`; it is pinned by
`rejects_an_invalid_slot_direction` (`coverage_tests.rs:612-643`).

**The gap this sub-plan should also close:** there is currently **no coverage test
for a crafted unknown ctype string** on the decode path. The direction byte has
one; the ctype string does not. Since `CBuffer` makes ctype position rules
load-bearing, add it.

## 3. Design Overview

Three mechanical pieces plus one judgement call.

1. **Encode/decode `buffers`** as a length-prefixed table after `BIND IN`,
   each entry `(slot: string, size: IrLinkExpr)`. Bound the count.
2. **Encode/decode the `LENGTH` expression** as an `Option<IrLinkExpr>` beside
   the existing `result` field (`IrLinkFunction.result`, `link.rs:411`).
3. **Extend the `IrLinkExpr` opcode space** with `Mul`/`Add`/`Sub`, keeping the
   existing opcodes at their current values so no existing expression re-encodes.
   Reject an unknown opcode on decode (do **not** default), and cap nesting depth.

**The judgement call — where to append.** The trailer is append-only, which
tempts a scheme where an old compiler ignores new trailing fields. Resist it: the
version gate is an **exact match** (`binary.rs:244-249`), not a floor, so
forward-compatibility is not a design goal and a partially-understood LINK table
is far more dangerous than a rejected package. Bump the version, append wherever
is clearest, and let the gate do its job. plan-50-C made the same call.

**Where the correctness risk concentrates:** in the decoder's bounds, not in the
round-trip. A round-trip bug fails the coverage test immediately and loudly. An
unbounded `buffers` count or an unbounded expression depth is a resource-exhaustion
primitive reachable by anyone who can hand the compiler a `.mfp` — the same threat
model that motivated `MAX_CSTRUCTS`/`MAX_CSTRUCT_FIELDS` (`binary.rs:497-498`) and
the audit-1 package-decode hardening (memory `audit-1-package-decode-impl`).

## 4. Detailed Design

### 4.1 Wire additions to `encode_link_function`

Appended after the existing `BIND IN` table:

```
u32   buffer_count            (<= MAX_LINK_BUFFERS)
  repeated:
    string  slot
    expr    size
u8    has_length              (0 | 1)
  if 1: expr length
```

`MAX_LINK_BUFFERS = 16`. Justification: a wrapper is bounded by the target's
external integer argument register count (6 on x86-64 SysV, 8 elsewhere;
`link_thunk.rs:659-675`), so more than a handful of buffers is already
unreachable. 16 is generous and far below anything that threatens allocation.

### 4.2 `IrLinkExpr` opcodes

Add `Mul`/`Add`/`Sub` at the next free opcode values; do **not** renumber
existing ones. Decode rejects an unknown opcode with an error naming the value,
mirroring `AbiDirection::from_code`'s message style ("invalid ABI slot direction
N").

Add `MAX_LINK_EXPR_DEPTH = 32` to the recursive decoder. There is no depth cap
today because the existing grammar (`Var`, comparisons, `And`/`Or`/`Not`) already
allowed arbitrary nesting — so **this is a pre-existing hole**, not one this
sub-plan opens. Note that in the commit message; the cap is worth landing either
way.

### 4.3 Version

Bump `BINARY_REPR_VERSION` by one. Update
`rejects_a_previous_format_version` (`coverage_tests.rs:578+`) if it pins a
literal, and update the `.mfp` format spec under `src/docs/spec/package/` with the
new fields and version — the spec documents the wire format and is the thing an
independent decoder would be written from.

Every in-tree `.mfp` must be rebuilt: `bindings/libsnd`, `bindings/sqlite3`, and
any committed package artifact. Grep for `*.mfp` before landing.

## Compatibility / Format Impact

- **Changes:** `BINARY_REPR_VERSION` bumps; every previously-compiled `.mfp` is
  rejected and must be rebuilt. This is the established behavior of the
  exact-match gate, not a new burden.
- **Changes:** `encode_link_function` gains three trailing fields.
- **Unchanged:** the section layout and `SECTION_BINARY_REPR = 16`; the encoding
  of every existing LINK field and `IrLinkExpr` opcode; the CSTRUCT
  offsets-are-never-transported rule; the exact-match gate itself.

## Phases

### Phase 1 — round-trip, bounds, and the cross-package runtime proof

- [ ] `src/ir/binary.rs`: encode/decode `buffers` and the optional `LENGTH`
      expression (§4.1); add `MAX_LINK_BUFFERS`.
- [ ] `src/ir/binary.rs`: add the `Mul`/`Add`/`Sub` opcodes; reject unknown
      opcodes explicitly; add `MAX_LINK_EXPR_DEPTH` to the recursive decoder.
- [ ] Bump `BINARY_REPR_VERSION`; update the `.mfp` format spec under
      `src/docs/spec/package/`; rebuild every in-tree `.mfp`.
- [ ] Tests: extend `full_project()` (`coverage_tests.rs:490-509`) with a
      `CBuffer` slot, a `BUFFER … SIZE` entry using arithmetic, and a `LENGTH`
      expression; extend `binary_round_trip_over_full_surface` (`:539-556`) to
      assert all three survive.
- [ ] Tests: extend `binary_round_trip_link_expr_variants` (`:645-662`) with the
      three new variants.
- [ ] Tests: `rejects_an_unknown_link_expr_opcode` and
      `rejects_a_buffer_count_over_the_cap`, mirroring
      `rejects_an_invalid_slot_direction`'s byte-diff approach (`:612-643`).
- [ ] Tests: **`rejects_a_crafted_unknown_ctype`** — the gap noted in §2. A
      package whose slot ctype string is `"CBogus"` must be rejected by
      `ir::verify` with `NATIVE_ABI_UNKNOWN_CTYPE`. This is not strictly a
      plan-58 obligation, but `CBuffer` is what makes ctype *position* rules
      load-bearing on the package path, and the test costs little.
- [ ] Tests: package-path rejection of each plan-58-A buffer rule via a crafted
      link table in `src/ir/verify/tests.rs` (at minimum: `IN`-direction
      `CBuffer`, missing `BUFFER` clause, `List OF Byte` return with no buffer
      slot).
- [ ] Tests: `tests/rt-behavior/native/native-buffer-import-rt/` — a **two-package**
      fixture where the `LINK` block with the `OUT CBuffer` lives in an imported
      binding package and `main` calls it across the boundary. Mirror
      `tests/rt-behavior/native/native-link-import-sqlite-rt/` and
      `native-resource-state-import-rt/`.

Acceptance: `native-buffer-import-rt` returns the same bytes as the single-package
`native-buffer-read-rt` from plan-58-B, proving the declaration survived the
`.mfp` round-trip; each crafted-package test is rejected with the named rule;
`scripts/test-accept.sh target/debug/mfb target/accept-actual` is green after
every in-tree `.mfp` is rebuilt.
Commit: —

## Validation Plan

- Tests: the extended `src/ir/coverage_tests.rs` round-trip and rejection cases;
  the crafted-link-table cases in `src/ir/verify/tests.rs`;
  `tests/rt-behavior/native/native-buffer-import-rt/`.
- Runtime proof: **required (Hard Completion Gate).** A round-trip unit test
  proves the bytes survive; only the cross-package fixture proves the *decoded*
  declaration drives a correct native call. These are different claims — the
  in-memory IR and the decoded IR are separate objects, and CSTRUCT offsets are
  recomputed on decode specifically because they differ.
- Doc sync: the `.mfp` format spec under `src/docs/spec/package/` (new fields, new
  opcodes, new version). No rule or error-code change — plan-58-A already
  registered `NATIVE_BUFFER_INVALID`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
  Expect churn wherever a `.mfp` version number is visible in a golden; confirm
  that is the *only* churn before accepting any of it.

## Open Decisions

- **`MAX_LINK_EXPR_DEPTH` is a pre-existing gap.** The recursive `IrLinkExpr`
  decoder has no depth cap today. Recommend landing the cap here since this
  sub-plan is already in the decoder, and noting in the commit message that it
  predates plan-58 — so it is not mistaken for a hole this feature opened.
  Alternative: a separate bug fix, which is cleaner provenance but a second pass
  over the same function.
- **Should the `.mfp` carry a *declared* buffer capacity as a number, for a
  decode-time bound?** Recommend no. It would be a forgeable number, which is
  precisely what the CSTRUCT design avoided by transporting ctypes rather than
  offsets (`src/ir/link.rs:279-281`). The expression is safer: it is evaluated at
  runtime and bounded by plan-58-B's `CBUFFER_MAX_BYTES` gate.

## Summary

Mechanically small — three fields and three opcodes on an append-only trailer,
behind a version bump. The risk is not in the round-trip (a coverage test catches
that immediately) but in the decoder's bounds, since anyone who can hand the
compiler a `.mfp` reaches this code. Hence the count cap, the depth cap, and
explicit rejection of unknown opcodes rather than defaulting.

Untouched: the section layout, every existing field's encoding, and the
offsets-are-never-transported rule.
