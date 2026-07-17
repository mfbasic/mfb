# plan-50-I: link-expression symbols — `Var` learns its name

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-C (the `Var` payload is a wire change and rides C's bump)
Lands **before** plan-50-H, which needs it. (Letters are identifiers, not an
order — see plan-50-D.)

Gives `SUCCESS_ON` / `RESULT` expressions a real symbol table. Today
`IrLinkExpr::Var` is a **unit variant** — one nameless variable, always the native
return — and any identifier at all lowers to it. This phase makes `Var` carry a
slot name, resolves it against the function's real slots, and **rejects a name
that is not one**.

No surface change: no new clause, no new keyword. `SUCCESS_ON status = 0` parses
and compiles exactly as before and emits **byte-identical** code. What changes is
that `SUCCESS_ON count = 5` becomes expressible (it currently cannot mean that),
and `SUCCESS_ON typo = 0` becomes an error instead of silently meaning the status.

This is the primitive plan-50-H needs to unify `RESULT <expr>` into
`RETURN <expr>`, and it fixes a real bug on the way.

The single behavioral outcome: a `SUCCESS_ON`/`RESULT` expression naming a slot
that does not exist is **rejected at compile time**; one naming a real slot reads
**that slot's** value; and every existing binding emits byte-identical code.

References (read first):

- `src/ir/link.rs:IrLinkExpr` (`:70`) — the enum, and the doc comment that states
  the limit outright:
  ```rust
  /// The native return variable (the `AS <name> <ctype>` value).
  Var,
  ```
  `Var` has **no payload**.
- `src/ir/lower.rs:lower_link_expr` (`:421`) — the bug:
  ```rust
  Expression::Identifier(name) if name == var  => IrLinkExpr::Var,
  Expression::Identifier(name) if name == "NOTHING" => IrLinkExpr::Int(0),
  Expression::Identifier(_) => IrLinkExpr::Var,     // <-- ANY identifier
  ```
  The third arm makes every unknown identifier the native return.
- `src/target/shared/code/link_thunk.rs:emit_link_expr` (`:1005`) — takes
  `status_off: usize`, and its `Var` arm (`:1022`) is
  `abi::load_u64(&dst, abi::stack_pointer(), status_off)`. One variable, one
  offset.
- `src/ir/binary.rs:encode_link_expr` (`:312`) — `IrLinkExpr::Var => put_u8(out, 0)`,
  tag 0 with no payload; decoder mirror at `:439`. Recursion depth-capped at
  `MAX_DECODE_DEPTH = 256` (`:87`).
- `src/target/shared/code/link_thunk.rs:339-347` — the frame offsets this phase
  must expose per slot: `cslot_base = param_base + n_params*8` (`:343`),
  `out_base = cslot_base + m_slots*8` (`:344`), `STATUS_OFF = 8` (`:339`).
- `src/ir/lower.rs:eval_link_const` (`:387`) — the sibling instance of this same
  anti-pattern (`_ => 0`), **out of scope here** but noted in §Open Decisions.

## 1. Goal

- `IrLinkExpr::Var(String)` names the slot it reads.
- An identifier in a `SUCCESS_ON`/`RESULT` expression resolves against the
  function's ABI slot names **and** its ABI return name; anything else is
  `NATIVE_ABI_UNBOUND_SLOT` (existing rule, no new code) instead of silently
  becoming the native return.
- `emit_link_expr` resolves each `Var(name)` to that slot's frame offset, so an
  expression may read an `OUT` slot, a `CONST`-pinned slot, or the C return.
- **Every existing binding emits byte-identical code.** Today the only
  identifier any in-tree expression uses *is* the ABI return name, so correct
  resolution reproduces today's behavior exactly.
- `NOTHING` keeps its meaning (`Int(0)`); it is not a slot name.

### Non-goals (explicit constraints)

- **No surface change.** No new clause, no new keyword, no migration. `RESULT`
  still exists and `SUCCESS_ON` is untouched syntactically. plan-50-H does the
  surface work.
- No fix for `eval_link_const`'s `_ => 0` (`src/ir/lower.rs:398`). Same
  anti-pattern, different function, and `CONST` pins are a different grammar.
  §Open Decisions.
- No struct/`CSTRUCT` awareness — a `Var` naming a struct slot is plan-50-E's
  problem; this phase resolves scalar slots and the C return.
- No new rule code: `NATIVE_ABI_UNBOUND_SLOT` (`2-203-0094`) already means
  exactly "this name is not a slot".

## 2. Current State

The link-expression language is `Var | Int | Compare | And | Or | Not`
(`src/ir/link.rs:70`), and `Var` is a **unit** variant meaning "the native return".
`lower_link_expr(expr, var)` takes the *one* legal name —
`&native.abi.return_name` (`src/ir/lower.rs:341`/`:345`) — and maps it to `Var`.

The bug is the fallthrough (`src/ir/lower.rs:427`):

```rust
Expression::Identifier(_) => IrLinkExpr::Var,
```

Every other identifier also becomes `Var`. So today:

```
ABI (path CString, count OUT CInt32) AS status CInt32
SUCCESS_ON typo = 0        ' silently means: status = 0
SUCCESS_ON count = 5       ' silently means: status = 5   <-- cannot mean what it says
```

Neither is diagnosed. `count` is a real slot and the expression looks like it
reads it; it reads the status instead. This is the same "silently default rather
than diagnose" shape as `eval_link_const`'s `_ => 0` (`:398`, any unsupported
const expression pins **0**) and the unvalidated slot ctype (plan-50-A) — three
instances in one subsystem.

