# plan-50-A: validate ABI slot C types

Last updated: 2026-07-16
Overall Effort: x-large (1d–3d) — the whole plan-50 feature (A–I)
Effort: medium (1h–2h)
Depends on: nothing

Closes the hole that makes the rest of plan-50 unsafe to build: an `ABI (...)`
slot's C type is parsed as a **free identifier** and is never validated against
any list, at any stage. A typo'd or unknown ctype compiles clean and silently
marshals as a raw 64-bit load.

This phase adds a slot-ctype allow-list and a diagnostic, and deletes the silent
codegen fallthrough. It ships no new feature and has no callers — it is pure
hardening, separately valuable on its own merits, and it is a **precondition**
for plan-50-B: the moment we introduce a struct ctype, a misspelled
`CStrcut SfInfo` would otherwise lower to an 8-byte scalar load against a
24-byte C struct, which is memory corruption rather than a compile error.

The single behavioral outcome: an `ABI` slot naming a C type the marshaling
backend does not implement is **rejected at compile time** with
`NATIVE_ABI_UNKNOWN_CTYPE`, on both the source path and the `.mfp` package path,
instead of silently marshaling as a raw 64-bit value.

References (read first):

- `src/ast/items.rs:parse_c_type_name` (`:969`) — the whole of ctype "validation"
  today: `consume_identifier(...)`. Any identifier is accepted.
- `src/target/shared/code/link_thunk.rs:856-862` — the silent default arm:
  ```rust
  _ => {
      instructions.push(abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), cret_off));
  }
  ```
- `src/syntaxcheck/helpers.rs:is_c_abi_type` (`:204`) and its twin
  `src/ir/verify/mod.rs:is_c_abi_type` (`:2611`) — **do not touch these.** They are
  deliberately narrower and answer the *opposite* question (which names are
  banned from a wrapper's MFBASIC-facing signature, via `NATIVE_CPTR_ESCAPE`,
  `src/ir/verify/mod.rs:2633-2652` / `src/syntaxcheck/mod.rs:414-441`). The spec
  states the narrowness is intentional — see below. This plan adds a *new,
  separate* list for slot ctypes.
- `src/docs/spec/language/17_native-libraries.md:~100` — documents today's
  behavior as intentional: *"The ABI-slot C type is not validated against a fixed
  list by the source checker; it is parsed as a free identifier and any name not
  handled below falls through to raw 64-bit passthrough."* and the Implementation
  status note flagging `CFloat32`/`CIntPtr`/`CUIntPtr`/`CSize` as *"not
  diagnosed; it simply marshals as a raw 64-bit register value, which is usually
  wrong for floats and narrow integers."* Both statements become false in this
  phase and must be rewritten.
- `src/rules/table.rs:939-999` — the `2-203-0089`–`0099` native-ABI rule block.
- `src/docs/spec/diagnostics/01_rule-codes.md:52` — the `G-SSS-EEEE` scheme.
- `.ai/compiler.md` — Hard Completion Gate; `.ai/specifications.md` — spec-sync rule.

## 1. Goal

- A `LINK` wrapper whose `ABI (...)` slot names a ctype outside the implemented
  set (e.g. `CStrcut`, `CFloat32`, `CIntPtr`, `CSize`, `Cint32`) is rejected with
  `NATIVE_ABI_UNKNOWN_CTYPE` (`2-203-0123`), naming the offending slot and ctype.
- The same rejection fires on the **package path** (`ir::verify`), not just on
  source — a crafted `.mfp` link table drives raw C calls and is a marshaling
  security gate (the argument already made for `check_link_functions`,
  `src/ir/verify/mod.rs:2602-2609`).
- The ABI **return** ctype (`abi_return_ctype`) is validated by the same list.
- `src/target/shared/code/link_thunk.rs`'s default arm no longer silently emits a
  raw 8-byte load: after validation it is unreachable, so it returns `Err`
  (the `lower_link_thunk` signature is already `Result<CodeFunction, String>`).
- Every existing binding (`bindings/sqlite3`, the `tests/rt-behavior/native/**`
  suite) still compiles and runs **byte-identically** — this phase rejects only
  what was already broken.

