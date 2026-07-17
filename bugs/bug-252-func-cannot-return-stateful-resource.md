# bug-252: a `FUNC` cannot return a resource carrying `STATE`, and the return's `STATE` escapes every verify rule

Last updated: 2026-07-16
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: — (Phase 1)

`returnType = "AS" [ "RES" ] type [ "STATE" type ]` is in the grammar, the parser
builds it (`src/ast/items.rs:79-91`), and `syntaxcheck` validates it
(`src/ir/../syntaxcheck/mod.rs:2034-2044`) — but `ir::lower` never appends the
`STATE` to the function's return type string. So the declared return of
`FUNC openTagged(...) AS RES File STATE Cursor` lowers to the bare string `File`,
while the value being returned lowers to `File STATE Cursor`. The two are compared
at the `RETURN` and rejected. **A stateful resource can never be returned from a
`FUNC`**, which is the only way a callee could hand a caller a resource with a
populated `STATE`.

The same omission has a second, opposite symptom: the two verify rules that police
`STATE` are keyed on finding `" STATE "` *in the type string*
(`src/ir/verify/mod.rs:825-845`). Because the return's type string never contains
it, `TYPE_UNION_STATE_FORBIDDEN` and `TYPE_STATE_INVALID` **never fire on a return
type**. Violations that are correctly rejected on a binding compile clean on a
return. So the same missing append both rejects the legal program and accepts two
illegal ones.

The correct behavior a fix produces: **`FUNC f(...) AS RES File STATE Cursor` with
`RETURN <a RES f AS File STATE Cursor>` compiles, and the caller's binding receives
the resource with the callee's populated state intact; and a `STATE` on a return
type is subject to exactly the same rules as a `STATE` on a binding.**

References:

- `./mfb spec language resource-management` §15 — `STATE` is the language's only
  way to attach data to a resource ("Data that belongs with a resource travels in
  the resource's STATE"), and §15 lists `RETURN` of a resource as one of the four
  ownership-transfer events.
- `./mfb spec language grammar` §19 — `returnType = "AS" [ "RES" ] type [ "STATE" type ]`,
  with the note "STATE in a returnType is likewise honored only when `RES` is present."
- bug-253 (STATE type confusion) — **must be read alongside this doc**: it is the
  hole this fix's caller/callee state-type agreement question runs straight into.
  Fixing 252 without 253 lets a caller re-type a returned state payload.
- Found while designing `bindings/libsnd`'s `openFile` wrapper (`sf_open` filling
  an `SF_INFO`), which wants exactly this signature.

## Failing Reproduction

Built against `target/debug/mfb` on macOS aarch64. Project is a stock
`kind: executable` with `sources: [{root: src, role: main, include: ["**/*.mfb"]}]`.

### (a) The legal program is rejected

```basic
IMPORT io
IMPORT fs

TYPE Cursor
  pos AS Integer
  len AS Integer
END TYPE

FUNC openTagged(path AS String) AS RES File STATE Cursor
  RES f AS File STATE Cursor = fs::openFile(path)
  f.state.pos = 42
  f.state.len = 7
  RETURN f
END FUNC

SUB main()
  RES h AS File STATE Cursor = openTagged("src/main.mfb")
  io::print("pos=" & toString(h.state.pos) & " len=" & toString(h.state.len))
END SUB
```

- Observed: **exit 1**, no binary produced —
  `error[2-203-0041 TYPE_RETURN_MISMATCH]: RETURN value has type File STATE Cursor, expected File.`
- Expected: builds, and prints `pos=42 len=7`.

Declaring the return as bare `AS RES File` while returning a stateful `RES f AS
File STATE Cursor` fails **identically** — the value's type string carries the
`STATE` either way, so there is no spelling that returns a stateful resource.

### (b) Two illegal programs are accepted

Both are the *exact* violations that
`tests/syntax/resources/resource-union-state-invalid` and
`tests/syntax/resources/resource-state-invalid` pin on a **binding**, moved verbatim
to a **return type**:

```basic
' A resource union carries no STATE (TYPE_UNION_STATE_FORBIDDEN on a binding).
FUNC openStream(path AS String) AS RES Stream STATE StreamState
  RES f AS File = fs::openFile(path)
  RETURN f
END FUNC
```

