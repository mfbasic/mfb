# plan-58-A: the `CBuffer` slot ctype — vocabulary, position rules, and gates

Last updated: 2026-07-20
Overall Effort: large (3h–1d) — the whole plan-58 feature (A–D)
Effort: medium (1h–2h)
Depends on: nothing
Produces: `abi_slot_ctype_is_known("CBuffer")`, `IrBuffer`,
`IrLinkFunction::buffers`, `ir::link::check_buffer_slots`, rule
`NATIVE_BUFFER_INVALID` (`2-203-0132`), the `BUFFER <slot> SIZE <expr>` clause.
Consumed by B (marshaling), C (encode/decode), D (the binding).

Introduces `CBuffer` into the ABI slot ctype namespace as an **OUT-only,
runtime-sized byte buffer** that surfaces as `List OF Byte`, together with the
`BUFFER <slot> SIZE <expr>` clause that gives it a capacity. This sub-plan ships
**parsing and rejection only** — every accepted declaration still fails to lower,
loudly, at `link_thunk.rs`'s `Err` arm. plan-58-B makes it marshal.

Landing the vocabulary first is deliberate and mirrors plan-50-A→B: the moment a
new ctype exists, an unimplemented or mis-positioned one must fail as a
*diagnostic*, never as a silent raw 64-bit load. It also closes a latent hole
found while planning (§2.3): a LINK wrapper may already declare
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
- `src/docs/spec/language/17_native-libraries.md` — the ctype table and §Rules.
- `.ai/compiler.md`, `.ai/specifications.md`.

## Prerequisite: plan-57 must be COMPLETE

> ### **If plan-57 is not complete, plan-58 cannot be started. Full stop.**

This is a precondition on the whole feature, not a dependency to negotiate
mid-flight and not work plan-58 absorbs. plan-58 does not promote, port, finish,
or work around any part of plan-57. It waits.

**The entry check — run this before writing a line of plan-58:**

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-57-A…E all landed and archived to `planning/old-plans/` | `ls planning/plan-57-*` → no matches | **NOT MET** — A–E still in `planning/` |
| A `pub(crate)` byte-list constructor exists | `rg -n 'fn emit_alloc_list' src/` | **NOT MET** — no matches |
| A `pub(crate)` data-pointer helper exists | `rg -n 'fn emit_collection_data_pointer_into' src/` | **NOT MET** — no matches |
| `kind = 2` is the **default** representation, ungated | `rg -n 'MFB_KIND2' src/` → no matches | **NOT MET** — gate live at `builder_collection_layout.rs:2191` |

**As of 2026-07-20 none of the four are met, so plan-58 is not startable.** If any
row still fails when this plan is picked up, stop and finish plan-57. Do not
start plan-58-A "because A is independent" — A is cheap, but landing a ctype the
feature cannot finish leaves a known-but-unusable name in the ABI namespace.

Everything below is written against the post-plan-57 tree: `kind = 2` live,
`emit_alloc_list` available, no entry table. That is the *only* representation
plan-58 targets — there is no dual-mode support, no `MFB_KIND2` branch, and no
41×-cost fallback anywhere in A–D. If you find yourself adding one, the
precondition was not met and you are intertwining the two plans again.

### What plan-57 completion buys, and the numbers that follow from it

With `kind = 2` live, a `List OF Byte` block is `COLLECTION_HEADER_SIZE + N`
(40 + N), `dataBase = block + 40` is a **constant** offset, and there is no
entry-fill loop. Every capacity figure in plan-58 derives from that:

| | value | consequence |
|---|---|---|
| `CBUFFER_MAX_BYTES` (plan-58-B) | **64 MiB** | 64 MB of arena, 1.0× |
| `MAX_LOAD_BYTES` (plan-58-D) | **64 MiB** | **349.5 s ≈ 5.8 min** of stereo 48 kHz s16 |

(For contrast only, not a supported mode: under the pre-plan-57 `kind = 1`
layout the same buffer cost 41× — 344 MB for 8 MiB, 43.7 s of stereo. That is
the situation plan-57 exists to remove, and plan-58 simply does not ship into
it.)

## Dependency graph (whole feature)