### Non-goals (explicit constraints)

- **Do not change `is_c_abi_type` in either location.** Its narrowness (excluding
  `CBool`, `CByte`, `CVoid`) is deliberate and specified; widening it would newly
  reject `CBool`/`CByte` from wrapper signatures and break working bindings.
- No new ctype is *added* in this phase — the allow-list enumerates exactly what
  the thunk implements today, nothing more.
- No `.mfp` format change. `BINARY_REPR_VERSION` stays `4` (plan-50-C bumps it).
- No change to marshaling semantics for any currently-valid ctype. Acceptance
  goldens for existing native tests must not churn.

## 2. Current State

`AbiSlot.ctype` (`src/ast/types.rs:339`) is filled by
`src/ast/items.rs:parse_c_type_name:969`, which is a bare `consume_identifier`.
It flows unvalidated through `src/ir/lower.rs:link_functions:292` into
`IrAbiSlot.ctype` (`src/ir/link.rs:62`), through `.mfp` encode/decode
(`src/ir/binary.rs:279-283` / `:394-400`), and reaches the thunk as a string
`match`.

Nothing validates it anywhere. The two `is_c_abi_type` lists are not slot
validators — their doc comment (`src/syntaxcheck/helpers.rs:200-203`) says so
explicitly: *"Whether `type_name` is a raw C ABI type that may appear only inside
an `ABI (...)` slot, never in a wrapper's MFBASIC-facing signature."* They run
over `params`/`return_type`, not over slots.

The two lists have **already drifted from the backend**: both omit `CBool`,
`CByte`, and `CDouble` handling that `link_thunk.rs` implements (`:835`, `:849`,
`:800`). This is not a bug in them (they answer a different question), but it
demonstrates the hazard of a hand-maintained ctype list with no single authority.

The set the thunk **actually implements**, read off the code:

| ctype | argument path | return path |
|---|---|---|
| `CString` | `:439` `emit_copy_string_to_cstring` | — |
| `CInt32` | `:448` range-checked → `ErrOverflow` | `:828` sign-extended |
| `CDouble` | `:479` loaded into `fp_argument_register` | `:800` NaN/Inf rejected |
| `CPtr` | `:459` raw 64-bit | `:790` → `String` copy-out, else `:821` raw |
| `CInt64` | `:459` raw 64-bit | `:821` raw |
| `CBool` | `:459` raw 64-bit | `:835` nonzero→TRUE |
| `CByte` | `:459` raw 64-bit | `:849` low 8 bits |
| `CInt8`/`CInt16`/`CUInt8`/`CUInt16`/`CUInt32`/`CUInt64` | `:459` raw 64-bit | `:856` raw (default arm) |
| `CVoid` | — | return-type only |

Note the last two rows are **why the default arm cannot simply become an error
without an allow-list first**: the narrow integers legitimately land there today.
The allow-list must therefore enumerate them as *accepted* while still rejecting
unknown names — i.e. the fix is "validate the name", not "delete the arm".

Precedent to mirror: `check_link_functions` (`src/ir/verify/mod.rs:2610`) is the
established shape for a package-path marshaling gate, and its header comment
states the security rationale this phase extends.

## 3. Design Overview

One new function — the single authority for slot ctypes — consulted from three
places:

```
src/ir/link.rs:abi_slot_ctype_is_known(&str) -> bool     <-- new, the authority
        │
        ├── src/syntaxcheck/mod.rs:check_link_function    (source path, slot-level span)
        ├── src/ir/verify/mod.rs:check_link_functions      (package path, function-level span)
        └── src/target/shared/code/link_thunk.rs           (default arm -> Err, unreachable)
```

It lives in `src/ir/link.rs` because that module already owns `IrAbiSlot` and is
reachable from both the frontend and the backend without a new dependency edge.

The list is deliberately **flat and hand-written**, not derived from the thunk's
`match`: Rust cannot enumerate match arms, and a derived list would be a
liability the first time an arm is added without a test. The drift guard is a
unit test that asserts every accepted ctype produces a thunk without hitting the
`Err` arm (§Validation).

