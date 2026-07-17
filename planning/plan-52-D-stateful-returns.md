# plan-52-D: stateful returns — carry STATE on the return type, and close the laundering

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-52-A (the model + pending fixtures). Independent of C, but see §3 — the
binding rule here **must not** land separately from the return rule.

Makes `FUNC openFile(path) AS RES SfFile STATE FileInfo` work. Today it cannot: `ir::lower`
never appends `return_state_type` to the return type string, so a stateful `RETURN` is
rejected as `TYPE_RETURN_MISMATCH` — *"RETURN value has type File STATE Cursor, expected
File."* This is what blocks `bindings/libsnd`'s `openFile` from handing back an `SNDFILE*`
carrying its `SF_INFO`.

The same omission has an opposite second symptom: the STATE verify rules pattern-match
`" STATE "` in a type string, so a return type — which never contains it — **escapes them
entirely**. One missing append both rejects the legal program and accepts two illegal ones.

The single outcome: **a callee-populated STATE survives a `RETURN` into the caller's
binding (`pos=42 len=7`, not `pos=0 len=0`), while a bare return provably carries no
state.**

References:

- `plan-52-A` §3 — the model table (the return and binding rows); §Phase 3 — the `.mfp`
  audit that **gates this sub-plan**. **Read first.**
- `planning/res.md` §2 — the verified behavior table; §6 — why the `stateful → bare`
  question is answered "no" here despite an earlier recommendation of "yes".
- `bindings/libsnd/src/lib.mfb` — the motivating consumer.

## 1. Goal

- `FUNC openTagged(p) AS RES File STATE Cursor` with `RETURN <a RES f AS File STATE Cursor>`
  compiles, and the caller's `RES h AS File STATE Cursor = openTagged(p)` observes the
  callee's values (plan-52-A row 7).
- `openTagged(p).state` resolves to `Cursor` **from the call expression**, and
  `RES h = openTagged(p)` (unannotated) infers `File STATE Cursor`.
- A bare return (`AS RES File`) returning a stateful value is **rejected** (row 6 — earned,
  not accidental).
- Union-STATE and non-defaultable-STATE on a **return** are rejected with the same codes as
  their binding twins (row 8).
- A bare **binding** of a stateful value is rejected (row 9) — **in this same change**.
- `bindings/libsnd`'s `openFile` shape compiles.

### Non-goals (explicit constraints)

- **The `" STATE "` type-string encoding.** Restoring the missing append within the
  existing convention; replacing it with a structured field is a separate plan.
- **`emit_resource_state_init`'s null-check.** "A moved/returned resource that already
  carries a state keeps it (the slot is non-null)" is the mechanism this sub-plan
  *depends on*. Do not touch it.
- **`FILE_OFFSET_STATE` / record layout.** The runtime side is already correct and stays
  byte-identical.
- **LINK native funcs keep having no STATE clause.** `parse_link_function`
  (`src/ast/items.rs:748`, return parsed at `:771-776`) deliberately omits
  `parse_optional_state()`, and the grammar states "The native return has no STATE clause."
  A binding package wraps its native func in an ordinary `EXPORT FUNC` that carries the
  STATE. Do not add STATE to the LINK grammar here.
- **Tempting wrong fix #1: making `expression_compatible` strip `" STATE "` before
  comparing.** Makes row 7 pass — the runtime would even work — while leaving the return
  type string state-less. Call-expression `.state` typing stays broken, row 8 stays broken,
  and row 9's laundering becomes reachable through returns. The type string must carry the
  STATE; the compare must not be loosened.
- **Tempting wrong fix #2: deleting `return_state_type` and the grammar clause**
  ("nobody uses it"). The grammar documents it, `syntaxcheck` validates it, `bindings/libsnd`
  needs it. Unfinished, not unwanted.

## 2. Current State

Two of three type-string sites append the STATE; the return site does not:

- `src/ir/lower.rs:739-742` — **params**, appends, with a comment explaining why.
- `src/ir/lower.rs:974-977` — **bindings**, appends.
- `src/ir/lower.rs:724-730` — **returns**, takes `function.return_type` raw:

```rust
let returns = match function.kind {
    FunctionKind::Func => function
        .return_type
        .clone()
        .unwrap_or_else(|| "Unknown".to_string()),
    FunctionKind::Sub => "Nothing".to_string(),
};
```

`function.return_state_type` is read **nowhere** in `ir/` or `target/`. Only
`src/syntaxcheck/mod.rs:2041` (→ `check_resource_declaration`,
`src/syntaxcheck/checking.rs:74-87` — an existence check and nothing more) and
`src/ast/serialize.rs:710` (one-way AST→JSON). Everything else constructs it as `None`
(`escape.rs:411`, `monomorph/helpers.rs:521`, `resolver/mod.rs:599`, `testing/desugar.rs`).