Why it has never bitten: `SUCCESS_ON status = 0` is the only shape any in-tree
binding uses, and `status` *is* the return name, so the correct arm fires. The bug
is latent, not dormant-by-luck — the moment a binding references a real OUT slot
it gets a wrong answer with no diagnostic.

On the backend, `emit_link_expr` (`:1005`) hard-codes one offset (`status_off`) and
its `Var` arm loads from it unconditionally.

## 3. Design Overview

```
  lower_link_expr(expr, &slot_names)      -> Var(name)      | NATIVE_ABI_UNBOUND_SLOT
  encode_link_expr: tag 0 + put_str(name)                    (rides plan-50-C's bump)
  emit_link_expr(expr, &slot_offsets)     -> load_u64 [sp + offsets[name]]
```

Three mechanical edits and one table. The table is the interesting part: it maps
each name a `Var` may hold to a frame offset, and it is built where the frame is
already computed (`lower_link_thunk`, `link_thunk.rs:333`):

| name | offset | note |
|---|---|---|
| the ABI return name (`status`) | `STATUS_OFF` (`:339`) | the sign-extended C return (`:512-519`) |
| an `OUT` slot | its `out_base + seq*8` (`:407`) | the produced value |
| an ordinary/`CONST` slot | its `cslot_base + idx*8` (`:405`) | the marshaled C argument |

Every one of those offsets already exists and is already written before the
expression is evaluated (`SUCCESS_ON` is emitted at `:529`, after the call and
after `STATUS_OFF` is stored at `:519`). So this phase adds no new frame slots and
no new stores — it only lets the expression address what is already there.

**Where the risk concentrates:** byte-identity. This touches the lowering of every
`SUCCESS_ON` in the tree, and its safety argument is that correct name resolution
reproduces the current (accidentally correct) behavior. The proof is
`scripts/artifact-gate.sh`: not one instruction may move.

Rejected alternative: **keep `Var` nameless and give `emit_link_expr` a second
"which slot" parameter.** Rejected: it does not fix the lowering bug (the name is
already lost by then), and the IR would still be unable to represent "read slot
`count`".

Rejected alternative: **resolve names in the backend instead of at lowering.**
Rejected: the package path (`ir::verify`) must be able to reject a bad name, and it
never runs the backend. Resolution belongs where the other LINK checks are.

## 4. Detailed Design

### 4.1 The IR

```rust
pub(crate) enum IrLinkExpr {
    /// The value of a named ABI slot, or of the ABI return (`AS <name> <ctype>`).
    Var(String),
    Int(i64),
    …
}
```

