# bug-254: the resource `STATE` visibility model is unspecified — `RES x AS T` means different things by position, and a borrow can attach state it should only observe

Last updated: 2026-07-16
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: — (Phase 1)

`STATE` appears in four positions — a `RESOURCE` declaration, a binding, a parameter,
and a return — and the language has never stated what `RES x AS SfFile` versus
`RES x AS SfFile STATE FileInfo` *means* in each. The compiler's behavior in each
position is therefore incidental rather than designed: two of the four positions
happen to be right, one is right for the wrong reason, and one is wrong in two
different ways. bug-252 and bug-253 are each one half of the resulting damage, and
both of them contain Open Decisions that cannot be answered without this model. **This
document is the governing model; 252 and 253 become its implementation.**

The correct behavior a fix produces: **the model below holds in all four positions,
with a state attached exactly once — at its owning binding — and never observable at a
type other than the one it was attached as.**

References:

- `./mfb spec language resource-management` §15 — defines `STATE` on bindings and says
  it "rides through `RES` signatures", but never states the variance: what a signature
  *without* a `STATE` accepts, or what a bare return promises.
- bug-252 (a `FUNC` cannot return a stateful resource) — the **return** half. Its fix
  produces this model's return rule exactly (see Finding 1).
- bug-253 (STATE type confusion) — the **param/binding** half. Finding 3 below
  **contradicts its Fix Design** and must be reconciled.
- Model proposed by the user, 2026-07-16, in review of `bindings/libsnd`'s `openFile`.

## The Model

`RESOURCE SfFile CLOSE BY sndLink::closeFile` declares the resource. **No `STATE` in
the declaration** — a resource type has no intrinsic state; state is attached at a
binding. (This is the language today: `resourceDecl` has no `STATE` clause,
`src/ast/items.rs:540-567`. Recorded as decided; see Non-goals.)

| Position | `RES x AS SfFile` | `RES x AS SfFile STATE FileInfo` |
| --- | --- | --- |
| **Param** | accepts a SfFile with **any state or none**; `.state` is **not accessible** | accepts **only** a SfFile carrying `FileInfo`; `.state` **is** accessible |
| **Return** | returns a resource with **no state** — not "any state" | returns a resource **carrying** a `FileInfo` |
| **Binding** (owner) | **no state** — attaches nothing | attaches/adopts a `FileInfo` |

The load-bearing asymmetry: **bare means "opaque" in param position and "none"
everywhere else.** A bare param says "I can't see the state"; a bare return says "there
is no state." Two readings, one spelling.

That is sound only because a bare param can never escape into a return position: §15's
borrow rule forbids returning a borrowed resource, so the "opaque" reading is confined
to the frame that borrowed it. Verified:

```
error[2-203-0086 TYPE_RESOURCE_BORROW_INVALIDATE]: a borrowed resource cannot be
closed, returned, or transferred
  Binding `p` is a borrowed resource; only its owner may close, `RETURN`, or transfer it.
```

**The rule that falls out, and that the fix must hold to:** bare erases state *only*
where the resource cannot escape (params). Everywhere it can escape (bindings,
returns), bare must mean **provably no state**.

## Failing Reproduction

All four positions, built against `target/debug/mfb` on macOS aarch64 with a clean
`build/`. Uses the built-in `File` + `Cursor` (the model is resource-agnostic; `SfFile`
is not needed to demonstrate it).

| Case | Model says | Today | Verdict |
| --- | --- | --- | --- |
| `test(RES p AS File)` given a stateful `File` | accept; `.state` inaccessible | accepts; `.state` inaccessible | ✓ correct |
| `test2(RES p AS File STATE Cursor)` given `STATE Label` | reject | **accepts → type confusion** | ✗ bug-253 |
| `test2(RES p AS File STATE Cursor)` given a **stateless** `File` | reject | **accepts → param attaches the state** | ✗ **new, Finding 3** |
| `test3() AS RES File` returning a stateful `File` | reject | rejects | ✓ **but by accident, Finding 1** |
| `test4() AS RES File STATE Cursor` returning a stateful `File` | accept | **rejects** (`TYPE_RETURN_MISMATCH`) | ✗ bug-252 |