Consequences:

- `returns` becomes `current_return_type` (`:745-746`), which `check_return_type`
  (`src/ir/verify/mod.rs:3895-3909`) compares against the returned value's inferred type.
  The value's string carries the STATE; the expected one does not → `TYPE_RETURN_MISMATCH`.
- `src/ir/verify/mod.rs:825-845` gates `TYPE_UNION_STATE_FORBIDDEN` / `TYPE_STATE_INVALID`
  on `type_.find(" STATE ")` over a **binding's** string — unreachable from a return.
- `src/ir/lower.rs:1963-1970` (the `returns` map feeding `expression_type` for calls) has
  the same omission — which is why `openTagged(p).state` cannot resolve.

**Verified:** the return's STATE annotation is fully **inert** — annotating
`AS RES File STATE Cursor` while returning a *stateless* value compiles and runs. It does
not even change the expected type.

**The runtime plumbing already exists and is correct.** `emit_resource_state_init`
(`src/target/shared/code/builder_value_semantics.rs:10-36`) null-checks the state slot and
**skips init when it is already populated** — "a moved/returned resource that already
carries a state keeps it". That comment describes exactly the scenario this bug makes
unreachable. Only the type-string append is missing.

## 3. Design Overview

Mirror `src/ir/lower.rs:739-742` at both return sites: when `return_resource` is set and
`return_state_type` is `Some(state)`, lower the return type as
`format!("{return_type} STATE {state}")`.

That is four lines. **The engineering risk is not the append** — it is what the append
exposes.

**The return rule falls out for free.** `check_return_type` already does the comparison;
the append makes it correct in both directions:

- row 7: expected `File STATE Cursor` == actual → **accept**.
- row 6: expected `File` ≠ actual `File STATE Cursor` → **reject**, and the bare return's
  "no state" promise becomes enforced rather than accidental.

That promise is load-bearing: `RES h AS File STATE Cursor = test3()` allocates a fresh
Cursor *only because the slot is null*. A return that secretly carried a `FileInfo` would
alias it.

**The laundering, and why the binding rule ships here.** A bare binding **erases** the
STATE from the type string. Once returns carry STATE, that erasure defeats the return
check:

```basic
FUNC launder() AS RES SfFile             ' promises "no state"
  RES tmp AS SfFile = openStateful()     ' bare bind of a stateful value
  RETURN tmp                             ' expected SfFile, actual SfFile -> ACCEPTED
END FUNC
RES g AS SfFile STATE Cursor = launder() ' attaches a Cursor over a live FileInfo
```

So `RES tmp AS SfFile = <stateful>` must be **rejected**. It is unreachable today only
because nothing can produce a stateful resource from a call — **it opens the moment this
sub-plan's append lands.** Land the binding rule in the same change; never after.