```basic
' An ENUM is not a defaultable data type (TYPE_STATE_INVALID on a binding).
FUNC openTinted(path AS String) AS RES File STATE Color
  RES f AS File = fs::openFile(path)
  RETURN f
END FUNC
```

- Observed: **both build clean and run** (exit 0, no diagnostic of any kind).
- Expected: `TYPE_UNION_STATE_FORBIDDEN` and `TYPE_STATE_INVALID` respectively —
  the same codes the binding fixtures already assert.

### Contrast cases that work correctly today (regression guards)

- **Stateless return + caller attaches the STATE** → works. `FUNC openPlain(p) AS
  RES File` returning a bare `RES f AS File`, with the caller binding `RES h AS
  File STATE Cursor = openPlain(...)`, builds and prints the caller-assigned value
  (verified: `pos=99`). This is the only working pattern today, and it is the
  workaround — but the callee cannot pre-populate the state, which is the whole point.
- **STATE on a `RES` binding** → correct (`resource-state-valid` and 4 sibling
  fixtures).
- **STATE on a `RES` param** → correct (`resource-state-field-assign-valid`) —
  `src/ir/lower.rs:739-742` appends it, and this is the model for the fix.
- **The return's STATE annotation is inert, not merely dropped**: annotating
  `AS RES File STATE Cursor` while returning a *stateless* `RES f AS File` also
  builds and runs. The annotation changes nothing at all — it does not even alter
  the expected type.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | `target/debug/mfb`, console executable | (a) fails ✗, (b) fails ✗ (accepts) |

Platform-independent by inspection: the defect is in target-neutral IR lowering,
above every backend.

## Root Cause

`STATE` is carried by convention **inside the type string** — `"File STATE Cursor"` —
and recovered with `crate::builtins::resource::state_type_name`
(`src/builtins/resource.rs:231-233`, a literal `split_once(" STATE ")`), with
`base_resource_name` stripping it for resource recognition. Every stage that needs
`STATE` reads it back out of that string.

Two of the three sites that build a type string append the `STATE`. The return site
does not:

- `src/ir/lower.rs:739-742` (**params**) — appends, with the comment "Carry a `RES`
  parameter's `STATE T` in the local type string so `s.state` resolves inside the
  callee, matching `lower_param`."
- `src/ir/lower.rs:974-977` (**bindings**) — appends, with the comment "A `RES`
  binding's `STATE T` rides in the lowered type string (`File STATE T`) so codegen
  can default-initialize and address the state payload."
- `src/ir/lower.rs:724-730` (**returns**) — **does not append.** It takes
  `function.return_type` raw:

```rust
let returns = match function.kind {
    FunctionKind::Func => function
        .return_type
        .clone()
        .unwrap_or_else(|| "Unknown".to_string()),
    FunctionKind::Sub => "Nothing".to_string(),
};
```

`function.return_state_type` is simply never read here — nor anywhere in `ir/` or
`target/`. Its only consumers tree-wide are `syntaxcheck/mod.rs:2041` (which passes
it to `check_resource_declaration`, `src/syntaxcheck/checking.rs:74-87` — a
`check_type_reference` existence check and nothing more) and `ast/serialize.rs:710`
(a one-way AST→JSON dump). Every other mention **constructs** it as `None`
(`escape.rs:411`, `monomorph/helpers.rs:521`, `resolver/mod.rs:599`,
`testing/desugar.rs`). The field is parsed, existence-checked, and discarded.

That single omission produces both symptoms:

- **(a) the legal program is rejected.** `returns` becomes `current_return_type`
  (`src/ir/lower.rs:745-746`), which `check_return_type`
  (`src/ir/verify/mod.rs:3895-3909`) compares against the returned value's inferred
  type. The value is a local whose type string *does* carry the STATE, so
  `expression_compatible("File", "File STATE Cursor")` fails →
  `TYPE_RETURN_MISMATCH`. The two strings are built by different rules.
- **(b) the illegal programs are accepted.** `src/ir/verify/mod.rs:825-845` gates
  both `TYPE_UNION_STATE_FORBIDDEN` and `TYPE_STATE_INVALID` on
  `type_.find(" STATE ")` over a **binding's** type string. A return type string can
  never contain `" STATE "`, so neither rule is reachable from a return.

