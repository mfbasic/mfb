# plan-58-A: the `CBuffer` slot ctype — vocabulary, position rules, and gates

Last updated: 2026-07-19
Overall Effort: large (3h–1d) — the whole plan-58 feature (A–D)
Effort: medium (1h–2h)
Depends on: nothing for this sub-plan (it lands before plan-58-B, which
implements its marshaling). **Feature-level prerequisites for plan-58 as a
whole:** `bugs/bug-364-libsnd-sf-info-missing-frames-field.md` (plan-58-D sizes
its PCM buffer from `frames`) and **plan-57** (plan-58-B allocates through
`emit_alloc_list` and relies on the `kind = 2` byte-list representation and its
index-order guarantee).

Introduces `CBuffer` into the ABI slot ctype namespace as an **OUT-only,
runtime-sized byte buffer** that surfaces as `List OF Byte`, together with the
`BUFFER <slot> SIZE <expr>` clause that gives it a capacity. This sub-plan ships
**parsing and rejection only** — every accepted declaration still fails to lower,
loudly, at `link_thunk.rs`'s `Err` arm. plan-58-B makes it marshal.

Landing the vocabulary first is deliberate and mirrors plan-50-A→B: the moment a
new ctype exists, an unimplemented or mis-positioned one must fail as a
*diagnostic*, never as a silent raw 64-bit load. It also closes a latent hole
found while planning (§2): a LINK wrapper may already declare
`AS List OF Byte` today and it compiles to **garbage** rather than being
rejected.

The single behavioral outcome: `CBuffer` is a known ctype that is accepted only
as an `OUT` slot carrying a `BUFFER … SIZE` clause on a wrapper returning
`List OF Byte`, and is rejected with a specific diagnostic in every other
position — identically on the source path and the `.mfp` package path.

References (read first):

- `planning/old-plans/plan-50-A-abi-ctype-allowlist.md` — **the template for this
  sub-plan.** Same three-gate shape (`ir::link` authority → syntaxcheck →
  `ir::verify`), same drift-guard test, same rule-table + spec obligation. Read
  its "Landed note" (`:313-319`): the drift guard discovered the
  argument/return position split. Expect the same here.
- `src/ir/link.rs:16-35` (`abi_slot_ctype_is_known`), `:41-43`
  (`abi_ctype_valid_as_argument`), `:52-54` (`abi_ctype_valid_as_return`),
  `:104-120` (`ctype_size_align`), `:466-500` (`AbiDirection`, `writes_back()`),
  `:504-509` (`IrAbiSlot`), `:377-434` (`IrLinkFunction`).
- `src/ir/link.rs:549-575` — `tests::ctype_list_is_exhaustive`, which pins the
  authority against `link_thunk.rs`'s `CTYPES` literal. Both move together.
- `src/syntaxcheck/mod.rs:752-787` — the source-path ctype gate, including the
  `writes_back()` → `valid_as_return` routing at `:771-775`.
- `src/ir/verify/mod.rs:3042-3079` — the package-path mirror.
- `src/ast/items.rs:1146-1205` (`parse_abi_spec`; `INOUT` before `OUT` at
  `:1164-1172` is load-bearing), `:1217-1219` (`parse_c_type_name` is a bare
  `consume_identifier` — `CBuffer` already *parses* today).
- `src/rules/table.rs:830` (`NATIVE_ABI_UNKNOWN_CTYPE` = `2-203-0123`) and the
  native-ABI block at `:992-1058`; `src/docs/spec/diagnostics/01_rule-codes.md`.
- `src/rules/mod.rs:every_rule_is_documented_in_the_spec` — a new rule without a
  spec row fails the suite.
- `src/docs/spec/language/17_native-libraries.md` — the ctype table (~`:187-205`
  in rendered form) and §Rules.
- `.ai/compiler.md`, `.ai/specifications.md`.

## 1. Goal

