# plan-50-H: the `RETURN <name>` clause — deleting the magic slot name

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-C (carries `result_slot` in the `.mfp`, so this needs no
second format bump — see §Open Decisions for landing it earlier instead)

Replaces the magic ABI slot name `return` with an explicit clause. Today a
wrapper's result is identified by a slot literally *named* `return`; after this,
every slot and the C return carry ordinary names and `RETURN <name>` says which
one is the result.

This is a **LINK surface change with no relation to structs** — it stands on its
own merits and would be worth doing if plan-50 did not exist. It is in plan-50
because plan-50-E needs it: struct slots made the old rule untenable (§3).

The single behavioral outcome: `bindings/sqlite3` compiles and runs identically
with every `return`-named slot renamed and an explicit `RETURN <name>` clause, and
a slot named `return` no longer parses at all.

> **The tree is RED until this phase lands.** `bindings/sqlite3/src/lib.mfb` was
> migrated to the `RETURN` surface ahead of the compiler (deliberately, at the
> user's direction — this is the next plan to implement). It currently fails:
>
> ```
> ./src/lib.mfb:143 error[1-102-0004 MFB_PARSE_UNEXPECTED_STATEMENT]
>   A native FUNC body may only contain SYMBOL, ABI, CONST, SUCCESS_ON, ERROR_ON, RESULT, or FREE clauses.
> ```
>
> That error names the exact dispatch list this phase extends. Every native test
> that imports sqlite3 fails until Phase 1 completes. **Do not "fix" this by
> reverting the binding** — the binding is the target state.

References (read first):

- `src/ast/items.rs:parse_abi_slot_name` (`:962-967`) — the special case to delete:
  ```rust
  pub(super) fn parse_abi_slot_name(&mut self) -> Option<String> {
      if self.match_keyword(Keyword::Return) {
          return Some("return".to_string());
      }
      self.consume_identifier("Expected an ABI slot name.")
  }
  ```
- `src/target/shared/code/link_thunk.rs:414` — `if slot.name == "return"` selects
  the OUT result buffer. The *name*, not `is_out`.
- `src/ir/verify/mod.rs:2707` — `if function.abi_return_name == "return" {
  result_markers += 1; }` — the second spelling of the same magic.
- `src/ir/verify/mod.rs:2710-2726` — `NATIVE_ABI_NO_RESULT` /
  `NATIVE_ABI_RESULT_MARKER`, the two rules this phase re-points (it adds none).
- `src/ast/items.rs:parse_link_function` (`:666`, dispatch `:705-779`) — where the
  `RETURN` clause is added; unknown clause → `MFB_PARSE_UNEXPECTED_STATEMENT`
  (`:772-776`).
- `src/ir/lower.rs:lower_link_expr` (`:421`) — takes `&native.abi.return_name` as
  **the single variable** an expression may reference. This is why `RESULT status
  = 100` works and `RESULT count = 5` cannot (§Open Decisions).
- `src/docs/spec/language/17_native-libraries.md` — 19 occurrences to rewrite.
- `bindings/sqlite3/src/lib.mfb` — 19 sites (§2).

## 1. Goal

- A LINK `FUNC` may carry `RETURN <name>`, where `<name>` is an ABI slot name or
  the ABI return's name. That slot/value becomes the wrapper's result.
- The name `return` loses all special meaning; `parse_abi_slot_name`'s keyword
  case is deleted, so `return` is simply not a legal slot name.
- `FREE <slot>` names the real slot (`FREE sql`, not `FREE return`).
- `bindings/sqlite3` is migrated and behaves **identically at runtime**.
- All 14 native test fixtures and the spec are migrated.
- A value-returning wrapper with no `RETURN` and no `RESULT` is
  `NATIVE_ABI_NO_RESULT`; one with both (or two `RETURN`s) is
  `NATIVE_ABI_RESULT_MARKER`. **No new rule codes.**

### Non-goals (explicit constraints)

- **No behavior change.** This is a rename plus a clause. Every migrated sqlite3
  thunk must be **byte-identical** to its pre-migration counterpart — that is the
  acceptance bar and the whole safety argument.
- **`RESULT <expr>` is not removed here.** Unifying it into `RETURN <expr>` is
  the one genuinely open design question (§Open Decisions); it needs real work in
  `lower_link_expr` and is separable.
- No struct/`CSTRUCT` awareness. plan-50-E layers struct slots on top.
- No `.mfp` format change *of its own* — plan-50-C already carries `result_slot`.

## 2. Current State

The result marker has **two spellings of the same magic name**, and the second is
five times commoner:

| form | meaning | mechanism | sqlite3 sites |
|---|---|---|---|
| `return OUT CPtr` (a slot) | an OUT slot is the result | `link_thunk.rs:414` `slot.name == "return"` | **3** |
| `AS return CInt64` (the C return) | the C return is the result | `verify:2707` `abi_return_name == "return"` | **15** |
| `FREE return` | frees the produced slot | names the slot | 1 |

Blast radius, counted: **19** sites in `bindings/sqlite3/src/lib.mfb`, **14**
files under `tests/syntax/native` + `tests/rt-behavior/native`, **19** occurrences
in `src/docs/spec/language/17_native-libraries.md`.

Why the current design has held up: today the compiler *forces* every OUT slot to
be named `return` (`NATIVE_ABI_UNBOUND_SLOT`, `verify:2687`), so "is OUT", "is
named `return`", and "is the result" are **indistinguishable** — no counterexample
can exist. The magic is invisible because it is unavoidable.

## 3. Design Overview

```
  every ABI slot has a name          ABI (path CString, db OUT CPtr) AS status CInt32
  the C return has a name                                             ^^^^^^
  RETURN <name> picks one            RETURN db
```

That is the whole design. `result_slot: Option<String>` on `IrLinkFunction`
(carried by plan-50-C §4.2b) replaces both `slot.name == "return"` and
`abi_return_name == "return"` as the single source of "which value is the result".
`abi_return_name` demotes to what its name says: the name you refer to the C
return by.

**Why the old rule must go, concretely.** plan-50-E introduces `INOUT` slots, and
`sf_open` is a slot that receives output but is *not* the result:

```
FUNC open(path AS String) AS RES Sndfile
  ABI (path CString, mode CInt32, info INOUT SfInfo) AS handle CPtr
  RETURN handle
```

`info` is filled by libsndfile; the result is the handle. Under the old rule this
is inexpressible — the direction can't disambiguate, and only one slot may be
named `return`. So the name has to become explicit. This phase is the price of
admission for struct slots, and it happens to be an improvement on its own.

**Where the risk concentrates:** the migration, not the feature. 19 sqlite3 sites
touched by hand, each a working native call. The defense is that this is a pure
rename with a mechanical proof: **byte-identical thunks**. If any migrated
function's generated code differs by one instruction, the migration is wrong.
`scripts/artifact-gate.sh` makes that check execution-free and cheap.

Rejected alternative: **lower `RETURN <name>` by renaming the slot to `return`
in the IR.** This would make the phase pure-frontend — no IR field, no format
bump, no backend change. Rejected: the IR would then carry a slot name the source
never wrote, every IR dump and diagnostic would reference a phantom `return`, and
it preserves the exact magic this phase exists to delete — just hidden one layer
down.

Rejected alternative: **keep `AS return <ctype>` and add `RETURN` only for slots.**
Rejected: 15 of 19 sites are the `AS return` form, so the magic name would survive
in the common case and the phase would add a clause without removing the wart.

## 4. Detailed Design

### 4.1 Surface

```
' before                                      ' after
ABI (stmt CPtr, col CInt32) AS return CInt64  ABI (stmt CPtr, col CInt32) AS value CInt64
                                              RETURN value

ABI (path CString, return OUT CPtr)           ABI (path CString, db OUT CPtr)
  AS status CInt32                              AS status CInt32
SUCCESS_ON status = 0                         RETURN db
                                              SUCCESS_ON status = 0

ABI (stmt CPtr) AS return CPtr                ABI (stmt CPtr) AS sql CPtr
FREE return                                   RETURN sql
  SYMBOL "sqlite3_free"                       FREE sql
  ABI (ptr CPtr) AS CVoid                       SYMBOL "sqlite3_free"
END FREE                                        ABI (ptr CPtr) AS CVoid
                                              END FREE
```

`RETURN` joins the clause dispatch in `parse_link_function` (`:705-779`). No
grammar ambiguity: a LINK `FUNC` body contains **clauses, not statements**, so
`Keyword::Return` here can only be this clause.

### 4.2 Validation

- `RETURN <name>` must name a declared ABI slot or the ABI return name; otherwise
  `NATIVE_ABI_UNBOUND_SLOT` (existing).
- `RETURN` on a `CONST`-pinned slot is `NATIVE_CONST_OUT`-adjacent nonsense —
  reject with `NATIVE_ABI_RESULT_MARKER`.
- Exactly one of `RETURN` / `RESULT` for a value-returning wrapper: zero →
  `NATIVE_ABI_NO_RESULT`; two → `NATIVE_ABI_RESULT_MARKER`.
- A `Nothing`-returning wrapper with a `RETURN` → `NATIVE_ABI_RESULT_MARKER`.
- Enforced on **both** the source and package paths, per plan-50-C §4.3's posture.

Both rules already exist (`2-203-0093`, `2-203-0096`); their messages need
rewording away from "`return`" toward "`RETURN`". **No new codes**, so
`01_rule-codes.md` needs no new rows — but re-read the reworded messages against
it, since `afdcceb6`'s `every_rule_is_documented_in_the_spec` guard matches on
code and name (not message), which stay the same.

### 4.3 Backend

`link_thunk.rs:414` becomes `if Some(&slot.name) == function.result_slot.as_ref()`.
The `emit_return_marshal` dispatch (`:789`) keys off `result_slot` naming the ABI
return rather than `abi_return_name == "return"`. Everything downstream — the
ctype-driven marshaling, `FREE`, `SUCCESS_ON` — is unchanged.

Because this is a rename, the emitted instruction sequence for every existing
wrapper is **identical**. That is the acceptance test.

## Compatibility / Format Impact

- **Changes (breaking, source):** every existing LINK binding must migrate. A slot
  named `return` no longer parses. This is a deliberate break of the binding
  surface; it is spent alongside plan-50-C's `.mfp` `4`→`5` bump, so the
  compatibility cost is paid once.
- **Unchanged:** all runtime behavior; every generated thunk's bytes; the rule
  codes; `SUCCESS_ON`/`ERROR_ON`/`CONST`/`FREE` semantics.
- No published-package concern (none exist); `bindings/sqlite3/sqlite3.mfp` is
  regenerated by plan-50-C anyway.

## Phases

One landable unit.

### Phase 1 — the clause, and the migration

- [ ] `src/ast/types.rs`: add `result_slot: Option<String>` to `LinkFunction`.
- [ ] `src/ast/items.rs`: add the `RETURN <name>` clause to `parse_link_function`'s
      dispatch (`:705-779`); **delete** the `Keyword::Return` case in
      `parse_abi_slot_name` (`:962-967`).
- [ ] `src/ir/lower.rs:link_functions` (`:292`): carry `result_slot` into
      `IrLinkFunction` (the IR field + encoding land in plan-50-C §4.2b).
- [ ] `src/syntaxcheck/mod.rs` + `src/ir/verify/mod.rs`: replace the two magic-name
      checks (`verify:2707`, and the slot scan) with `result_slot`; implement §4.2;
      reword the `NATIVE_ABI_NO_RESULT` / `NATIVE_ABI_RESULT_MARKER` messages.
- [ ] `src/target/shared/code/link_thunk.rs`: `:414` and the `:789` dispatch key off
      `result_slot` (§4.3).
- [x] ~~Migrate `bindings/sqlite3/src/lib.mfb`~~ — **already done** (uncommitted,
      ahead of the compiler). All 19 sites: `open`/`openV2` → `db`, `prepare` →
      `stmt`, `expandedSql` → `sql` (+ `FREE sql`), and the rest named for their
      value (`rowid`, `count`, `idx`, `text`, `name`, `kind`, `value`, `message`,
      `code`). 16 `RETURN` + 1 `RESULT` (`step`) = 17 value-returning wrappers; the
      8 `Nothing` wrappers correctly have neither. Verify against this when
      implementing rather than re-deriving names.
- [ ] Migrate the 14 fixtures under `tests/syntax/native/` + `tests/rt-behavior/native/`.
- [ ] Tests: `tests/syntax/native/native-abi-return-slot-invalid/` — a slot named
      `return` no longer parses; `RETURN` naming an unknown slot; `RETURN` +
      `RESULT` together; a value-returning wrapper with neither.
- [ ] Spec: rewrite all 19 occurrences in
      `src/docs/spec/language/17_native-libraries.md` — the `RETURN` clause, the
      dead magic name, `FREE <slot>`, and every worked example (the big `sqlite3`
      example at the topic's tail is most of the churn). Cite
      `[[src/ast/items.rs:parse_link_function]]`.

Acceptance: `bindings/sqlite3` rebuilds and every generated thunk is
**byte-identical** to pre-migration (`scripts/artifact-gate.sh`);
`tests/rt-behavior/native/native-link-sqlite-rt` passes **at runtime** unchanged;
a slot named `return` fails to parse; `scripts/test-accept.sh` green with churn
only in the migrated fixtures' own goldens.
Commit: —

## Validation Plan

- Tests: the invalid suite above, on both source and package paths.
- Runtime proof: `native-link-sqlite-rt` and `native-link-import-sqlite-rt` must
  **execute** identically — this phase touches every native call in the tree, so a
  passing compile proves nothing (`.ai/compiler.md`).
- **The byte-identity gate is the real test.** A pure rename cannot change one
  instruction; any diff is a bug in the migration, not an improvement.

  Note the reference point is *not* the working tree — the binding source was
  migrated ahead of the compiler, so "before" no longer exists on disk. Build the
  **pre-migration** source with the **pre-H** compiler to get the baseline:
  ```
  git show HEAD:bindings/sqlite3/src/lib.mfb > /tmp/sqlite3-pre.mfb
  ```
  and diff the emitted thunks against the post-H build of the migrated source.
  `scripts/artifact-gate.sh` for the rest of the tree (which must not move at all).
- Doc sync: `src/docs/spec/language/17_native-libraries.md` (19 spots); then
  `cargo build`, `cargo test --bin mfb spec`, no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Unify `RESULT <expr>` into `RETURN <expr>` and delete `RESULT`?** Recommend
  **yes, but not in this phase.** Two near-synonymous clauses both answering "what
  is the result?" is a documentation hazard:
  ```
  RETURN info            ' the result IS this slot
  RESULT status = 100    ' the result is computed from the status
  ```
  Unified, `RETURN count` is just the degenerate case where the expression is a
  bare slot reference, and `step` reads `RETURN status = 100`. The blocker is real
  though: `lower_link_expr` (`src/ir/lower.rs:421`) is handed **one** variable —
  `&native.abi.return_name` — so `status` resolves only because it *is* the ABI
  return name. Unifying means giving the expression a symbol table (slot name →
  stack offset) in both `lower_link_expr` and `emit_link_expr`
  (`link_thunk.rs:531`). That is the only non-mechanical work in the whole idea,
  and it would break this phase's byte-identity acceptance gate. Do it as a
  follow-on where its own risk is visible.
- **Land H before plan-50-C with its own `4`→`5` bump?** Recommend **no** — depend
  on C and let it carry `result_slot`, for one bump instead of two. The cost is
  that a LINK-surface cleanup is gated behind the CSTRUCT stack for a bookkeeping
  reason. If that ordering grates, H can bump independently (C then bumps `5`→`6`);
  bumps are cheap here (no published packages, and `sqlite3.mfp` is regenerated
  either way). **User's call.**
- **Slot naming in the sqlite3 migration.** The 15 `AS return` sites each need a
  name. Recommend naming for the *value* (`value`, `text`, `rowid`, `count`), not
  the mechanism (`ret`, `out`) — these names appear in `SUCCESS_ON`/`RESULT`
  expressions and are the binding's readable surface.

## Summary

A rename with a clause bolted on, whose entire risk is that it touches all 19
native call sites in the only real binding in the tree. The defense is unusually
strong for a refactor: a pure rename must produce **byte-identical** thunks, so
`artifact-gate.sh` mechanically proves the migration correct.

The one thing deliberately left undone is folding `RESULT` into `RETURN` — the
right end state, but it needs a real symbol table in the link-expression lowering
and would destroy this phase's byte-identity proof.