```
   plan-57 COMPLETE (precondition — not a node in this graph)
                                    │
                                    ▼
   A (vocabulary) ──► B (marshaling) ──► C (.mfp path) ──► D (libsnd::loadSound)
```

Execution is topological over this graph, not alphabetical. Every letter is
gated behind the plan-57 precondition above; past that, A is first.

Letters are identifiers, not an order. Do not re-letter.

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
  slot is rejected — closing the pre-existing garbage-codegen hole in §2.3.
- `CBuffer` is rejected as a CSTRUCT field (`ctype_size_align` returns `None`,
  as `CVoid` does) and as the ABI return.
- Every rejection fires identically from `syntaxcheck` and from `ir::verify`, so a
  crafted `.mfp` gets exactly the source-path treatment
  (`src/ir/link.rs:279-281`).
- **`link_thunk.rs` is given an explicit `Err` arm for `CBuffer`** so no binding
  can use it yet. This is a real edit, not a pre-existing property — see §2.4.

### Non-goals (explicit constraints)

- **No marshaling.** That is plan-58-B. The only `CBuffer` code this sub-plan adds
  to `link_thunk.rs` is the `Err` arm that refuses to lower it (§2.4) — it must
  not emit a single *instruction* for `CBuffer`.
- **Do not touch `is_c_abi_type`** in any of its three copies
  (`src/syntaxcheck/helpers.rs:204-220`, `src/ir/verify/mod.rs:2991-3008`,
  `src/resolver/mod.rs:132-141`). It answers the opposite question and its
  narrowness is specified (`src/ir/link.rs:5-8`). `CBuffer` must **not** be added
  to it — a wrapper's MFBASIC-facing signature never names `CBuffer`, it names
  `List OF Byte`, so the escape rule needs no new entry.
- No `.mfp` format change *in this sub-plan*. `BINARY_REPR_VERSION` is unchanged
  here; plan-58-C owns the encode/decode and its version bump.
- No change to any currently-valid ctype's semantics. Zero acceptance-golden
  churn outside the new negative fixtures.
- The `IrLinkExpr` arithmetic extension (`*`, `+`, `-`) that plan-58-B's `LENGTH`
  clause needs is **not** in scope here.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| ABI slot ctypes in the authority | 15 | `rg -o '"C[A-Za-z0-9]+"' src/ir/link.rs \| sed -n '/16,35p/'` — enumerated at `src/ir/link.rs:16-35` |
| `2-203-01xx` rule codes already taken | 32 (`0100`–`0131`, contiguous, no gaps) | `rg -o '"2-203-01[0-9]{2}"' src/rules/table.rs \| sort -u \| wc -l` |
| Next free `2-203` code | **`2-203-0132`** | `rg -o '"2-203-[0-9]{4}"' src/rules/table.rs \| sort -u \| tail -1` → `0131`; `rg -c '2-203-0132' src/ docs/` → 0 |
| Copies of `is_c_abi_type` (must stay untouched) | 3 | `rg -n 'fn is_c_abi_type' src/` |

### 2.2 How the namespace works today

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

### 2.3 The latent hole this sub-plan closes

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

### 2.4 The thunk does NOT reject an unknown slot ctype — verified 2026-07-20

The 2026-07-19 draft asserted "`link_thunk.rs` still returns `Err` for `CBuffer`"
and built its Phase-1 acceptance on a valid declaration failing there. **That is
false.** Traced through the exact shape this sub-plan makes legal
(`buf OUT CBuffer` + `RETURN buf`):

| Step | Code | Behaviour for `CBuffer` |
|---|---|---|
| Staging | `link_thunk.rs:564-575` | `writes_back()` is true and it is not a CSTRUCT, so it takes the **generic scalar-OUT arm**. No ctype dispatch exists here. No error. |
| Arg loop | `:678-694` | dispatches only on `ctype == "CDouble"`. `CBuffer` loads the cslot into an integer arg register. No error. |
| Result marshal | `:852-858` | `result_out_ctype == Some("CBuffer")` falls to `_ =>`, a bare `load_u64(RESULT_VALUE_REGISTER, sp, out_off)`. **No error.** |
| `emit_return_passthrough` | `:860` | **Never reached** — it sits behind `else if result_var == Some(function.abi_return_name…)`. An OUT-slot result took the `if`. |