### 4.2 Lowering

`lower_link_expr(expr, var: &str)` becomes `lower_link_expr(expr, slots: &SlotSet)`
where `SlotSet` holds every ABI slot name plus the ABI return name:

```rust
Expression::Identifier(name) if name == "NOTHING" => IrLinkExpr::Int(0),
Expression::Identifier(name) if slots.contains(name) => IrLinkExpr::Var(name.clone()),
Expression::Identifier(name) => /* NATIVE_ABI_UNBOUND_SLOT */,
```

Keep the `NOTHING` arm **first**: it is a literal, not a slot, and a binding could
otherwise declare a slot named `NOTHING` and change the meaning of every `NOTHING`
in its expressions.

Lowering cannot emit diagnostics directly, so follow the established split: lower
an unknown name to a marker (or leave the checking to the two checkers) and let
`src/syntaxcheck/mod.rs` (slot-level span) and `src/ir/verify/mod.rs`
(function-level span) raise `NATIVE_ABI_UNBOUND_SLOT` — the same both-paths posture
as every other LINK rule (plan-50-C §4.3). **The package path must reject it too**:
a crafted `.mfp` can carry any `Var(name)` string.

### 4.3 Wire format

Tag 0 gains a payload:

```rust
IrLinkExpr::Var(name) => { put_u8(out, 0); put_str(out, name); }
```

This is a hard break for the link-expr encoding, and it **rides plan-50-C's
`4`→`5` bump** — do not bump again. The `MAX_DECODE_DEPTH = 256` guard (`:87`) is
unchanged; a `Var`'s string is bounded by `put_str`'s existing length handling.

### 4.4 Backend

`emit_link_expr(expr, status_off, …)` becomes
`emit_link_expr(expr, offsets: &HashMap<&str, usize>, …)`, and the `Var` arm:

```rust
IrLinkExpr::Var(name) => {
    let off = offsets.get(name.as_str()).ok_or_else(|| /* verified upstream */)?;
    instructions.push(abi::load_u64(&dst, abi::stack_pointer(), *off));
}
```

The map is built in `lower_link_thunk` from the frame constants (§3). Both call
sites — the `SUCCESS_ON` gate (`:531`) and the `RESULT` mapping — pass it.

Note the ABI return name maps to `STATUS_OFF`, **not** `CRET_OFF`: `STATUS_OFF`
holds the sign-extended value (`:512-519`), which is what `SUCCESS_ON status = -1`
must compare against. Getting this backwards would break `ERROR_ON status = -1`
POSIX-style gates on any negative return — and byte-identity would catch it.

## Compatibility / Format Impact