**Where the risk concentrates:** the risk here is *over*-rejection — a ctype some
existing binding uses that this list omits would break a working build. Mitigated
by enumerating from the code table in §2 rather than from the spec prose, and by
the full acceptance suite plus `bindings/sqlite3` rebuild.

Rejected alternative: *validate in the parser* (`parse_c_type_name`). Rejected
because it would give the package path no protection — a crafted `.mfp` never
runs the parser, and the package path is precisely the security-relevant one.
Validation must live at syntaxcheck + `ir::verify`, mirroring how every other
`NATIVE_*` rule is enforced in both places.

Rejected alternative: *make the default arm an error and skip the allow-list*.
Rejected because the narrow integers (`CInt8`, `CUInt64`, …) legitimately reach
the default arm today (§2), so this would reject valid bindings.

## 4. Detailed Design

### 4.1 The authority

```rust
// src/ir/link.rs
/// Whether `ctype` is a C ABI type the marshaling backend implements for an
/// `ABI (...)` slot or the ABI return. This is the *slot* namespace — distinct
/// from `is_c_abi_type`, which is the narrower set banned from a wrapper's
/// MFBASIC-facing signature (`NATIVE_CPTR_ESCAPE`) and deliberately excludes
/// `CBool`/`CByte`/`CVoid`.
pub fn abi_slot_ctype_is_known(ctype: &str) -> bool {
    matches!(
        ctype,
        "CPtr" | "CString"
            | "CInt8" | "CInt16" | "CInt32" | "CInt64"
            | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
            | "CBool" | "CByte" | "CFloat" | "CDouble" | "CVoid"
    )
}
```

`CFloat` is included: the spec lists it as a supported 32-bit float and
`is_c_abi_type` carries it, though note it currently lands on the raw-64-bit
paths in the thunk (`:459`/`:856`) — that is a **pre-existing marshaling gap, not
this phase's to fix**. Record it in §Open Decisions; do not silently "fix" it
here.

`CVoid` is accepted as an ABI **return** ctype only. A slot (non-return) declared
`CVoid` is meaningless; reject it with the same rule.

### 4.2 The two call sites

- **Source path** — `src/syntaxcheck/mod.rs:check_link_function` (near the
  existing slot loop that raises `NATIVE_ABI_UNBOUND_SLOT`/`NATIVE_CONST_OUT`).
  Emit with the slot's own line (`AbiSlot.line`, `src/ast/types.rs:343`) so the
  diagnostic points at the slot, matching the existing source-path spans.
- **Package path** — `src/ir/verify/mod.rs:check_link_functions:2610`, in the
  existing `for slot in &function.abi_slots` loop (`:2661`). Function-level span,
  per that function's documented convention (`:2608`).

Both check every slot's `ctype` **and** `function.abi_return_ctype`.

### 4.3 The codegen arm

`src/target/shared/code/link_thunk.rs:856-862` becomes:

```rust
other => {
    return Err(format!(
        "LINK function '{}.{}' has unknown ABI return ctype '{other}'",
        function.alias, function.name
    ));
}
```

This requires threading `function` into `emit_return_marshal` (it already takes
`function`, per `:789`) and making that helper return `Result<(), String>`. The
narrow integers must be listed **explicitly** in the match (joining `CPtr` |
`CInt64` at `:821`) so they keep their current raw-load behavior — this is the
step that makes the arm unreachable rather than changing anyone's semantics.

This is defense in depth, not the primary gate: verification already rejected the
name. It exists so a future ctype added to the allow-list without a thunk arm
fails loudly at build time instead of silently marshaling garbage.

### 4.4 The rule

New entry in `src/rules/table.rs`:

| Code | Name | Severity |
|---|---|---|
| `2-203-0123` | `NATIVE_ABI_UNKNOWN_CTYPE` | Error |

