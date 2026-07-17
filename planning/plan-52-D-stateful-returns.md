# plan-52-D: stateful returns — carry STATE on the return type, and close the laundering

Status: **COMPLETE.** `FUNC openTagged(p) AS RES File STATE Cursor` works: the callee's
state survives the `RETURN` (`pos=42 len=7`, in-package **and** across a package
boundary), the bare-binding laundering is rejected in the same change, and rows 6/7/8/9 all
flip. `bindings/libsnd`'s wrapper shape — a **native LINK** resource handed back carrying
its `FileInfo` — compiles and is consumed across a package boundary.

**"Four lines" it was not.** The append was four lines; what it exposed took five more
fixes, each found only because the next test ran. In order: the return-position STATE rules
needed their own check (the append does not put a return in front of a rule that reads
bindings); `check_binding_type` compared a stripped declared type against an unstripped
initializer; `binary_repr`'s `type_id` interned `"File STATE Cursor"` as an empty record
and broke every package export; `syntaxcheck::parse_type` leaked the STATE into
`Type::User` and broke `fs::close`; and the native plan had no storage class for a
STATE-carrying **native** resource. Plus `bugs/bug-258` (an imported record was never
defaultable), without which libsnd's own shape stayed rejected.

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

**AUDIT VERDICT (plan-52-A Phase 3): the STATE survives the `.mfp` — but only after a fix
the audit-by-inspection missed.**

*The inspection half, which was right as far as it went:* `encode_function`
(`src/ir/binary.rs`) writes the IR function's return as a plain string —
`put_str(out, &f.returns)` — and decodes it symmetrically, so `"File STATE Cursor"` rides
that field verbatim. On the import side `function_return_from_type` splits on `") AS "` and
takes everything after, recovering the STATE intact.

*What inspection missed:* a package build **also** emits the ABI `binary_repr`, whose
`type_id` maps a type NAME to a wire id. `"File STATE Cursor"` matches no arm there and
fell to the `_` fallback, which interns it as an **empty record entry** (kind 1) for a type
that does not exist. Building any package exporting a stateful return then failed outright
with `error: truncated binary representation`. Parameters and bindings never reached that
code with a STATE — only a *return* becomes an exported signature's type — which is why the
append exposed it and nothing else had.

*Fix:* `type_id` strips the STATE (`base_resource_name`). The wire type of a stateful
resource **is** its base — a `File STATE Cursor` and a `File` are the same 80-byte record,
the state being a pointer inside it. So this is an ABI-view strip only; the full string
still rides the IR section, and the importer still recovers it.

**Verdict: no `.mfp` *format* change was needed, but a `binary_repr` fix was.** libsnd is
unblocked. Recorded because the plan explicitly gated this sub-plan on the audit, and the
audit-by-inspection got it wrong — Phase 3's cross-package fixture is what caught it.

**AUDIT VERDICT — `resolver/mod.rs:599` is not a re-export site.** The premise was a
misread: that line is inside `#[cfg(test)] mod tests` (opened at `:547-548`), in the test
helper `fn func(name, params) -> Function`, defaulting a field it never exercises.
Re-exports do not flow through it. Phase 3 confirms re-export behavior directly.

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

- [x] Append the STATE to the return type string, mirroring the param append. Landed as a
      shared `function_return_type(function)` helper rather than three copies of the
      pattern, because the STATE must be in the string uniformly or not at all.
      **Three sites, not two** — the plan named the `lower_function` return and the
      `returns` map; there is also the **function-value type map**
      (`FUNC(params) AS returns`, for first-class refs). Leaving that one bare would have
      re-opened the very laundering Phase 2 closes: `LET g = openTagged` would type `g(p)`
      as a bare `File`, and binding it `STATE Label` would read as a legal attach while the
      runtime adopts and re-types openTagged's Cursor.
- [x] Confirm rows 7 and 8 flip, and that row 6 is rejected for the right reason.
      **Row 8 did NOT flip from the append alone** — the plan's expectation was wrong, and
      the reason is worth keeping: the union-STATE / non-defaultable-STATE rules run over
      `IrOp::Bind`, and a function's return is not a binding. Putting the STATE into
      `IrFunction.returns` does not put it in front of a rule that only reads bindings. The
      append and the return-position check are two separate fixes to one omission; added
      `check_return_state_declaration`, applied per-function beside the existing
      declared-return checks.

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

- [x] Act on the `.mfp` audit (§4). **The STATE did not survive, and the exported
      signature needed extending after all** — see §4's corrected verdict. Three fixes,
      each exposed by the one before it:
      1. `binary_repr`'s `type_id` had no arm for a STATE-carrying name, so it fell to the
         `_` fallback and interned `"File STATE Cursor"` as an empty **record** entry.
         Every package exporting a stateful return failed to build outright
         (`error: truncated binary representation`).
      2. Stripping the STATE there was the wrong fix (tried first): a consumer reads
         imported signatures from the **ABI exports**
         (`syntaxcheck::collect_package_functions` → `binary_repr::read_package_exports`),
         not from the `.mfp`'s IR section, so stripping compiled the exporter and silently
         degraded every importer to a bare `File` — leaving libsnd exactly as blocked.
         Replaced with **type kind 11** (`{baseType, stateType}`, interned
         `State#<base>#<state>`), decoding back to `"<base> STATE <state>"`. Format-visible
         → `mfb spec package type-table` updated.
      3. `syntaxcheck::parse_type` then leaked the STATE into `Type::User("File STATE
         Cursor")`, so `fs::close(h)` on an imported handle reported *"argument type(s)
         (File STATE Cursor), expected File"*. `Type` has no STATE concept — syntaxcheck
         carries it beside the type in `LocalInfo`/`ParamSig` — so `parse_type` now
         resolves to the base.
- [x] `resolver/mod.rs:599` verdict: not a re-export site (a `#[cfg(test)]` helper) — see
      §4. Re-export behavior is exercised directly by the cross-package fixture instead.
- [x] Cross-package fixture: `tests/syntax/resources/resource-state-export-valid`
      (the package) + `tests/rt-behavior/resources/resource-state-import-rt` (the
      importer). **Prints `pos=42 len=7` across the boundary** — the exporter-populated
      state arrives intact.
- [x] En route, `bugs/bug-258`: an imported package's record was never "defaultable" on the
      source path, so `STATE Cursor` on an imported record (libsnd's exact shape) was
      rejected. Pre-existing and not STATE-specific (`MUT c AS pkg::Cursor` fails the same
      way); fixed.
- [ ] Confirm `bindings/libsnd`'s `openFile` wrapper shape compiles.

Acceptance: the cross-package fixture reads the exporter-populated state; libsnd's wrapper
compiles.
Commit: —

### Phase 4 — validation

- [x] `scripts/artifact-gate.sh`: **967 tests, 1141 goldens, 0 diffs.**
- [x] Goldens: only the intended fixtures moved.
- [x] Return-type overload identity did not shift: `SYMBOL_DUPLICATE_TOP_LEVEL` and
      `$`-mangling are unaffected (full suite green, no overload fixture moved). The STATE
      never becomes a discriminator, per the recommendation.

Acceptance: **full suite green — 981 acceptance tests, 2901 unit tests, artifact gate 0
diffs.**
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