- `abi_slot_ctype_is_known("CBuffer")` is `true`.
- A `CBuffer` slot is accepted **only** when all of the following hold; each
  violation has its own diagnostic (§4.3):
  1. its direction is `OUT` (not `IN`, not `INOUT`);
  2. the function declares exactly one `BUFFER <slot> SIZE <expr>` clause naming
     it;
  3. it is not `CONST`-pinned;
  4. it is the slot named by `RETURN`, and the wrapper's return type is
     `List OF Byte`.
- Conversely, a wrapper returning `List OF Byte` **without** a `CBuffer` result
  slot is rejected — closing the pre-existing garbage-codegen hole in §2.
- `CBuffer` is rejected as a CSTRUCT field (`ctype_size_align` returns `None`,
  as `CVoid` does) and as the ABI return.
- Every rejection fires identically from `syntaxcheck` and from `ir::verify`, so a
  crafted `.mfp` gets exactly the source-path treatment
  (`src/ir/link.rs:279-281`).
- `link_thunk.rs` still returns `Err` for `CBuffer`; no binding can use it yet.

### Non-goals (explicit constraints)

- **No marshaling.** That is plan-58-B. This sub-plan must not emit a single
  instruction for `CBuffer`.
- **Do not touch `is_c_abi_type`** in any of its three copies
  (`src/syntaxcheck/helpers.rs:204-220`, `src/ir/verify/mod.rs:2991-3008`,
  `src/resolver/mod.rs:132-141`). It answers the opposite question and its
  narrowness is specified (`src/ir/link.rs:5-8`). `CBuffer` must **not** be added
  to it — a wrapper's MFBASIC-facing signature never names `CBuffer`, it names
  `List OF Byte`, so the escape rule needs no new entry.
- No `.mfp` format change. `BINARY_REPR_VERSION` is unchanged: `BUFFER`'s `SIZE`
  expression reuses the existing `IrLinkExpr` encoding, appended to the existing
  LINK trailer. (plan-58-C covers the encode/decode and its version question — if
  it concludes a bump is needed, this sub-plan does not pre-empt it.)
- No change to any currently-valid ctype's semantics. Zero acceptance-golden
  churn outside the new negative fixtures.
- The `IrLinkExpr` arithmetic extension (`*`, `+`, `-`) that plan-58-B's `LENGTH`
  clause needs is **not** in scope here; `SIZE` accepts the expression grammar as
  it stands plus that extension, whichever has landed.

## 2. Current State

**The ctype namespace is closed and hand-maintained.** `abi_slot_ctype_is_known`
(`src/ir/link.rs:16-35`) enumerates 15 names. Two position predicates carve it up:
`abi_ctype_valid_as_argument` = everything but `CVoid` (`:41-43`);
`abi_ctype_valid_as_return` = everything but `CString` (`:52-54`). An `OUT` slot
is checked as a *return*, because it is a produced value
(`syntaxcheck/mod.rs:771-775`, `verify/mod.rs:3072-3076`).

**Every existing ctype is fixed-width.** `ctype_size_align`
(`src/ir/link.rs:104-120`) returns a constant `(size, align)` per name, and both
`compute_c_layout` (`:135`) and the thunk's fixed frame layout
(`link_thunk.rs:336-415`) rest on that. `CBuffer` is the first ctype whose size is
a runtime value — which is precisely why it needs its own clause rather than
riding the existing slot syntax.

**`parse_c_type_name` is a bare `consume_identifier`** (`ast/items.rs:1217`), so
`CBuffer` already parses; the entire vocabulary check is semantic. Adding the name
to the authority is therefore sufficient to make it *reachable*, and the position
rules are what make it *safe*.

### The latent hole this sub-plan closes

A LINK wrapper's MFBASIC return type is **almost entirely unvalidated** against
its ABI return. The only such check is the CSTRUCT one
(`syntaxcheck/mod.rs:509-519`: `RETURN <struct-slot>` forces
`return_type == decl.maps_to`), and it is not even mirrored on the package path.
`parse_type_name` (`src/ast/expr.rs:579-611`) accepts `List OF Byte` generically;
`is_c_abi_type` does not reject it; nothing in `check_link_function_in`
constrains a non-CSTRUCT return type.