`2-203-0123` is the next free code in the `2-203` subsystem. Note the native-ABI
block `0089`–`0099` is **full** — it butts against `2-203-0100`
(`TYPE_RESOURCE_ELEMENT_NOT_OWNER`) — so this rule cannot sit contiguously with
its semantic siblings and instead follows the native-library block's tail
(`…0122` `NATIVE_LIBRARY_VENDOR_COLLISION`). Codes are display labels and
`rule_for` keys on `name` (`src/rules/mod.rs:rule_for`), so this is cosmetic;
the spec explicitly says a new rule takes the next free code in range rather than
backfilling a gap (`01_rule-codes.md:116-117`).

`src/rules/table.rs` is **hand-maintained** and is *not* generated from any spec
file — only the runtime `errorCode::` registry
(`src/docs/spec/diagnostics/02_error-codes.md`) is build input via `build.rs:178`.
This phase adds **no runtime error code**, so `02_error-codes.md` is untouched.

It does, however, add a **rule**, and as of `afdcceb6` those are drift-guarded:
`every_rule_is_documented_in_the_spec` (`src/rules/mod.rs`) asserts each `RULES`
entry's code and name appear in `01_rule-codes.md`. The rule table and the spec
table must therefore be updated together — the guard exists precisely because
plan-46-D shipped `2-203-0122` into `RULES` without a spec row. Every rule-adding
sub-plan in plan-50 (A, B, E) carries this obligation.

## Compatibility / Format Impact

- **Changes:** a `LINK` binding with an unknown/misspelled slot ctype now fails
  to compile. Any such binding was already miscompiling (silent raw 64-bit
  marshal), so no correct program changes behavior.
- **Unchanged:** `.mfp` byte format (`BINARY_REPR_VERSION` stays `4`); all
  marshaling semantics for known ctypes; `is_c_abi_type` and `NATIVE_CPTR_ESCAPE`;
  every generated thunk's instruction sequence.
- Spec `17_native-libraries.md` prose changes from "free identifier / falls
  through to raw 64-bit passthrough" to the validated behavior.

## Phases

This sub-plan is one landable unit; the list below is its task breakdown.

### Phase 1 — allow-list, both gates, spec, tests

Adds the authority and wires it into both checkers plus codegen.

- [ ] Add `abi_slot_ctype_is_known` to `src/ir/link.rs` with the doc comment from
      §4.1 explaining why it is separate from `is_c_abi_type`.
- [ ] Add rule `NATIVE_ABI_UNKNOWN_CTYPE` = `2-203-0123` to `src/rules/table.rs`
      **and a matching row in `src/docs/spec/diagnostics/01_rule-codes.md`** — the
      `every_rule_is_documented_in_the_spec` guard (`src/rules/mod.rs`, added
      `afdcceb6`) fails the suite otherwise.
- [ ] Source gate: emit it from `src/syntaxcheck/mod.rs:check_link_function` for
      every slot ctype and for `abi.return_ctype`, using the slot's line; reject
      a non-`return` slot declared `CVoid`.
- [ ] Package gate: emit it from `src/ir/verify/mod.rs:check_link_functions`
      (`:2661` loop) for slot ctypes and `abi_return_ctype`.
- [ ] Codegen: list the narrow integers explicitly in the return match and turn
      the default arm at `link_thunk.rs:856-862` into `Err` (§4.3).
- [ ] Spec: rewrite the "not validated against a fixed list … falls through to
      raw 64-bit passthrough" paragraph and the `CFloat32`/`CIntPtr`/`CSize`
      Implementation-status note in
      `src/docs/spec/language/17_native-libraries.md`; add
      `NATIVE_ABI_UNKNOWN_CTYPE` to the diagnostics list at its `§Rules` tail.
      Cite `[[src/ir/link.rs:abi_slot_ctype_is_known]]`.
- [ ] Tests: new `tests/syntax/native/native-abi-unknown-ctype-invalid/` proving
      a misspelled slot ctype is rejected (mirror the existing
      `tests/syntax/native/native-abi-unbound-slot-invalid/` layout).
- [ ] Tests: unit test in `src/ir/verify/tests.rs` asserting the package path
      rejects an unknown ctype (mirror `rejects_link_out_slot_not_return`
      `:2653`), and one asserting `CVoid` in a non-return slot is rejected.