A second site builds a return type string with the same omission:
`src/ir/lower.rs:1963-1970` populates the `returns` map used by `expression_type`
for calls. This is why `openTagged(p).state` cannot resolve and why an
un-annotated `RES h = openTagged(...)` infers a state-less type. A fix to
`:724-730` alone would fix the `RETURN` and leave call-expression typing broken.

Why the contrast cases are immune: params and bindings both go through the two
appending sites above, so their type strings are well-formed and every downstream
`state_type_name` lookup — `.state` member typing (`src/ir/lower.rs:2188-2195`,
`src/target/shared/code/builder_value_semantics.rs:175-190`) and state init
(`builder_control.rs:289-297`) — sees the STATE.

The runtime plumbing for a returned stateful resource **already exists and is
correct**: the state is a pointer in the resource record at `FILE_OFFSET_STATE`
(= 16, `src/target/shared/code/error_constants.rs:652`); resource constructors zero
it (`fs_helpers_io.rs:792`, `fs_helpers_atomic.rs:201,1772`, `net/mod.rs:275`); and
`emit_resource_state_init` (`src/target/shared/code/builder_value_semantics.rs:10-36`)
**null-checks that slot and skips init when it is already populated**, with the
comment "a moved/returned resource that already carries a state keeps it (the slot
is non-null)". That comment describes precisely the scenario this bug makes
unreachable. Only the type-string append is missing.

## Goal

- Reproduction (a) builds and prints `pos=42 len=7`.
- Reproduction (b)'s two programs are rejected with `TYPE_UNION_STATE_FORBIDDEN`
  and `TYPE_STATE_INVALID` — the same codes as the sibling binding fixtures.
- `openTagged(p).state` resolves to `Cursor` from the call expression, and
  `RES h = openTagged(p)` (no annotation) infers `File STATE Cursor`.
- The stateless-return contrast case (caller attaches the STATE) keeps working.

### Non-goals (must NOT change)

- **The `" STATE "` type-string encoding.** Replacing the stringly-typed convention
  with a structured field is a real cleanup, but it touches every
  `state_type_name`/`base_resource_name` caller and is a separate plan. This bug
  restores the missing append within the existing encoding.
- **`FILE_OFFSET_STATE` / the resource record layout.** The runtime side is correct
  and stays byte-identical.
- **`emit_resource_state_init`'s null-check semantics.** "Allocate once, a carried
  state survives a move" is the behavior the fix *depends on*.
- **LINK native funcs keep having no STATE clause.** `parse_link_function`
  (`src/ast/items.rs:748`, return parsed at :771-776) deliberately omits
  `parse_optional_state()`, and the grammar states "The native return has no STATE
  clause." A binding package wraps its native func in an ordinary `EXPORT FUNC` that
  carries the STATE. Do not add STATE to the LINK grammar under this bug.
- **Tempting wrong fix #1: making `expression_compatible` strip `" STATE "` before
  comparing.** This makes reproduction (a) pass — the runtime would even work — while
  leaving the return type string state-less. Call-expression `.state` typing stays
  broken, symptom (b) stays broken, and a caller declaring a *different* STATE type
  than the callee is silently accepted (that is bug-253, and this "fix" would make it
  reachable through returns too). The type string must carry the STATE; the compare
  must not be loosened.
- **Tempting wrong fix #2: deleting `return_state_type` and the grammar clause**
  ("nobody uses it"). The grammar documents it, `syntaxcheck` validates it, and
  `bindings/libsnd` needs it. The feature is unfinished, not unwanted.

## Blast Radius

Found by grepping `return_state_type`, `state_type_name`, and `FILE_OFFSET_STATE`
tree-wide — not from memory.