So today:

```basic
FUNC bogus() AS List OF Byte
  SYMBOL "some_symbol"
  ABI (n CInt64) AS r CInt64
  RETURN r
END FUNC
```

compiles. `emit_return_passthrough` (`link_thunk.rs:1121-1220`) has no
List-building arm, so `RESULT_VALUE_REGISTER` receives a raw scalar which the
caller then dereferences as a collection block — garbage, with no diagnostic.
This is the same class of defect plan-50-A closed for unknown ctype *names*, left
open for return *types*. Since plan-58 makes `List OF Byte` a legitimate LINK
return, the rule must land with it.

## 3. Design Overview

Three layers, mirroring plan-50-A exactly:

```
src/ir/link.rs                      <-- the authority
  ├── abi_slot_ctype_is_known          + "CBuffer"
  ├── abi_ctype_valid_as_argument      - "CBuffer"   (OUT-only)
  ├── abi_ctype_valid_as_return        + "CBuffer"   (it is a produced value)
  ├── ctype_size_align                 -> None       (no CSTRUCT field, like CVoid)
  └── check_buffer_slots(...)          <-- NEW: the four position rules, shared
          │
          ├── src/syntaxcheck/mod.rs:check_link_function_in   (slot-level span)
          └── src/ir/verify/mod.rs:check_link_functions       (function-level span)
```

`check_buffer_slots` is a **shared** function in `ir::link`, called from both
gates, rather than two hand-mirrored implementations. This is a deliberate
departure from the older `NATIVE_*` rules, which are duplicated verbatim between
the passes: the newer `check_cstruct` / `check_struct_slot`
(`src/ir/link.rs:285-361`, `:223-274`) already established the shared-helper
shape, and duplication is what let the two `is_c_abi_type` copies drift.

**Where the correctness risk concentrates:** in the position rules being
*complete*. `CBuffer` is the first ctype that is not interchangeable across
positions, and every position this sub-plan forgets to reject becomes a path that
reaches plan-58-B's marshaler with an assumption it does not hold — a wrong-sized
or unallocated buffer handed to a C function. The mitigation is a negative test
per rule (§Validation), not reasoning about which positions are reachable.

**Rejected alternative:** *infer the buffer size from a `CInt64` sibling slot by
naming convention* (e.g. `buf OUT CBuffer` + `buflen CInt64` → use `buflen`).
Rejected: it is implicit, unstated in the ABI line, and silently picks the wrong
slot when a C function takes two lengths. The `BUFFER … SIZE` clause states the
relationship the C API actually has.

**Rejected alternative:** *make `CBuffer` an `INOUT` ctype so a binding can also
send bytes.* Rejected for now — a send direction needs a `List OF Byte` **input**
marshal (copy the capacity-based data region into a native buffer), which is
independent work with its own failure modes. `INOUT CBuffer` is rejected here and
left as the obvious extension point; see §Open Decisions.

**Rejected alternative:** *validate in the parser for a better span.* Rejected for
the same reason plan-50-A rejected it (`:152-156`): the parser cannot protect the
package path, and a second list there is drift bait.

## 4. Detailed Design

### 4.1 The authority

```rust
// src/ir/link.rs
pub(crate) fn abi_slot_ctype_is_known(ctype: &str) -> bool {
    matches!(ctype,
        "CPtr" | "CString" | "CBuffer"
            | "CInt8" | "CInt16" | "CInt32" | "CInt64"
            | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
            | "CBool" | "CByte" | "CFloat" | "CDouble" | "CVoid")
}

/// `CBuffer` joins `CVoid` in the exclusion: it is a *produced* value only.
/// An input byte buffer is a separate, unimplemented direction (plan-58-A §3).
pub(crate) fn abi_ctype_valid_as_argument(ctype: &str) -> bool {
    abi_slot_ctype_is_known(ctype) && !matches!(ctype, "CVoid" | "CBuffer")
}
```