So without an explicit arm, a well-formed `CBuffer` wrapper lowers cleanly and
returns a **raw zeroed 8-byte word as a collection pointer** — reproducing the
exact garbage-codegen failure mode §2.3 says this sub-plan exists to prevent.

Two guards are therefore required in this sub-plan, not in B:

1. an explicit `"CBuffer" => Err(...)` arm in the OUT-slot result match at
   `:852`, ahead of the `_` default;
2. a `continue`-style guard in the staging loop at `:564` so `CBuffer` does not
   silently take the scalar-OUT path (mirror the CSTRUCT `continue` at `:562`).

Note the general hazard this exposes: the `_` default is a silent raw 8-byte load
for **any** future ctype. That is the bug-238 mechanism (a `CInt32` OUT surfacing
`-1` as `4294967295`). Worth filing separately; not this sub-plan's job to fix
generally.

### 2.5 Verified properties

Claims a `file:line` cannot settle, and how each was checked:

| Claim | Verdict | How checked |
|---|---|---|
| The LINK ABI has no bulk-buffer ctype | **CONFIRMED** | Read `abi_slot_ctype_is_known` `src/ir/link.rs:16-35` — 15 fixed-width names, `CSTRUCT` the only aggregate |
| `CBuffer` already parses today | **CONFIRMED** | `parse_c_type_name` `ast/items.rs:1217` is a bare `consume_identifier` |
| `2-203-0132` is free | **CONFIRMED** | Codes `0100`–`0131` contiguous in `table.rs`; `rg -c '2-203-0132' src/ docs/` → 0. Spec doc max is also `0131` |
| A `List OF Byte` LINK return compiles to garbage today | **CONFIRMED** | `emit_return_passthrough` `link_thunk.rs:1121-1220` has no List-building arm |
| `link_thunk.rs` rejects an unknown slot ctype like `CBuffer` | **FALSE** | §2.4 — staging, arg loop and result marshal all accept it; `emit_return_passthrough`'s `Err` is unreachable for an OUT slot (`:860`). An explicit arm is required |
| `emit_alloc_list` exists (plan-58-B's premise) | **FALSE** | See §Prerequisite. Never built — plan-57-B:336-341 marks it `[~]`, explicitly not built |
| kind = 2 is the live representation | **FALSE** | Gated on `MFB_KIND2`, `builder_collection_layout.rs:2191`. Off by default |
| `NATIVE_*` rule codes are densely ordered, so the tail gives the max | **FALSE** | The `2-203` block is non-monotonic — `0127`/`0128` precede `0126`; `0131` precedes `0130`. Scan the whole subsystem, never the tail |
| The canonical byte-list type string is `"List OF Byte"` | **CONFIRMED** | `src/docs/spec/architecture/21_type-name-encoding.md:29`; already used verbatim at `audio_specs.rs:100,201,212`, `fs_specs.rs:90,103` |

## 3. Design Overview

Three layers, mirroring plan-50-A exactly:

```
src/ir/link.rs                      <-- the authority
  ├── abi_slot_ctype_is_known          + "CBuffer"
  ├── abi_ctype_valid_as_argument      - "CBuffer"   (OUT-only)
  ├── abi_ctype_valid_as_return        + "CBuffer"   (it is a produced value)
  ├── ctype_size_align                 -> None       (no CSTRUCT field, like CVoid)
  └── check_buffer_slots(...)          <-- NEW: the position rules, shared
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

**Where design uncertainty concentrates:** nowhere in this sub-plan. Every
mechanism here has a landed precedent in plan-50-A, and §2.4 records the premises
as verified. This is why A is safe to start immediately while B's blocker is
resolved.

**Where correctness risk concentrates:** in the position rules being *complete*.
`CBuffer` is the first ctype that is not interchangeable across positions, and
every position this sub-plan forgets to reject becomes a path that reaches
plan-58-B's marshaler with an assumption it does not hold — a wrong-sized
or unallocated buffer handed to a C function. The mitigation is a negative test
per rule (§Validation), not reasoning about which positions are reachable.

**Rejected alternative:** *infer the buffer size from a `CInt64` sibling slot by
naming convention* (e.g. `buf OUT CBuffer` + `buflen CInt64` → use `buflen`).
Rejected: it is implicit, unstated in the ABI line, and silently picks the wrong
slot when a C function takes two lengths. The `BUFFER … SIZE` clause states the
relationship the C API actually has.

**Rejected alternative:** *make `CBuffer` an `INOUT` ctype so a binding can also
send bytes.* Rejected for now — a send direction needs a `List OF Byte` **input**
marshal, which is independent work with its own failure modes. `INOUT CBuffer` is
rejected here and left as the obvious extension point; see §Open Decisions.

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
        /* … the remaining 13 existing names, unchanged … */
    )
}
```

`abi_ctype_valid_as_argument` gains a `CBuffer` exclusion alongside `CVoid`;
`abi_ctype_valid_as_return` accepts it. `ctype_size_align` returns `None` for
`CBuffer`, which is what makes it invalid as a CSTRUCT field — the same mechanism
`CVoid` uses, so no new rejection path is needed for that case.

**But note which rule fires.** The 2026-07-19 draft claimed CSTRUCT rejection
would come through `NATIVE_ABI_UNKNOWN_CTYPE` at `link.rs:323-328`. It will not:
that arm is guarded by `!abi_slot_ctype_is_known(ctype)`, which `CBuffer` now
**passes**. Rejection instead falls through to `link.rs:330`
(`ctype_size_align(...).is_none()`) → **`NATIVE_CSTRUCT_INVALID`**. Write the
CSTRUCT-field fixture to expect that code, not the unknown-ctype one.

`tests::ctype_list_is_exhaustive` (`:549-575`) pins the authority against
`link_thunk.rs`'s `CTYPES` literal; both move in the same commit or the suite
fails. Expect this test to be the one that catches an incomplete edit.

### 4.2 The clause and the IR

`BUFFER <slot> SIZE <expr>` sits alongside `SYMBOL` / `ABI` / `RETURN` in the
LINK function body. A unit suffix on `SIZE` was considered (`SIZE_BYTES`) and
rejected as noise; the spec and the DOC template carry the unit instead.

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
| 8 | `return_type == "List OF Byte"` but `RETURN` does not name a `CBuffer` slot | `NATIVE_BUFFER_INVALID` — closes the §2.3 hole |
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
| `2-203-0132` | `NATIVE_BUFFER_INVALID` | Error |

`2-203-0132` is the next free code, measured in §2.1 — the spec is explicit that
a new rule takes the next free code rather than backfilling a gap
(`01_rule-codes.md:116-117`). If it is taken by the time this lands, re-run the
command in §2.1 and take the new next-free; do not backfill. Add the matching row
to `src/docs/spec/diagnostics/01_rule-codes.md` **in the same change**, or
`every_rule_is_documented_in_the_spec` (`src/rules/mod.rs`) fails the suite. No
new *runtime* error code, so `02_error-codes.md` is untouched.

## Compatibility / Format Impact

- **Changes:** a LINK wrapper declaring `AS List OF Byte` without a `CBuffer`
  result slot now fails to compile. This is a **fix** — such a wrapper produced
  garbage (§2.3) — but it is a source-compatibility break for any binding that
  declares one. Measured blast radius: `rg -l 'AS List OF Byte' bindings/` before
  landing; if any bundled binding trips it, that binding is already broken and
  the fix belongs in the same change.
- **Unchanged:** `BINARY_REPR_VERSION`, every existing ctype's semantics, the
  thunk's emitted bytes for every existing binding (`scripts/artifact-gate.sh`
  must be byte-identical), `is_c_abi_type` in all three copies.