- **Changes:** the link-expr `Var` wire tag gains a string payload (rides
  plan-50-C's bump). A `SUCCESS_ON`/`RESULT` naming a non-slot now fails to
  compile — such a binding was already silently miscompiling.
- **Unchanged:** all syntax; all runtime behavior for every correct binding; every
  emitted instruction; `MAX_DECODE_DEPTH`; the `RESULT` clause; `NOTHING`.

## Phases

One landable unit.

### Phase 1 — name the variable, resolve it, reject the rest

- [ ] `src/ir/link.rs`: `IrLinkExpr::Var` → `Var(String)`; update the doc comment
      (it currently asserts the single-variable limit).
- [ ] `src/ir/lower.rs:lower_link_expr` (`:421`): take the slot set; resolve
      identifiers per §4.2; **delete the `Identifier(_) => Var` fallthrough**.
      Update both call sites (`:341`, `:345`).
- [ ] `src/ir/binary.rs`: `encode_link_expr:312` / `decode_link_expr:439` — tag 0
      carries a string (§4.3). No version bump (plan-50-C owns it).
- [ ] `src/syntaxcheck/mod.rs` + `src/ir/verify/mod.rs`: raise
      `NATIVE_ABI_UNBOUND_SLOT` for a `Var` naming no slot, on both paths.
- [ ] `src/target/shared/code/link_thunk.rs`: build the name→offset map in
      `lower_link_thunk` (§3); thread it through `emit_link_expr:1005` and both
      call sites; `Var` loads from the resolved offset (§4.4).
- [ ] Tests: `tests/syntax/native/native-link-expr-unknown-name-invalid/` — a
      `SUCCESS_ON` naming a non-slot is rejected (this is the bug fix; assert the
      diagnostic, not just failure).
- [ ] Tests: `src/ir/verify/tests.rs` — the package path rejects a decoded
      `Var("nope")`.
- [ ] Tests: round-trip byte-identity for a `Var`-bearing expression
      (`src/ir/coverage_tests.rs:505-513`).
- [ ] Tests (**the point of the phase**): a runtime case where `SUCCESS_ON` reads
      an **OUT slot** rather than the status — impossible to express today —
      proving `Var` resolves per-name.
- [ ] Spec: `src/docs/spec/language/17_native-libraries.md` — `SUCCESS_ON`/`RESULT`
      expressions range over **any** ABI slot name (not just the native return),
      and an unknown name is rejected. Correct the current text, which says
      `<expr>` is "any Boolean expression over slot names" — **true as documented,
      false as implemented**; that gap is this bug. Cite
      `[[src/ir/lower.rs:lower_link_expr]]`.

Acceptance: `SUCCESS_ON typo = 0` is rejected with `NATIVE_ABI_UNBOUND_SLOT` on
both paths; a `SUCCESS_ON` over an `OUT` slot gates on that slot's value at
runtime; `bindings/sqlite3`'s thunks are **byte-identical**
(`scripts/artifact-gate.sh`) and `native-link-sqlite-rt` passes unchanged;
`scripts/test-accept.sh` green with zero golden churn beyond the new tests.
Commit: `cf66dcfb`

**Landed note.** Byte-identical, as predicted: artifact-gate 946 tests / 1121
goldens / 0 diffs. The `Var(String)` wire payload rode plan-50-C's bump, so this
phase was pure semantics.

## Validation Plan

- Tests: as above. The OUT-slot `SUCCESS_ON` test is the one that proves the
  feature; the unknown-name test is the one that proves the bug is fixed.
- Runtime proof: `native-link-sqlite-rt` must execute unchanged (every
  `SUCCESS_ON status = 0` in the tree flows through the rewritten path), plus the
  new OUT-slot gate case.
- **Byte-identity is the safety net.** Every in-tree expression resolves to the
  return name today; after this phase it resolves by name to the same offset.
  `scripts/artifact-gate.sh` must show zero movement.
- Doc sync: `17_native-libraries.md`; `cargo build`, `cargo test --bin mfb spec`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Fix `eval_link_const`'s `_ => 0` (`src/ir/lower.rs:398`) here too?** Same
  anti-pattern: any unsupported `CONST` expression silently pins **0**. Recommend
  **no — file it separately** (`/write-bug`); it is a different function and a
  different grammar (const-fold, not slot resolution), and AGENTS.md forbids
  bundling unrelated changes. But note plan-50-E's `SIZEOF <CStruct>` pin lands in
  exactly that function, so E must handle `SIZEOF` explicitly **and** should turn
  the catch-all into an error while it is there. Sequence accordingly.
- **Should `Var` resolve `CONST`-pinned slots?** Recommend **yes** — a pin has a
  known value at a known offset (`:433-436`), and `SUCCESS_ON flags = 0` over a
  pinned slot is meaningful (if useless). Excluding them adds a rule with no
  benefit. Alternative: reject, on the grounds that comparing against a constant
  you wrote is dead code — rejected as paternalistic.

## Summary

A three-line bug (`Identifier(_) => Var`) that silently turns any name into the
status, sitting under a documented feature ("any Boolean expression over slot
names") that has never actually worked. Fixing it requires `Var` to carry its
name, which is exactly the primitive plan-50-H needs to fold `RESULT` into
`RETURN`.

All the risk is byte-identity: this rewrites the lowering of every `SUCCESS_ON`
in the tree, and the argument that it is safe is that today's single legal name
resolves to today's single offset. `artifact-gate.sh` proves it mechanically.