### The new one — a borrow attaches state it should only observe

```basic
IMPORT io
IMPORT fs

TYPE Cursor
  pos AS Integer
END TYPE

SUB a(RES p AS File STATE Cursor)
  p.state.pos = 7
  io::print("a saw pos = " & toString(p.state.pos))
END SUB

SUB main()
  RES h AS File = fs::openFile("src/main.mfb")   ' owner is STATELESS
  a(h)
  io::print("main survived")
END SUB
```

- Observed: **builds clean**, prints `a saw pos = 7` / `main survived`. The *borrow*
  allocated a `Cursor` on an owner that never declared one.
- Expected: rejected — `h` carries no state, `a` demands `STATE Cursor`.

Why it matters beyond tidiness: attachment-from-a-borrow makes the state type a
free-for-all. Two borrows disagreeing is then reachable **without a single stateful
binding anywhere**:

```basic
SUB a(RES p AS File STATE Cursor)   ' first call allocates a Cursor
SUB b(RES p AS File STATE Label)    ' second reads that block as a Label
RES h AS File = fs::openFile(...)   ' stateless owner
a(h)
b(h)                                ' -> bug-253's confusion, no diagnostic
```

### Contrast cases that work correctly today (regression guards)

- **Bare param + stateful argument** → accepted, state survives intact (`pos still = 42`
  after the call). This is the model's param rule, and it must not regress.
- **`.state` write through a bare param** → correctly rejected, with a purpose-built
  message: `TYPE_STATE_INVALID: `p` has no STATE to assign; declare the resource with
  `STATE T`.`
- **Returning a borrow** → correctly rejected (`TYPE_RESOURCE_BORROW_INVALIDATE`). The
  model depends on this.
- **Owner declares STATE, borrow declares the same STATE** →
  `tests/rt-behavior/resources/resource-state-field-assign-valid`. The borrow observes
  and mutates; the owner sees it after the call. Must not regress.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | `target/debug/mfb`, console executable, clean `build/` | 3 of 5 cases wrong |

Platform-independent by inspection: every defect is in the target-neutral front end.

## Root Cause

There is no model, so each position was implemented against a local intuition:

- **Param (bare)** — correct by construction. `STATE` rides *inside* the type string
  (`"File STATE Cursor"`, split by `builtins::resource::state_type_name`,
  `src/builtins/resource.rs:231-233`). A bare param's type string has no `" STATE "`, so
  `.state` cannot resolve — the capability restriction is a free consequence of the
  encoding, not a decision.
- **Param (stateful)** — wrong twice. `src/ir/lower.rs:739-742` carries the declared
  STATE into the param's local type string, and codegen's bind path then runs
  `emit_resource_state_init` (`src/target/shared/code/builder_value_semantics.rs:10-36`),
  which allocates iff the slot is null. Nothing compares the param's declared state type
  against the argument's. So a param **attaches** when the slot is null (Finding 3) and
  **re-types** when it is not (bug-253).
- **Return** — the type string never carries the STATE at all
  (`src/ir/lower.rs:724-730` omits `return_state_type`; bug-252). The return rule is
  therefore vacuous: it rejects *every* stateful return, which is coincidentally right
  for `test3` and wrong for `test4`.

The single mechanism underneath all three: `emit_resource_state_init` decides by
**null-check, not type-check**, and the payload carries no runtime type tag. Its
call-site comment ("a moved/returned resource that already carries a state keeps it")
is correct for the *move* case it was written for; nothing scoped it to that case.

## Goal

- All five rows of the reproduction table match the Model column.
- A state is attached **exactly once**, at its owning binding. No borrow ever attaches.
- No program observes a state payload at a type other than the one it was attached as.
- `resource-state-field-assign-valid` and the four sibling STATE fixtures pass unchanged.

### Non-goals (must NOT change)