## Phases

### Phase 1 — vocabulary, clause, gates, spec, tests

Everything in this sub-plan lands as one phase: the authority edit, the clause,
the shared checker, both gate call sites, the rule row, the spec row, and the
negative fixtures. Splitting it would ship a ctype that is known but unpoliced,
which is the exact state §2.3 shows to be dangerous.

- [ ] `src/ir/link.rs`: add `"CBuffer"` to `abi_slot_ctype_is_known` (`:16-35`);
      exclude from `abi_ctype_valid_as_argument` (`:41-43`); include in
      `abi_ctype_valid_as_return` (`:52-54`); return `None` from
      `ctype_size_align` (`:104-120`).
- [ ] `src/target/shared/code/link_thunk.rs`: add `"CBuffer"` to the `CTYPES`
      literal at `:2008-2011` so `ctype_list_is_exhaustive` (`link.rs:550`) passes.
      **Admitting `CBuffer` to `valid_as_return` breaks the test's loop 1
      (`:2016-2053`)**, which filters on that predicate — expect to fix it.
      Loop 2 (`:2055-2085`) uses `AbiDirection::In` + a `CONST` pin, which a
      `CBuffer` can never satisfy, so it needs no change.
- [ ] `link_thunk.rs`: **add the two refusal guards from §2.4** — an explicit
      `"CBuffer" => Err(...)` arm in the OUT-slot result match ahead of the `_`
      default at `:852`, and a `continue` guard in the staging loop at `:564`
      mirroring the CSTRUCT one at `:562`. Without both, a valid declaration
      lowers to garbage.
