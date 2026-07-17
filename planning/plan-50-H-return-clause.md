# plan-50-H: the `RETURN <expr>` clause — deleting the magic slot name and `RESULT`

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-I (`IrLinkExpr::Var(String)` + the slot symbol table),
plan-50-C (the wire bump I rides on)

Replaces the magic ABI slot name `return` with an explicit clause, and folds
`RESULT <expr>` into it. Today a wrapper's result is identified by a slot
literally *named* `return`, or by a separate `RESULT` clause; after this, every
slot and the C return carry ordinary names and one clause — `RETURN <expr>` —
says what the result is.

`RETURN db` is simply the degenerate case where the expression is a bare slot
reference; `RETURN status = 100` is the computed case that used to be `RESULT`.
One clause, one concept.

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

- A LINK `FUNC` carries `RETURN <expr>`, an expression over its ABI slot names and
  its ABI return name (plan-50-I's symbol table). A bare `RETURN db` names a slot;
  `RETURN status = 100` computes.
- **`RESULT <expr>` is deleted.** Every `RESULT` becomes a `RETURN`.
- The name `return` loses all special meaning; `parse_abi_slot_name`'s keyword
  case is deleted, so `return` is simply not a legal slot name.
- `FREE <slot>` names the real slot (`FREE sql`, not `FREE return`).
- `bindings/sqlite3` is migrated and behaves **identically at runtime**.
- All 14 native test fixtures and the spec are migrated.
- A value-returning wrapper with no `RETURN` is `NATIVE_ABI_NO_RESULT`; two
  `RETURN`s is `NATIVE_ABI_RESULT_MARKER`. **No new rule codes.**
- **No new IR field.** The result rides the *existing*
  `IrLinkFunction.result: Option<IrLinkExpr>` (`src/ir/link.rs:43`), which is
  already encoded (`src/ir/binary.rs:291`). `result_slot` is not needed and is not
  added (§3).

### Non-goals (explicit constraints)

- **No behavior change.** This is a rename plus a clause merge. Every migrated
  sqlite3 thunk must be **byte-identical** to its pre-migration counterpart — that
  is the acceptance bar and the whole safety argument.
- No struct/`CSTRUCT` awareness. plan-50-E layers struct slots on top — a struct
  result is just `RETURN <bare-slot>` with a different marshaling arm.
- No `.mfp` format change *of its own* — plan-50-I's `Var(String)` payload and
  plan-50-C's bump cover it.
- No change to `SUCCESS_ON`/`ERROR_ON`, whose expression grammar this reuses
  verbatim.

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
  RETURN <expr> says what the        RETURN db              (a bare slot ref)
  result is                          RETURN status = 100    (computed — was RESULT)
```

That is the whole design, and the pleasing part is that **the IR already has the
field**. `IrLinkFunction.result: Option<IrLinkExpr>` (`src/ir/link.rs:43`) is the
old `RESULT` mapping, already encoded as tag-13 of the positional record
(`src/ir/binary.rs:291`). Once plan-50-I gives `Var` a name, `RETURN db` is just
`result = Var("db")` and `RETURN status = 100` is
`result = Compare(Var("status"), 100)` — one field expresses both.

So this phase **adds no IR field and no wire field**. Both magic checks —
`slot.name == "return"` (`link_thunk.rs:414`) and `abi_return_name == "return"`
(`verify:2707`) — are deleted outright, replaced by "is there a `result`
expression, and what does its `Var` name?". `abi_return_name` demotes to what its
name says: the name you refer to the C return by.

**`result_slot` is therefore not needed**, and plan-50-C §4.2b no longer carries
it. The unification *removed* a planned format field rather than adding one, which
is the strongest argument for it: the design got smaller.

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

Rejected alternative: **keep `RESULT <expr>` alongside `RETURN <name>`.** This was
the original plan. Rejected: two near-synonymous clauses both answering "what is
the result?" is a documentation hazard, and keeping them apart *costs* a field —
`RETURN <name>` would need a new `result_slot` on the IR and the wire, while
`RETURN <expr>` reuses the `result` field that already exists. Merging is both
smaller and simpler.

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

SUCCESS_ON status = 100 OR status = 101       SUCCESS_ON status = 100 OR status = 101
RESULT status = 100                           RETURN status = 100
```

`RETURN` joins the clause dispatch in `parse_link_function` (`:705-779`) and
**`RESULT` leaves it**. No grammar ambiguity: a LINK `FUNC` body contains
**clauses, not statements**, so `Keyword::Return` here can only be this clause.
The expression after it is parsed by the same parser `RESULT` used, so the
grammar is unchanged — only the keyword moves.

### 4.2 Validation

- Every name inside `RETURN <expr>` must resolve to a slot or the ABI return
  name — that is plan-50-I's job (`NATIVE_ABI_UNBOUND_SLOT`), inherited for free.
- Exactly one `RETURN` for a value-returning wrapper: zero →
  `NATIVE_ABI_NO_RESULT`; two → `NATIVE_ABI_RESULT_MARKER`.
- A `Nothing`-returning wrapper with a `RETURN` → `NATIVE_ABI_RESULT_MARKER`.
- Enforced on **both** the source and package paths, per plan-50-C §4.3's posture.

Both rules already exist (`2-203-0093`, `2-203-0096`); their messages need
rewording away from "`return`"/"`RESULT`" toward "`RETURN`". **No new codes**, so
`01_rule-codes.md` needs no new rows — but re-read the reworded messages against
it, since `afdcceb6`'s `every_rule_is_documented_in_the_spec` guard matches on
code and name (not message), which stay the same.

### 4.3 Backend

Both magic checks die:

- `link_thunk.rs:414` (`if slot.name == "return"`) is deleted. The result-producing
  slot is instead identified by the `result` expression being a **bare
  `Var(name)`** naming that slot — then its existing `result_out_off` /
  `result_out_ctype` path (`:414-417`, `:546-...`) applies unchanged.
- `verify:2707` (`abi_return_name == "return"`) is deleted; a wrapper produces a
  value iff `result.is_some()`.

The backend therefore dispatches on the **shape** of `result`:

| `result` | path | example |
|---|---|---|
| `Var(n)`, `n` is an `OUT` slot | the OUT-buffer result path (`:546`) | `RETURN db` |
| `Var(n)`, `n` is the ABI return name | `emit_return_marshal` (`:789`), ctype-driven | `RETURN value` |
| anything else (a computed expr) | `emit_link_expr` → `RESULT_VALUE_REGISTER` | `RETURN status = 100` |

Each of those three paths **already exists** and is reached today by a different
trigger; this phase only changes what selects them. That is why every existing
wrapper's instruction sequence is **identical** — the acceptance test.

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

- [ ] `src/ast/items.rs`: add the `RETURN <expr>` clause to `parse_link_function`'s
      dispatch (`:705-779`), reusing `RESULT`'s expression parser; **delete** the
      `RESULT` clause; **delete** the `Keyword::Return` case in
      `parse_abi_slot_name` (`:962-967`).
- [ ] `src/ast/types.rs` / `src/ir/lower.rs:link_functions` (`:292`): `RETURN`
      populates the **existing** `result` field. **No new IR field, no new wire
      field** — `IrLinkFunction.result` (`src/ir/link.rs:43`) and its encoding
      (`src/ir/binary.rs:291`) already exist.
- [ ] `src/syntaxcheck/mod.rs` + `src/ir/verify/mod.rs`: delete the two magic-name
      checks (`verify:2707` and the `slot.name == "return"` scan); a wrapper
      produces a value iff `result.is_some()`; implement §4.2; reword the
      `NATIVE_ABI_NO_RESULT` / `NATIVE_ABI_RESULT_MARKER` messages.
- [ ] `src/target/shared/code/link_thunk.rs`: replace `:414`'s magic-name test and
      the `:789` dispatch with the three-way shape dispatch on `result` (§4.3).
- [x] ~~Migrate `bindings/sqlite3/src/lib.mfb`~~ — **already done** (uncommitted,
      ahead of the compiler). All 19 sites: `open`/`openV2` → `db`, `prepare` →
      `stmt`, `expandedSql` → `sql` (+ `FREE sql`), and the rest named for their
      value (`rowid`, `count`, `idx`, `text`, `name`, `kind`, `value`, `message`,
      `code`). 16 `RETURN` + 1 `RESULT` (`step`) = 17 value-returning wrappers; the
      8 `Nothing` wrappers correctly have neither. Verify against this when
      implementing rather than re-deriving names.
- [ ] **One migration site remains:** `step`'s `RESULT status = 100` →
      `RETURN status = 100` (`bindings/sqlite3/src/lib.mfb`), plus the two comments
      above it that describe `RESULT` as a distinct clause.
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
Commit: `56d7df32`

**Landed notes.**
1. **A third magic-name site the plan missed**: `link_returns_cstring`
   (`src/target/shared/code/mod.rs`) decides whether to *emit*
   `_mfb_rt_validate_utf8`, and tested `abi_return_name == "return"`. It must
   agree exactly with the thunk's `needs_encoding`, or the thunk references a
   helper nobody emitted — which is how it surfaced (a link error, not a test
   failure).
2. **Byte-identity, measured**: the migrated executable differs from its
   pre-migration build in exactly 40 words — 16 `MOVZ` immediates shifted by
   **+5** (ErrorLoc lines; each wrapper gained a `RETURN` line) and 24 inside
   `LC_CODE_SIGNATURE`. Zero instruction or logic changes. Literal byte-identity
   was never achievable because the migration adds source lines; this is the
   honest form of that gate.
3. **Restored 9 crafted security fixtures clobbered by plan-50-C**: the `.mfp`
   regeneration rebuilt them from source, destroying deliberate corruption so
   packages meant to be rejected built and ran. They regenerate via their own
   `tools/security-package-sources/*/generate.py`; `mfp_craft.py` now tracks
   `BINARY_REPR_VERSION` by constant instead of a literal `4`.

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

- ~~**Unify `RESULT` into `RETURN`?**~~ **ACCEPTED** (2026-07-16) and folded into
  this phase. The prerequisite — a slot symbol table in `lower_link_expr` /
  `emit_link_expr` — is split out as plan-50-I so its own risk (and the latent
  `Identifier(_) => Var` bug it fixes) stays visible. Byte-identity survives: every
  in-tree expression resolves to the same offset it uses today.
- ~~**Land H before plan-50-C with its own bump?**~~ **Moot.** The unification
  reuses the existing `result` field, so `result_slot` is never added and H needs
  no format change of its own. H depends on I, which rides C's bump for the
  `Var(String)` payload.
- **Slot naming in the sqlite3 migration** — settled, already applied: named for
  the *value* (`value`, `text`, `rowid`, `count`, `message`), not the mechanism
  (`ret`, `out`). These names appear in `SUCCESS_ON`/`RETURN` expressions and are
  the binding's readable surface.

## Summary

A rename with a clause bolted on, whose entire risk is that it touches all 19
native call sites in the only real binding in the tree. The defense is unusually
strong for a refactor: a pure rename must produce **byte-identical** thunks, so
`artifact-gate.sh` mechanically proves the migration correct.

The one thing deliberately left undone is folding `RESULT` into `RETURN` — the
right end state, but it needs a real symbol table in the link-expression lowering
and would destroy this phase's byte-identity proof.