- `src/ir/lower.rs:724-730` (`lower_function`'s `returns`) — **the bug**; fixed here.
- `src/ir/lower.rs:1963-1970` (the `returns` map feeding `expression_type` for
  calls) — **same omission, in scope**: without it, call-expression `.state` typing
  and un-annotated inference stay broken.
- `src/ir/lower.rs:220-224` (`function_return_from_type` over
  `external_function_types`, i.e. **imported/cross-package** functions) — **in scope,
  needs confirmation in Phase 1.** `function_return_from_type`
  (`src/ir/lower.rs:2509-2515`) takes everything after `") AS "`, so `"FUNC(String) AS
  File STATE Cursor"` would textually yield `"File STATE Cursor"` and round-trip.
  Must confirm the `.mfp` writer actually emits the STATE in an exported signature —
  otherwise a stateful return works in-package and silently degrades across a package
  boundary, which is exactly the `bindings/libsnd` use case.
- `src/ir/verify/mod.rs:825-845` (`TYPE_UNION_STATE_FORBIDDEN`, `TYPE_STATE_INVALID`) —
  **fixed by this bug as a consequence**: once the return type string carries the
  STATE these become reachable. Confirm they key off returns as well as bindings
  rather than assuming the string append is sufficient.
- `src/ir/verify/mod.rs:3895-3909` (`check_return_type`) — a consumer, not a cause.
  It compares whatever `lower` produced; no change expected once the strings agree.
- `src/target/shared/code/builder_control.rs:289-297` (state default-init at bind) —
  unaffected; the null-check already handles a carried state.
- `src/target/shared/code/builder_arena_transfer.rs:336-337` (thread::transfer copies
  the state slot across the resource plane) — unaffected: it moves the pointer at
  `FILE_OFFSET_STATE` without consulting the type string.
- `src/target/shared/code/builder_value_semantics.rs:175-190` (`.state` read typing) —
  unaffected mechanically, but it is the site that *benefits*: it types the load from
  the type string, so a STATE-carrying return type is what makes `openTagged(p).state`
  work.
- `src/escape.rs:411`, `src/monomorph/helpers.rs:521`, `src/resolver/mod.rs:599`,
  `src/testing/desugar.rs:300,1100,1117` — construct `return_state_type: None` for
  synthesized functions. Unaffected: none of them synthesizes a stateful-resource
  return. Re-confirm `resolver/mod.rs:599` in Phase 1 — it builds functions for
  re-exports/aliases, and a `FUNC alias AS pkg::openTagged` re-export of a stateful
  return would drop the STATE there.
- `bindings/sqlite3`, `bindings/libsnd` — consumers, not causes. libsnd's `openFile`
  is the motivating case and should become a fixture once it works.

## Fix Design

Mirror the param path at `src/ir/lower.rs:739-742` at both return-type sites
(`:724-730` and `:1963-1970`): when `function.return_resource` is set and
`function.return_state_type` is `Some(state)`, lower the return type as
`format!("{return_type} STATE {state}")`.

That is a small change; the engineering risk is **not** the append. It is in three
places the append newly exposes:

1. **Caller/callee STATE agreement.** With the return typed `File STATE Cursor`, a
   caller binding `RES h AS File STATE Cursor` matches, but what about `RES h AS
   File` (drops the annotation) or `RES h AS File STATE Other` (re-types it)? The
   latter is bug-253's type confusion arriving through a return. The binding compare
   (`TYPE_BINDING_MISMATCH`) would newly reject both, which is likely correct for
   `Other` and **arguably wrong for the bare `File` case** — a caller that ignores the
   state is reasonable, and the state is freed by the close op regardless (confirm:
   `emit_resource_cleanup_call`, `src/target/shared/code/builder_codegen_primitives.rs:1512`,
   only calls the close symbol; the payload free is inside the close helper / arena,
   not driven by the binding's type string — so a bare bind should not leak). Decide
   this deliberately; see Open Decisions.
2. **Cross-package `.mfp`.** Per the blast radius, confirm the STATE survives the
   exported-signature encode/decode before claiming the feature works for binding
   packages.
3. **Return-type overload sets.** A callable's identity includes its return type
   (§6). Whether `File` and `File STATE Cursor` are two distinct return types for
   overload purposes falls out of the type string changing — check that
   `SYMBOL_DUPLICATE_TOP_LEVEL` and the monomorphizer's `$`-mangling do not shift.

Rejected alternative: loosening `expression_compatible` to ignore `" STATE "` — see
Non-goals, tempting wrong fix #1. It treats the symptom at the compare and leaves the
type string (the actual carrier) wrong.

Expected output shift: no existing fixture returns a stateful resource (confirmed:
all 14 `STATE` uses under `tests/` are bindings and params), so goldens should not
move. The two newly-reachable verify rules could in principle fire on existing
sources — verify with the artifact gate rather than assuming.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `tests/rt-behavior/resources/resource-state-return-rt/`: reproduction (a),
      asserting the callee-populated state (`pos=42 len=7`) survives the return.
      Confirm it fails today with `TYPE_RETURN_MISMATCH`.
- [ ] Add `tests/syntax/resources/resource-return-union-state-invalid/` and
      `resource-return-state-invalid/`: reproduction (b)'s two programs. Confirm they
      build clean today (i.e. the fixtures fail by *not* erroring).
- [ ] Add the stateless-return contrast (caller attaches STATE) as a regression guard
      so the workaround pattern stays pinned.
- [ ] Confirm the three flagged audit items: the `.mfp` cross-package round-trip, the
      `resolver/mod.rs:599` re-export path, and that a bare `RES h AS File` bind of a
      stateful resource does not leak the payload.

Acceptance: the new tests fail for the documented reasons; every blast-radius entry
has a verdict written into this file.
Commit: —

### Phase 2 — the fix

- [ ] Append `STATE` to the return type string at `src/ir/lower.rs:724-730` and
      `:1963-1970`, mirroring the param path at `:739-742`.
- [ ] Resolve the caller/callee STATE-agreement decision (Open Decisions) and make
      the binding compare match it.
- [ ] Extend the cross-package path if Phase 1 shows the STATE does not survive the
      `.mfp` signature.

Acceptance: Phase 1's tests pass; the contrast cases still behave as documented;
nothing in Non-goals moved.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/artifact-gate.sh`; confirm the codegen delta is nil (no existing
      fixture returns a stateful resource).
- [ ] Regenerate any goldens the newly-reachable verify rules shift; diff and confirm
      the delta is only that.
- [ ] `scripts/test-accept.sh` green; `cargo test --bin mfb` green.
- [ ] Re-run reproductions (a) and (b) end-to-end.

Acceptance: full suite green; golden deltas are exactly the intended change; (a)
prints `pos=42 len=7` and (b) is rejected with the two expected codes.
Commit: —

## Validation Plan

- Regression tests: `tests/rt-behavior/resources/resource-state-return-rt/` (runtime
  proof), plus the two `tests/syntax/resources/resource-return-*-invalid/` fixtures.
- Runtime proof: **required**. A build assertion only proves the `RETURN` is accepted;
  only running the binary proves the callee's populated state actually crossed the
  return rather than being re-default-initialized at the caller's bind (`.ai/compiler.md`
  runtime completion gate). `pos=42 len=7` vs `pos=0 len=0` distinguishes them
  unambiguously — a state-less return that silently re-inits would print zeros.
- Doc sync: `./mfb spec language resource-management` §15 should gain the returning-a-
  stateful-resource case (it currently documents `STATE` only on bindings and params,
  and says only "A function returns a resource with an explicit `AS RES <Type>` return").
- Full suite: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Should a caller be allowed to bind a stateful return with a bare `RES h AS File`
  (dropping the annotation)?** Recommend **yes** — the state is still freed by the
  close op, and forcing every caller to restate the STATE type is noise. This requires
  the binding compare to accept `File` ← `File STATE Cursor` (one-way), while still
  rejecting `File STATE Other` ← `File STATE Cursor`. The asymmetry must be
  deliberate, not incidental. (§Fix Design 1)
- **Does the STATE survive an exported signature in `.mfp`?** Unconfirmed; Phase 1
  resolves it. If it does not, `bindings/libsnd` — the motivating case — still cannot
  use the feature, and the bug is only half fixed.
- **Return-type overloading on the STATE.** Recommend treating `File` and
  `File STATE Cursor` as the *same* return type for overload identity, to avoid
  accidentally making STATE an overload discriminator. Confirm nothing shifts. (§Fix
  Design 3)

## Summary

A missing four-line append in `src/ir/lower.rs` makes a documented, parsed,
partially-validated language feature unusable — and because the same string is what
the `STATE` verify rules pattern-match on, the omission simultaneously rejects the
legal program and accepts two illegal ones. The runtime plumbing is already complete
and correct: `emit_resource_state_init` explicitly null-checks the state slot so a
returned resource "keeps" its state, a code path nothing can currently reach. The real
risk is not the append but what it exposes — caller/callee STATE agreement (which runs
into bug-253's type confusion), the cross-package `.mfp` round-trip, and return-type
overload identity. The type-string encoding, the record layout, the thread-transfer
plane, and LINK's deliberate no-STATE rule all stay untouched.