This reverses the intuitive answer. Asked in isolation, *"is `stateful → bare` allowed?"*
reads as an obvious **yes** — and it is, for **params**, where bare is safe precisely
because a borrow cannot escape (res.md §2 fact #9). A binding is an owner and *can* escape,
so the same laxity becomes a laundering primitive. **Yes for params, no for bindings**; the
escape distinction is the whole rule, and it is the thing this plan's research got wrong
twice (res.md §6).

Rows, reusing plan-52-C's `TYPE_STATE_MISMATCH`:

| site | rule |
|---|---|
| **Return** | value `STATE T` → return `STATE T` ✓ · value `STATE T` → return **bare** ✗ · value stateless → return bare ✓ |
| **Binding** | init `STATE T` → binding `STATE T` ✓ · init `STATE T` → binding **bare** ✗ · init stateless → binding `STATE T` ✓ (**the one true attach point**) |

## 4. Cross-package

`src/ir/lower.rs:220-224` derives imported functions' returns via
`function_return_from_type` (`:2509-2515`), which takes everything after `") AS "` — so
`"FUNC(String) AS File STATE Cursor"` **would** round-trip textually. plan-52-A Phase 3
audits whether the `.mfp` writer actually emits the STATE in an exported signature.

**This gates the sub-plan's value.** If the STATE does not survive an exported signature,
stateful returns work in-package and silently degrade across a package boundary — and
`bindings/libsnd`, the motivating case, is a package boundary. Resolve the audit before
Phase 2.

Same for `resolver/mod.rs:599` (`return_state_type: None` for re-exports): does
`FUNC alias AS pkg::openTagged` drop the STATE?

## Compatibility / Format Impact

- **Layout / runtime: unchanged.** Byte-identical; the append is a front-end type string.
- **`.mfp`:** may need to carry the STATE in an exported signature (§4). If so, that is a
  format-visible change — coordinate with `./mfb spec package`.
- **Source compatibility:** row 9 (bare binding of a stateful value) is newly rejected. It
  is unreachable today, so nothing existing breaks.
- **Goldens:** no fixture returns a stateful resource, so codegen should not move. The two
  newly-reachable verify rules (row 8) could in principle fire on existing sources —
  verify with the artifact gate rather than assuming.

## Phases

### Phase 1 — the append

- [ ] Append the STATE to the return type string at `src/ir/lower.rs:724-730` and
      `:1963-1970`, mirroring `:739-742`.
- [ ] Confirm rows 7 and 8 flip, and that row 6 is now rejected for the *right* reason
      (expected `File` ≠ actual `File STATE Cursor`, not "everything is rejected").

Acceptance: plan-52-A row 7 builds and prints `pos=42 len=7`; row 8's two programs are
rejected with `TYPE_UNION_STATE_FORBIDDEN` / `TYPE_STATE_INVALID`; row 6 still rejected.
Commit: —

### Phase 2 — the binding rule (same change as Phase 1 — see §3)

- [ ] Reject `RES x AS T = <a value carrying STATE S>` with `TYPE_STATE_MISMATCH`
      (`src/ir/verify/mod.rs`, beside plan-52-C's param rule).
- [ ] Keep `init stateless → binding STATE T` **accepted** — the one true attach point.
- [ ] Flip plan-52-A row 9 from pinned-pending to passing.

Acceptance: the §3 laundering program is rejected; row 9 passes; the attach path (row 9's
inverse) still works.
Commit: —

### Phase 3 — cross-package + libsnd

- [ ] Act on plan-52-A Phase 3's `.mfp` audit (§4); extend the exported signature if the
      STATE does not survive.
- [ ] Act on the `resolver/mod.rs:599` re-export verdict.
- [ ] Add a cross-package fixture: a package exporting `AS RES T STATE S`, an importer
      binding and reading `.state`.
- [ ] Confirm `bindings/libsnd`'s `openFile` wrapper shape compiles.

Acceptance: the cross-package fixture reads the exporter-populated state; libsnd's wrapper
compiles.
Commit: —

### Phase 4 — validation

- [ ] `scripts/artifact-gate.sh`; confirm the codegen delta is nil.
- [ ] Regenerate any goldens the newly-reachable rules shift; confirm the delta is only
      those.
- [ ] Confirm return-type overload identity did not shift — `File` and `File STATE Cursor`
      must remain the **same** return type for overload purposes
      (`SYMBOL_DUPLICATE_TOP_LEVEL`, `$`-mangling).

Acceptance: full suite green; deltas are exactly the intended change.
Commit: —

## Validation Plan

- Tests: plan-52-A rows 6/7/8/9 (flip); the cross-package fixture; the laundering
  rejection.
- Runtime proof: **required and load-bearing.** A build assertion only proves the `RETURN`
  is accepted; only running proves the callee's state actually crossed the return rather
  than being re-default-initialized at the caller's bind (`.ai/compiler.md` gate).
  `pos=42 len=7` vs `pos=0 len=0` distinguishes them unambiguously — a silently
  re-initialized return prints zeros.
- Doc sync: `src/docs/spec/language/15_resource-management.md` — §15 documents STATE on
  bindings and params only, and says merely "A function returns a resource with an explicit
  `AS RES <Type>` return". Add the returning-a-stateful-resource case. If `.mfp` changes,
  `./mfb spec package` too.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Does the STATE survive an exported signature in `.mfp`?** Unconfirmed; plan-52-A
  Phase 3 resolves it. If not, libsnd stays blocked and this sub-plan is half-done (§4).
- **Return-type overloading on the STATE.** Recommend `File` and `File STATE Cursor` are
  the **same** return type for overload identity, so STATE never becomes a discriminator.
  Confirm nothing shifts (Phase 4).
- **Owner-side opt-out.** Shared with plan-52-A. With row 9 rejected, an owner wanting only
  the handle must still write `STATE FileInfo`. That is the price of row 6's promise.
  `STATE _` stays in reserve.

## Summary

Four lines restore a documented, parsed, partially-validated feature — and because the same
string is what the STATE verify rules pattern-match on, the omission simultaneously rejects
the legal program and accepts two illegal ones. The runtime is already complete:
`emit_resource_state_init` explicitly null-checks so a returned resource "keeps" its state,
a path nothing can currently reach. The risk is not the append but what it exposes — the
bare-binding laundering (which **must** land in the same change) and the cross-package
`.mfp` round-trip (which decides whether libsnd is actually unblocked).