- [ ] Update **every `IrLinkFunction` struct-literal construction site** for the
      new `buffers` field — including both loops in `link_thunk.rs`'s
      `ctype_list_is_exhaustive` (`:2016-2085`), which name every field.
      `rg -n 'IrLinkFunction {' src/` to enumerate them.
- [ ] `src/ast/items.rs`: parse `BUFFER <slot> SIZE <expr>` in the LINK function
      body, near `parse_abi_spec` (`:1146-1205`).
- [ ] `src/ir/link.rs`: add `IrBuffer` and `IrLinkFunction::buffers` (`:377-434`);
      write `check_buffer_slots` implementing rules 1–9 (§4.3).
- [ ] `src/syntaxcheck/mod.rs:check_link_function_in`: call `check_buffer_slots`,
      mapping faults to slot-level spans.
- [ ] `src/ir/verify/mod.rs:check_link_functions` (`:3042-3079`): same call,
      function-level spans.
- [ ] `src/rules/table.rs`: add `NATIVE_BUFFER_INVALID` = `2-203-0132` in the
      native-ABI block (`:992-1058`).
- [ ] `src/docs/spec/diagnostics/01_rule-codes.md`: add the `2-203-0132` row.
- [ ] `src/docs/spec/language/17_native-libraries.md`: add `CBuffer` to the ctype
      table and the `BUFFER … SIZE` clause to §Rules, including the OUT-only and
      one-clause-per-slot constraints.
- [ ] Tests: one negative fixture per rule in §4.3 (rules 1, 2, 3, 6, 7, 8 each
      get their own; 4, 5, 9 assert the existing rule fires), under
      `tests/syntax/native/`, following `plan-50-A`'s fixture layout. Plus a
      package-path twin proving `ir::verify` rejects a crafted `.mfp` identically
      (`src/ir/coverage_tests.rs` pattern).
- [ ] Tests: a positive fixture — a well-formed `OUT CBuffer` declaration — that
      passes syntaxcheck and fails at `link_thunk.rs` with the not-yet-lowered
      `Err`, pinning the A/B boundary.

Acceptance: every rule in §4.3 has a fixture that fails with **its own** rule
code and message on the source path, and a package-path twin producing the same
code; the positive fixture reaches `link_thunk.rs`'s `Err`;
`ctype_list_is_exhaustive` and `every_rule_is_documented_in_the_spec` pass;
`scripts/artifact-gate.sh` shows every existing thunk byte-identical.
Commit: —

## Validation Plan

- Tests: negative fixture per rule (§Phase 1), both paths. Negative/error cases
  are the entire point of this sub-plan — a rule without a fixture is not landed.
- Coverage check: `tests/syntax/native/` fixtures are golden-backed, so these are
  in the gate's denominator. Confirm with `scripts/artifact-gate.sh` that the new
  fixtures produce goldens — note `tests/acceptance/` has **no** `golden/` dir by
  design, so do not put the proof there and assume it is covered.
- Runtime proof: none applicable — this sub-plan emits no instructions. Its proof
  is diagnostic behavior, which the fixtures carry. (Runtime proof arrives in B.)