`abi_ctype_valid_as_return` is unchanged in form (`!= "CString"`) and so newly
admits `CBuffer` — correct, because `OUT` slots route through it
(`syntaxcheck/mod.rs:771-775`). But note the asymmetry this creates and document
it in the doc comment: `CBuffer` passes `valid_as_return` yet must **not** be
accepted as the literal ABI return (`AS r CBuffer`) — a C function does not
return a caller-allocated buffer. That is rule (5) below, checked separately
against `abi_return_ctype`. Do not try to express it by splitting the predicate
further; the existing two predicates already conflate "OUT slot" with "ABI
return", and adding a third would be a fourth list to drift.

`ctype_size_align("CBuffer")` returns `None`, keeping it out of CSTRUCTs via the
existing `NATIVE_ABI_UNKNOWN_CTYPE` path at `src/ir/link.rs:323-328`.

### 4.2 Syntax

```
abiSlot   := name [ "OUT" | "INOUT" | "IN" ] ctype
bufferCl  := "BUFFER" slotName "SIZE" linkExpr
```

`BUFFER` is a new contextual keyword inside a `LINK FUNC` body, parsed alongside
`CONST` / `BIND IN` / `SUCCESS_ON` / `RETURN` in `src/ast/items.rs`. `SIZE` is the
**byte** capacity — not frames, not elements. Stating the unit in the keyword was
considered (`SIZE_BYTES`) and rejected as noise; the spec and the DOC template
carry it instead.

New IR, appended to `IrLinkFunction` (`src/ir/link.rs:377-434`):

```rust
/// `BUFFER <slot> SIZE <expr>` — the byte capacity of an `OUT CBuffer` slot.
/// Exactly one per `CBuffer` slot; a `CBuffer` slot without one is rejected.
pub(crate) struct IrBuffer {
    pub(crate) slot: String,
    pub(crate) size: IrLinkExpr,
}
```
plus `pub(crate) buffers: Vec<IrBuffer>` on `IrLinkFunction`.

### 4.3 The rules

`check_buffer_slots(function) -> Vec<CStructFault>` (reusing the existing
`CStructFault { rule, message }` carrier, `src/ir/link.rs:199-202`):

| # | Rejects | Rule |
|---|---|---|
| 1 | a `CBuffer` slot whose direction is `In` or `InOut` | `NATIVE_BUFFER_INVALID` |
| 2 | a `CBuffer` slot with no `BUFFER` clause, or >1 naming it | `NATIVE_BUFFER_INVALID` |
| 3 | a `BUFFER` clause naming an unknown slot, or a non-`CBuffer` slot | `NATIVE_BUFFER_INVALID` |
| 4 | a `CBuffer` slot that is `CONST`-pinned | `NATIVE_CONST_OUT` (existing; it is an OUT slot) |
| 5 | `abi_return_ctype == "CBuffer"` | `NATIVE_ABI_UNKNOWN_CTYPE` (existing; position rule) |
| 6 | a `CBuffer` slot not named by `RETURN` | `NATIVE_BUFFER_INVALID` — an unreachable buffer is always a mistake, and unlike a scalar OUT it costs an allocation |
| 7 | `RETURN` names a `CBuffer` slot but `return_type != "List OF Byte"` | `NATIVE_BUFFER_INVALID` |
| 8 | `return_type == "List OF Byte"` but `RETURN` does not name a `CBuffer` slot | `NATIVE_BUFFER_INVALID` — closes the §2 hole |
| 9 | a `BUFFER` `SIZE` expression naming an unknown slot/param | `NATIVE_ABI_UNBOUND_SLOT` (existing; reuse `link_expr_var_names`, `src/ir/link.rs:163-175`) |

Rules 4, 5 and 9 reuse existing diagnostics — do not mint new codes for
conditions the existing rules already name. Rules 1, 2, 3, 6, 7, 8 share one new
rule with a distinguishing message.

Note rule 8's exact spelling: the canonical type string is whatever
`src/docs/spec/architecture/type-name-encoding` produces for a byte list. Read it
rather than hardcoding `"List OF Byte"` from this document.