- [ ] Tests: drift guard — a unit test that, for **every** ctype accepted by
      `abi_slot_ctype_is_known`, builds a minimal `IrLinkFunction` and asserts
      `lower_link_thunk` returns `Ok` (i.e. no accepted ctype reaches the `Err`
      arm). This is the test that keeps the list and the backend in step.

Acceptance: a binding declaring `ABI (n CIint32) AS return CInt32` fails to
compile with `NATIVE_ABI_UNKNOWN_CTYPE` naming slot `n`; the same link table fed
through a crafted `.mfp` is rejected by `ir::verify`; `bindings/sqlite3` rebuilds
and `tests/rt-behavior/native/native-link-sqlite-rt` still passes with an
unchanged golden; `scripts/test-accept.sh` is green with **zero** golden churn.
Commit: `e98645c7`

**Landed note.** The drift guard found a real gap on its first run: `CString` was
in the allow-list but has **no return arm**, because a `char *` return is spelled
`CPtr` + a `String` wrapper. So the predicate split in two —
`abi_ctype_valid_as_argument` (everything but `CVoid`) and
`abi_ctype_valid_as_return` (everything but `CString`) — and an `OUT` slot is
checked as a return, since it is a produced value. That position rule was not in
the plan; the test found it.

## Validation Plan

- Tests: `tests/syntax/native/native-abi-unknown-ctype-invalid/` (source
  rejection); `src/ir/verify/tests.rs` (package-path rejection, `CVoid` slot
  rejection); the accepted-ctype→thunk drift guard in
  `src/target/shared/code/` unit tests. Per `.ai/compiler.md` these are the
  invalid-usage cases; the "valid" side is the unchanged existing native suite.
- Runtime proof: `tests/rt-behavior/native/native-link-sqlite-rt` continues to
  pass — this phase must not perturb a working native call. Rebuild
  `bindings/sqlite3` and confirm the emitted thunks are byte-identical
  (`scripts/artifact-gate.sh`, the execution-free codegen gate).
- Doc sync: `src/docs/spec/language/17_native-libraries.md` (two stale paragraphs
  + the new rule) **and `src/docs/spec/diagnostics/01_rule-codes.md`** — as of
  `afdcceb6` the test `every_rule_is_documented_in_the_spec` (`src/rules/mod.rs`)
  asserts every `RULES` entry's code **and** name appear in that table, so a new
  rule without a spec row now fails the suite. Add one row for
  `2-203-0123 NATIVE_ABI_UNKNOWN_CTYPE`, and re-check the subsystem population
  count at `:60-62` if it is stated numerically. Then `cargo build`,
  `cargo test --bin mfb spec`, and confirm no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
  Expect **zero** golden churn; any churn is a real regression, not a stale
  fixture.

## Open Decisions

- **`CFloat` is accepted but not marshaled as a 32-bit float.** The thunk routes
  it to the raw-64-bit paths (`:459`/`:856`), so a `CFloat` slot is almost
  certainly already miscompiling — the same class of bug this phase closes for
  unknown names. Recommend: accept `CFloat` in the allow-list now (do not
  newly break it), and file a separate bug for the marshaling gap via
  `/write-bug` rather than widening this phase. Alternative: reject `CFloat`
  until implemented — rejected, as it would break any binding using it today,
  and nothing in-tree does (grep `bindings/` before finalizing).
  Decision: File the bug.
- **Should the parser also reject early, for a better span?** Recommend no: the
  parser cannot protect the package path, and a second list there is drift bait.
  The slot-level span from syntaxcheck is already precise.
  Decision: No.

## Summary

The engineering risk is over-rejection: the allow-list is hand-written, so a
ctype omitted from it breaks a working binding. It is enumerated from the thunk's
actual code table (§2), guarded by a test that walks every accepted ctype through
`lower_link_thunk`, and backstopped by zero-churn acceptance.

Untouched: the `.mfp` format, `is_c_abi_type`/`NATIVE_CPTR_ESCAPE`, and every
marshaling path for a currently-valid ctype. No new capability ships here — this
phase only makes the ctype namespace closed, which is what lets plan-50-B add
`CStruct` to it safely.