- Doc sync: `17_native-libraries.md` (ctype table + §Rules),
  `01_rule-codes.md` (the `2-203-0132` row).
- Acceptance: the project's full suite, plus `scripts/artifact-gate.sh`.

## Open Decisions

1. **`INOUT CBuffer`** — rejected here; the send direction needs a `List OF Byte`
   input marshal, which is independent work with its own failure modes.
   Recommended: leave rejected, revisit only if a real binding needs it. (§3)

None of the open decisions here gate anything. The feature's only hard stop is
the plan-57 precondition, checked once before plan-58-A begins.

## Corrections

<!-- Filled in during execution. Record every place this document turned out to
     be wrong: the claim, what was actually true, and the evidence. -->

- 2026-07-20 — **plan-57 is a hard precondition, not a dependency to negotiate.**
  The 2026-07-19 draft asserted plan-57-B had introduced `emit_alloc_list` and
  `emit_collection_data_pointer_into` (both absent) and treated `kind = 2` as the
  live layout (it is gated off, `builder_collection_layout.rs:2191`). An interim
  rewrite made these mid-flight blockers with a "promote the helper here" escape
  hatch — which is the intertwining that caused the problem in the first place.
  Now: plan-57 complete is a **single up-front gate** (§Prerequisite), plan-58
  never does plan-57's work, and there is no dual-representation support anywhere
  in A–D.
- 2026-07-20 — **`2-203-0132` measured.** The draft said "next free in `2-203`"
  without a number; measured and pinned in §2.1/§4.4. Also: the draft said to
  read "the native-library block tail at `:830-860`" — the `2-203` block is
  **non-monotonic** (`0127`/`0128` precede `0126`; `0131` precedes `0130`), so
  reading the tail finds `0126` and misleads. Scan the whole subsystem.
- 2026-07-20 — **The thunk does not reject an unknown slot ctype.** The draft's
  central safety claim — "`link_thunk.rs` still returns `Err` for `CBuffer`" —
  is false, and its Phase-1 acceptance rested on it. Traced in §2.4: staging,
  the arg loop and the result marshal all accept it, and
  `emit_return_passthrough`'s `Err` is unreachable for an OUT slot (it sits
  behind an `else if` at `:860`). Two explicit guards added as Phase-1 tasks.
  **Without them this sub-plan ships the exact garbage-codegen hole it exists to
  close.**
- 2026-07-20 — **CSTRUCT rejection fires a different rule than the draft said.**
  Once `CBuffer` is known, `link.rs:323-328`'s `NATIVE_ABI_UNKNOWN_CTYPE` arm is
  guarded by `!abi_slot_ctype_is_known` and no longer applies; rejection falls to
  `:330` → `NATIVE_CSTRUCT_INVALID`. The fixture must expect that code.
- 2026-07-20 — **Adding `buffers` breaks every `IrLinkFunction` struct literal**,
  including both loops in `ctype_list_is_exhaustive` (`link_thunk.rs:2016-2085`).
  Unmentioned in the draft's task list; added.
- 2026-07-20 — Line drift corrected: the syntaxcheck gate is `:749-786` (draft
  said `:752-787`) and its routing `:770-774` (draft `:771-775`); the
  `ir::verify` mirror extends to ~`:3086`; `rejects_link_out_slot_not_return` is
  at `verify/tests.rs:3043` (draft `:2653`); the ctype table in
  `17_native-libraries.md` is at `:94-104`. Also noted: `verify/mod.rs:3053-3066`
  contains a **verbatim duplicated CSTRUCT-skip block** — dead, remove it when
  wiring `check_buffer_slots` (plan-58-C §4.3).

## Summary

The engineering risk in this sub-plan is *completeness of the position rules*,
not mechanism — every mechanism has a landed precedent in plan-50-A. Nine rules,
each with a fixture, on two gates that share one implementation.

What is left untouched: all 15 existing ctypes, `is_c_abi_type` in three copies,
the `.mfp` format, and every emitted thunk byte.

The feature's real risk is not here — it is B's missing constructor (§Prerequisite) and the
41× memory reality (§Kind-2 gate). Both are now measured rather than assumed, and neither
blocks A.