### 4.4 The rule table entry

| Code | Name | Severity |
|---|---|---|
| next free in `2-203` | `NATIVE_BUFFER_INVALID` | Error |

Take the next free code in the `2-203` subsystem (`src/rules/table.rs`; the
native-library block tail is at `:830-860`) — the spec is explicit that a new rule
takes the next free code rather than backfilling a gap
(`01_rule-codes.md:116-117`). Add the matching row to
`src/docs/spec/diagnostics/01_rule-codes.md` **in the same change**, or
`every_rule_is_documented_in_the_spec` (`src/rules/mod.rs`) fails the suite. No
new *runtime* error code, so `02_error-codes.md` is untouched.

## Compatibility / Format Impact

- **Changes:** a LINK wrapper declaring `AS List OF Byte` without a `CBuffer`
  result slot now fails to compile (rule 8). Any such binding was already
  miscompiling to garbage (§2), so no correct program changes behavior. Nothing
  in-tree does this — grep `bindings/` and `tests/` before landing.
- **Changes:** `CBuffer` becomes a reserved ctype name; a binding using it as an
  identifier for something else would newly fail. Nothing in-tree does.
- **Unchanged:** `.mfp` byte format and `BINARY_REPR_VERSION`; `is_c_abi_type` in
  all three copies; every marshaling path for every existing ctype; every
  generated thunk's instruction sequence.
- Spec `17_native-libraries.md` gains `CBuffer` in the ctype table, a `BUFFER`
  clause subsection, and the new rule in §Rules.

## Phases

This sub-plan is one landable unit; the list below is its task breakdown.

### Phase 1 — vocabulary, clause, gates, spec, tests

- [ ] `src/ir/link.rs`: add `"CBuffer"` to `abi_slot_ctype_is_known`; exclude it
      from `abi_ctype_valid_as_argument`; extend both doc comments with the
      asymmetry note from §4.1. Confirm `ctype_size_align` returns `None`.
- [ ] `src/ir/link.rs`: add `IrBuffer` + `IrLinkFunction.buffers`; add
      `check_buffer_slots` implementing rules 1,2,3,6,7,8 of §4.3.
- [ ] `src/ast/items.rs`: parse `BUFFER <slot> SIZE <expr>` in the LINK FUNC body
      alongside `CONST`/`BIND IN`; add `AstBuffer` to `src/ast/types.rs`.
- [ ] `src/ir/lower.rs`: carry `buffers` from AST to IR (`link_functions`, ~`:292`).
- [ ] Source gate: call `check_buffer_slots` from
      `src/syntaxcheck/mod.rs:check_link_function_in`, with the slot's own line;
      add rules 5 and 9 alongside the existing checks at `:752-787`.
- [ ] Package gate: call it from `src/ir/verify/mod.rs:check_link_functions`
      (`:3042-3079`), function-level span.
- [ ] Add `NATIVE_BUFFER_INVALID` to `src/rules/table.rs` **and** a row in
      `src/docs/spec/diagnostics/01_rule-codes.md`.
- [ ] `src/ir/link.rs:549-575` (`ctype_list_is_exhaustive`) and
      `src/target/shared/code/link_thunk.rs:2009-2012` (`CTYPES`): add `"CBuffer"`
      to the literal list. It will then reach `emit_return_passthrough`'s `Err`
      arm (`:1209`) — **expected**; exclude `CBuffer` from that test's loops with
      a comment naming plan-58-B, rather than weakening the assertion.
- [ ] Spec: `src/docs/spec/language/17_native-libraries.md` — add `CBuffer` to the
      ctype table with its OUT-only restriction, a `BUFFER … SIZE` subsection
      stating the **byte** unit and the `List OF Byte` return contract, and
      `NATIVE_BUFFER_INVALID` in §Rules. Cite
      `[[src/ir/link.rs:check_buffer_slots]]`. Mark the marshaling as
      *Implementation status: declared but not yet lowered (plan-58-B)* — do not
      describe behavior that does not exist.