- **`STATE` on the `RESOURCE` declaration.** Considered and **rejected** by the user,
  2026-07-16. Intrinsic state (`RESOURCE SfFile STATE FileInfo CLOSE BY …`) would give
  one declaration site, making disagreement unrepresentable and killing bug-253's class
  for that resource — but it forfeits the bare-param opt-out ("I don't touch the
  state"), which this model makes load-bearing for close ops. Recorded here so it is not
  re-litigated; revisit only if the restatement burden (below) proves worse than the
  opt-out is worth.
- **The borrow rule** (`TYPE_RESOURCE_BORROW_INVALIDATE`). The model's param/return
  asymmetry is sound *because* of it. It is a dependency, not collateral.
- **`emit_resource_state_init`'s null-check.** Still correct for the move/return case
  bug-252 needs. The fix is a front-end rejection, not a codegen change.
- **The bare-param opt-out.** `RES p AS File` accepting a stateful resource, with
  `.state` inaccessible, is the model's param rule — not laxity to be tightened.
- **Tempting wrong fix: making a bare param mean "stateless only."** It would make bare
  uniform across positions and kill Finding 2's laundering — and it would break every
  close op, which must accept the resource whatever state it carries. The asymmetry is
  the design.

## Blast Radius

- `src/ir/lower.rs:724-730` + `:1963-1970` (return type string) — **bug-252**; the
  return half of the model.
- `src/ir/verify/mod.rs:825-845` (the STATE rules) — the natural home for the new
  relational rules; today they check a declared state type only in isolation.
- **Argument→param STATE agreement** — **bug-253** plus Finding 3. Both halves of the
  param rule land here.
- **Binding→initializer STATE agreement** — Finding 2's laundering; **latent, opens when
  bug-252 lands.**
- `src/target/shared/code/builder_value_semantics.rs:10-36`
  (`emit_resource_state_init`) — the mechanism. Unchanged; see Non-goals.
- `src/target/shared/code/builder_arena_transfer.rs:336-337` (`thread::transfer` moves
  the state pointer) — **latent, inherited from bug-253's audit and still unresolved.**
  If an `ISOLATED FUNC` entry can declare a different STATE than the sender's binding,
  the model is violated across a thread boundary.
  `tests/rt-behavior/threads/thread-transfer-state-rt` covers only the agreeing case.
- `tests/rt-behavior/resources/resource-state-field-assign-valid` and 4 siblings — the
  regression guards. All 14 in-tree `STATE` uses are bindings and params, all agreeing;
  **none exercises a return, a disagreement, or a param-attach.** That is why this
  shipped.

## Findings

### Finding 1 — `test3`'s guarantee is load-bearing, and bug-252's fix produces it

`test3() AS RES File` promising **no state** is not a stylistic choice; it is what makes
a caller's fresh attachment safe:

```basic
RES h AS File STATE Cursor = test3()
```

The caller's binding allocates a `Cursor` *only because the slot is null*. If `test3`
could return a resource secretly carrying a `FileInfo`, that attachment would silently
alias the `FileInfo` and read it as a `Cursor` — bug-253's confusion, arriving through a
return.

The pleasing part: **the check that enforces this already exists.** `check_return_type`
(`src/ir/verify/mod.rs:3895-3909`) compares the returned value's type string against the
declared return's. bug-252's fix — append the STATE to the return type string — makes it
do exactly the right thing in both directions:

- `test4`: expected `File STATE Cursor` == actual → **accept**.
- `test3` returning a stateful value: expected `File` ≠ actual `File STATE Cursor` →
  **reject**, guarantee upheld.

bug-252 is not just an unblock; it is the return rule's implementation. Nothing further
is needed for the return position.

### Finding 2 — a bare *binding* launders state; opens the moment bug-252 lands

A bare binding erases the STATE from the type string. Once returns carry STATE, that
erasure defeats the return check:

```basic
FUNC launder() AS RES SfFile             ' promises "no state"
  RES tmp AS SfFile = openStateful()     ' bare bind of a stateful value — allowed?
  RETURN tmp                             ' expected SfFile, actual SfFile -> ACCEPTED
END FUNC

RES g AS SfFile STATE Cursor = launder() ' attaches a Cursor over a live FileInfo
```

`RES tmp AS SfFile = <stateful>` must therefore be **rejected**, or `test3`'s guarantee
is unenforceable.

**This reverses a recommendation.** bug-252 and bug-253 each carry an Open Decision —
*"Is `stateful → bare` allowed? Recommend **yes**"* — reasoned from the param case,
where bare is safe because `.state` is unreachable and the borrow cannot escape. A
**binding** is an owner: it can escape, so the same laxity becomes a laundering
primitive. The answer is **yes for params, no for bindings** — the escape distinction is
the whole rule, and neither doc drew it.

Currently unreachable (bug-252 blocks producing a stateful resource from a call at all),
which is exactly why it must be fixed *with* bug-252 rather than after it.

### Finding 3 — a borrow attaches state it should only observe

Verified above: a stateful param on a stateless owner **allocates** the state. Under the
model, `test2` accepts only a resource already carrying `FileInfo`, so this is a
rejection.

**This contradicts bug-253's Fix Design**, which states *"the rule is asymmetric:
stateless → stateful is fine (allocate)"* and warns against over-rejecting it as the
main correctness risk. That was reasoned from `resource-state-field-assign-valid` — but
re-reading that fixture, its **owner** declares `RES f AS File STATE Cursor`, so the
param never allocates. **No in-tree fixture depends on param-attach.** bug-253's stated
risk is largely phantom, and its rule is the unsafe one: param-attach is what makes two
disagreeing borrows reachable with no stateful binding anywhere.

The model's rule is cleaner and strictly safer: **attachment happens exactly once, at
the owning binding; params only ever observe.**

### Finding 4 — the `.state` read path has no diagnostic

Write through a bare param is diagnosed precisely:

```
error[2-203-0085 TYPE_STATE_INVALID]: `p` has no STATE to assign; declare the resource with `STATE T`.
```

Read is not. `p.state.pos` degrades to `Unknown` and the error surfaces wherever that
Unknown lands — observed as `TYPE_CALL_ARGUMENT_MISMATCH` on `toString` complaining
about argument types, with no mention of STATE. It is rejected, so this is a diagnostic
gap and not a hole, but the two paths should say the same thing.

## Fix Design

Four relational rules, all decidable from type strings the verifier already holds. The
shape is a `TYPE_STATE_MISMATCH`-family rule in `src/ir/verify/mod.rs:825-845`:

| Site | Rule |
| --- | --- |
| **Param** | arg `STATE T` → param `STATE T` ✓ · arg `STATE T1` → param `STATE T2` ✗ · arg **stateless** → param `STATE T` ✗ (Finding 3) · arg anything → param **bare** ✓ (opt-out) |
| **Return** | value `STATE T` → return `STATE T` ✓ · value `STATE T` → return **bare** ✗ · value stateless → return **bare** ✓ |
| **Binding** | init `STATE T` → binding `STATE T` ✓ · init `STATE T` → binding **bare** ✗ (Finding 2) · init stateless → binding `STATE T` ✓ (**the one true attach point**) |
| **Thread** | pending Phase 1's `thread::transfer` verdict |

The return row is bug-252's fix; the param rows are bug-253's, corrected per Finding 3.

The correctness risk is **not** the rules — it is that the escape distinction (params
may erase, owners may not) is subtle enough that both prior docs got it backwards.
Encode it once, explicitly, rather than re-deriving it per site.

Restatement burden, accepted knowingly: under this model an owner that wants the handle
but not the state must still write `RES h AS SfFile STATE FileInfo`, because a bare
binding is now a "no state" *assertion* rather than an opt-out. There is no owner-side
opt-out. That is the price of `test3`'s guarantee, and it is what the rejected
RESOURCE-level design would have avoided. If it grates in practice, the escape hatch is
a third notion — "opaque state" — spelled distinctly (`STATE _`), permitted on owners,
and forbidden in a return. Not proposed now; recorded so the option is not lost.

Expected output shift: no fixture returns a stateful resource, disagrees on a state
type, or relies on param-attach — so goldens should not move. Verify with the artifact
gate rather than assuming.

## Phases

### Phase 1 — the model + failing tests + audit (no behavior change)

- [ ] Land the Model table into `./mfb spec language resource-management` §15. **This is
      the deliverable**; the code fixes are consequences. §15 currently documents STATE
      on bindings only and is silent on all four variance questions.
- [ ] Add a syntax fixture per wrong row: param state mismatch, param-attach on a
      stateless owner (Finding 3), bare-binding of a stateful value (Finding 2 — will be
      unreachable until bug-252 lands; add it as a pinned pending case).
- [ ] Add the runtime fixture proving the two-disagreeing-borrows confusion is a real
      wrong-type read.
- [ ] Confirm **no** in-tree fixture depends on param-attach (initial read: none; all 14
      STATE uses agree).
- [ ] Resolve the `thread::transfer` verdict inherited from bug-253's audit.
- [ ] Reconcile bug-252's and bug-253's Open Decisions against Findings 2 and 3, and
      cross-link them here.

Acceptance: §15 states the model; each new fixture fails for its documented reason; every
audit item has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Land bug-252's return-type-string append (the Return row).
- [ ] Land the param rows, corrected per Finding 3 (supersedes bug-253's Fix Design).
- [ ] Land the binding row, closing Finding 2's laundering **in the same change** as
      bug-252 — never after it.
- [ ] Give the `.state` read path a real diagnostic (Finding 4).

Acceptance: all five reproduction rows match the model; the four contrast cases are
unchanged; nothing in Non-goals moved.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/artifact-gate.sh`; confirm the codegen delta is nil.
- [ ] Regenerate any goldens the new rules shift; confirm the delta is only that.
- [ ] `scripts/test-accept.sh` green; `cargo test --bin mfb` green.
- [ ] Re-run all five rows end-to-end; re-run `bindings/libsnd`'s `openFile` shape.

Acceptance: full suite green; deltas are exactly the intended change.
Commit: —

## Validation Plan

- Regression tests: one fixture per row of the model table, plus the runtime
  wrong-type-read proof, plus the four contrast guards.
- Runtime proof: **required** for the rows that currently *build* — a build assertion
  cannot distinguish "attached correctly" from "aliased the wrong payload"
  (`.ai/compiler.md` runtime completion gate). After the fix the proof inverts for the
  invalid rows: they must not build.
- Doc sync: `./mfb spec language resource-management` §15 — the Model table. Phase 1's
  deliverable, not an afterthought.
- Full suite: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Owner-side opt-out.** Under the model there is none — a bare binding asserts "no
  state". Recommend accepting the restatement burden for now; the `STATE _` "opaque"
  spelling stays in reserve. (§Fix Design)
- **`thread::transfer`** — inherited unresolved from bug-253. If the resource plane
  admits a state-type disagreement, that is a cross-thread type confusion and may
  warrant its own severity.
- **Diagnostic codes.** One `TYPE_STATE_MISMATCH` across all four sites, or a code per
  site (param/return/binding)? Recommend one code with a site-specific message — the
  user's fix is the same shape everywhere.

## Summary

Four positions, two spellings, and no stated model — so `RES x AS SfFile` came to mean
"any state or none" as a param and "no state" as a return **by accident**, and the
accident is only safe because the borrow rule keeps the two from meeting. Writing the
model down turns three of the four positions into ordinary comparisons the verifier
already has the information to make. The two findings that matter are corrections to my
own prior docs: bare-binding must **not** be allowed (bug-252/253 recommend the
opposite, and it launders `test3`'s guarantee the moment 252 lands), and param-attach
must **not** be allowed (bug-253's Fix Design blesses it, on a risk that re-reading the
fixtures shows to be phantom). The escape distinction — params may erase state, owners
may not — is the single idea both docs missed, and the only one worth encoding once.