- [ ] Tests: one negative fixture per rule under `tests/syntax/native/`, mirroring
      `native-abi-unknown-ctype-invalid/`: `native-buffer-in-direction-invalid`,
      `native-buffer-no-size-invalid`, `native-buffer-size-unknown-slot-invalid`,
      `native-buffer-not-returned-invalid`, `native-buffer-wrong-return-type-invalid`,
      `native-bytelist-return-without-buffer-invalid` (rule 8), and
      `native-buffer-as-abi-return-invalid` (rule 5).
- [ ] Tests: package-path unit tests in `src/ir/verify/tests.rs` for at least
      rules 1, 2 and 8, mirroring `rejects_link_out_slot_not_return` (`:2653`) —
      a crafted `.mfp` must be rejected exactly as source is.

Acceptance: each of the seven negative fixtures fails to compile with the named
rule and a message identifying the offending slot; the same three link tables fed
through a crafted `.mfp` are rejected by `ir::verify` with the same rules; a
*valid* `CBuffer` declaration is accepted by both gates and then fails at
`lower_link_thunk` with the `Err` arm (proving the gates pass it through and the
backend is the only thing missing); `scripts/test-accept.sh target/debug/mfb
target/accept-actual` is green with golden churn confined to the new fixtures.
Commit: —

## Validation Plan

- Tests: the seven `tests/syntax/native/*-invalid/` fixtures above (source
  rejection, one per rule); `src/ir/verify/tests.rs` (package-path rejection);
  the updated `ctype_list_is_exhaustive` and `every_known_ctype_lowers` drift
  guards. Per `.ai/compiler.md` these are the invalid-usage cases; the valid side
  arrives with plan-58-B, which is why this sub-plan's acceptance asserts the
  `Err` arm rather than a passing program.
- Runtime proof: **none, by design.** This sub-plan ships no runtime behavior —
  it is a rejection surface. Do not claim otherwise; the Hard Completion Gate
  applies to plan-58-B.
- Doc sync: `src/docs/spec/language/17_native-libraries.md` and
  `src/docs/spec/diagnostics/01_rule-codes.md`. Then `cargo build`,
  `cargo test --bin mfb spec`, and confirm no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`;
  `scripts/artifact-gate.sh` to confirm existing thunks are byte-identical.

## Open Decisions

- **Rule 8 may be a separable bug fix.** Rejecting `AS List OF Byte` on a wrapper
  with no `CBuffer` result closes a live garbage-codegen hole that predates
  plan-58 entirely. Recommend: land it here (it is three lines once
  `check_buffer_slots` exists, and shipping `CBuffer` without it leaves the hole
  half-closed), **and** file it as a bug so the defect is recorded independently
  of this feature. Alternative: a standalone bug fix landing first — better
  provenance, but it duplicates the plumbing.
- **`INOUT CBuffer` (an input byte buffer).** Rejected in this sub-plan.
  Recommend keeping it rejected until a binding needs it; the marshal is
  independent work (copy the list's data region out to native storage) with its
  own lifetime question. The rule-1 message should say
  "not yet supported" rather than "invalid", so the extension point is legible.
- **Should `BUFFER`'s `SIZE` be capped?** A `CBuffer` allocates from the arena at
  a size the *caller* chooses, unlike `MAX_CSTRUCT_SIZE`'s frame-overflow bound.
  Recommend: no compile-time cap (the size is a runtime value, so there is
  nothing to check), but plan-58-B **must** gate it at runtime — see its
  `CBUFFER_MAX_BYTES` decision.

## Summary

The engineering risk is completeness of the position rules: `CBuffer` is the
first ctype that is not position-interchangeable, so any position left
unrejected becomes a path into plan-58-B's marshaler with a broken invariant.
Mitigated by one negative fixture per rule and by mirroring every rule on the
package path.

Untouched: all three `is_c_abi_type` copies, the `.mfp` format, and every
existing ctype's marshaling. No runtime behavior ships here — a valid `CBuffer`
declaration still fails to lower, deliberately.
