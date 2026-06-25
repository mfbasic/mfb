# MFBASIC `MUT` Lambda Capture Plan

Last updated: 2026-06-25

This document records the current limitation around lambda capture of `MUT`
bindings, why the limitation exists under the current ownership model, and the
plan to address it: allow `MUT` capture only in compiler-proven non-escaping
callback positions. That is the committed approach; this document is the work
to be done, not a survey of options.

It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/threading.md`

## 1. Goal

Make common callback-style code such as:

```basic
MUT total AS Integer = 0
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
```

legal when it is memory-safe to do so, without weakening the language's
ownership and deterministic cleanup guarantees.

The main user-facing motivation is ergonomic:

- `forEach` should support straightforward mutation of an outer accumulator.
- Users should not have to rewrite every callback-style accumulation into
  `reduce`.
- The language should preserve its move/copy/freeze semantics and avoid
  introducing accidental shared mutable aliasing.

## 2. Current State

The current language rule is intentionally strict:

- ordinary closures capture values, not live mutable cells
- capturing a `MUT` binding is a compile-time error
- capturing resources or other non-copyable values is also a compile-time error

This is already reflected in the spec and current diagnostics:

- closures may capture only copyable `LET` bindings by value
- `MUT` capture is rejected because closures do not capture live mutable cells
- resource and other non-copyable captures are rejected for the same reason

These checks live in `src/typecheck.rs` (the `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`
diagnostic). Capture-by-value is implemented in `builder_values.rs`: a closure
allocates an arena env block and **deep-copies** each captured value into it
(`lower_value_owned`), so the env owns an independent copy that outlives the
capturing scope. Captures are read back via `NirValue::Capture { index }`,
loading from the env block. This is why a `MUT` capture would be meaningless
under today's model — the closure would observe a frozen copy, never the live
binding.

Under the current source model, values cross boundaries by:

- copy
- move
- freeze for mutable collections crossing function boundaries

That model does not currently define:

- borrowing a live mutable cell into another scope
- temporarily lending a mutable binding to a callback
- closure-owned mutable storage that outlives the original lexical binding

## 3. Why The Current Restriction Exists

The key issue is not mutation by itself. The issue is allowing a live mutable
binding to cross a scope boundary.

Example:

```basic
FUNC makeCounter() AS FUNC() AS Integer
  MUT total AS Integer = 0
  LET nextValue AS FUNC() AS Integer = LAMBDA() ->
    total = total + 1
    RETURN total
  RETURN nextValue
END FUNC
```

If that were allowed, `nextValue` would escape the scope that owns `total`.
Under the current model, `total` is dropped at scope exit, so the returned
closure would need one of the following:

- a hidden heap-owned mutable cell
- a borrow/lifetime rule proving the closure cannot outlive `total`
- a new ownership rule for closure-owned mutable state

Without one of those, the closure would effectively access storage after the
binding's lexical lifetime ends.

This is why the original example:

```basic
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
```

is attractive but not trivial. It looks local, but semantically it passes code
into another scope.

## 4. Problem Restated In Ownership Terms

MFBASIC currently has move-or-copy ownership semantics, plus freeze for mutable
collections. That means:

- copyable values may be duplicated
- non-copyable values must move
- a moved value cannot be used again
- a mutable collection crossing a boundary becomes a frozen owned value

Since the flat-value rework (plan-02) and scope-drop frees (plan-01 Phase 5 /
plan-02 Phase 8), this model has a concrete implementation shape that matters
for the lowering below:

- every non-resource value is a flat, pointer-free arena block; copy is one
  `memcpy` (`copy_flat_block`) and free is one `arena_free`
- storing a value into a structure, or binding it from an aliasing source,
  inserts a copy (`lower_value_owned`), so each owned local is independent
- each owned flat local is freed at scope exit via
  `ActiveCleanup::OwnedValue`; the free is suppressed on the escape paths
  (`RETURN`, `thread::transfer`)
- "freeze" is now largely a semantic label on top of the flat-copy mechanism
  rather than a distinct runtime operation

Capturing `MUT total` in a lambda does not naturally fit any of those:

- it is not a copy, because the callback must observe and update the same live
  state
- it is not a normal move, because the outer scope expects `total` to remain
  meaningful after the callback interaction
- it is not freeze, because the point is to preserve mutation

So supporting this feature requires adding a new semantic category, not merely
relaxing an existing check.

## 5. The User's Proposed Mental Model

One plausible mental model is:

- move the `MUT` binding into the lambda while it is executing
- let the lambda mutate it
- move the updated value back out when the lambda returns

That model can work, but only in a narrow setting:

- the callback must be non-escaping
- the callback must run synchronously
- the callback must not be stored
- the callback must not be invoked concurrently
- the outer scope must not access the `MUT` binding while the callback owns it

In other words, this model is a good fit for specific callback positions such
as compiler-known `forEach`, but it is not a complete model for ordinary
first-class closures.

## 6. The Approach

Add a compiler-known **non-escaping callback** category and allow `MUT` capture
only in those positions. Everything else stays exactly as today.

Under this model:

- `forEach(items, LAMBDA(x) -> total = total + x)` becomes legal
- `LET f = LAMBDA(x) -> total = total + x` stays illegal
- returning a `MUT`-capturing lambda stays illegal
- storing it in a record, union, list, or map stays illegal
- passing it to unknown functions stays illegal
- sending it to a thread stays illegal

The captured `MUT` is loaned to the callback for the duration of the known,
synchronous call and is the outer binding's again once the call returns. With
flat values this is a borrow of the parent's binding slot — no copy, no
separate free (§11.2) — not a move/restore dance.

The cost is accepted deliberately: a new notion of non-escaping callback
positions, compiler/runtime guarantees that selected built-ins do not retain,
forward, or concurrently invoke their callbacks, and two closure categories
(ordinary first-class closures and non-escaping callback lambdas).

### 6.1 Alternatives rejected

- **Broadly banning function escape routes** (e.g. forbidding `FUNC` return
  types) is too blunt and still does not prove non-escape — lambdas escape via
  locals, collections, retaining callees, and exported APIs — so it does not
  solve local `forEach` mutation.
- **General mutable closure capture** (ordinary closures capturing `MUT` in
  any position) is a much larger feature: it needs closure-owned mutable cells,
  copy-vs-alias and move/drop rules for captured state, thread interaction, and
  package/verifier metadata for mutable environments. Treated as a possible
  future major feature, not this work.

Both are out of scope here; do not broaden to general mutable closures.

## 7. Design Invariants

The approach must keep the language's ownership story coherent:

- normal first-class closure capture remains capture-by-value of copyable
  bindings
- mutable-cell capture exists only as a temporary call-bound loan
- no live mutable cell may outlive its lexical owner
- local callback mutation is allowed only where the compiler can prove safety

## 8. What "Non-Escaping" Must Mean

For a callback position to allow `MUT` capture, the callee must be compiler
known to satisfy all of the following:

- the callback is invoked only during the dynamic extent of the call
- the callback is not stored anywhere
- the callback is not returned
- the callback is not passed onward to another function unless that target is
  also proven non-escaping
- the callback is not invoked concurrently
- the callback is not invoked after the original call returns
- the callback is not invoked from another thread

This property must be part of compiler semantics, not a casual implementation
assumption.

## 9. Candidate Scope For First Support

The smallest initial surface is:

- `forEach`

Possible later expansion:

- `transform`
- `filter`
- `reduce`
- other future built-ins explicitly marked non-escaping

Whether `transform` and `filter` should permit `MUT` capture is a separate
ergonomic choice, not a safety requirement. The main pressure case is
`forEach`.

## 10. Source-Level Behavior

### Allowed

```basic
MUT total AS Integer = 0
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
```

### Still Rejected

```basic
MUT total AS Integer = 0
LET f AS FUNC(Integer) AS Nothing = LAMBDA(x AS Integer) -> total = total + x
```

```basic
FUNC makeCounter() AS FUNC() AS Integer
  MUT total AS Integer = 0
  RETURN LAMBDA() ->
    total = total + 1
    RETURN total
END FUNC
```

```basic
thread::start(LAMBDA(worker, input) -> total = total + input, 1)
```

### Important Invariant

During the non-escaping callback call, the outer `MUT` binding is logically
unavailable except through the callback capture.

That avoids aliasing such as:

```basic
MUT total AS Integer = 0
someCallbackAPI(
  LAMBDA(x AS Integer) -> total = total + x
)
io::print(toString(total))
```

which is fine after the call returns, but not while the callback is in flight.

## 11. Compiler And IR Changes Required

### 11.1 Front-End And Type Checker

Add a notion of callback capture mode:

- ordinary closure capture
- non-escaping mutable-cell capture

The type checker must:

- identify compiler-known non-escaping callback parameters
- allow capturing outer `MUT` bindings only in those positions
- reject `MUT` capture everywhere else
- mark the outer binding unavailable for the duration of the call
- reject nested or forwarded uses that would let the capture escape

### 11.2 IR

The IR must represent non-escaping mutable capture explicitly rather than
pretending it is an ordinary closure environment.

The flat-value + scope-drop-free rework gives a cleaner lowering than the
"transfer ownership in, restore out" sketch this section originally described.
Because a flat value lives in a stable arena block referenced from the
binding's slot for the binding's whole lifetime, and the non-escaping call is
synchronous and cannot outlive that slot, the capture can be a **borrow** with
no move/restore dance:

- ordinary capture (today) deep-copies the value into the closure env and
  registers an independent owned-value free. A non-escaping `MUT` capture does
  the opposite: store a pointer to the parent's live binding (a borrow) into
  the env, with **no copy at capture** and **no free at drop** — the parent
  retains ownership and remains the sole freer.
- the primitives for this already exist: field reads and `UnionExtract` already
  return *borrows* into a parent block, and `is_freeable_flat_value` plus the
  `ActiveCleanup::OwnedValue` exclusion logic already model "this slot is not
  ours to free." The non-escaping `MUT` capture reuses both rather than
  inventing a new ownership category.
- the env slot must reference the binding's **slot** (so a callback that
  reassigns or grows the value is observed by the outer binding — see §12.3),
  not a snapshot of the block pointer.

The important point is semantic clarity:

- do not simulate this as an ordinary heap closure (which would deep-copy and
  independently free) unless the language is ready to define ordinary mutable
  closures
- mark the env slot as a borrow so the existing copy-insertion
  (`lower_value_owned`) and scope-drop free machinery both skip it

### 11.3 Built-In Function Metadata

The compiler needs function metadata that says:

- this callback parameter is non-escaping
- this callback parameter is synchronous
- this callback parameter is not retained or forwarded

That metadata likely belongs in built-in function registration and package
format metadata if user-defined future features ever expose it.

### 11.4 Package And Verifier Metadata

If non-escaping callback positions ever appear in public package APIs, `.mfp`
metadata and verifier rules must preserve:

- callback non-escape guarantees
- mutable capture eligibility
- any relevant thread-safety restrictions

For a built-ins-only first phase, this may remain compiler-internal.

## 12. Hard Cases And Open Questions

### 12.1 Repeated Invocation

If `forEach` calls the callback many times, the model must remain well-defined.

This is acceptable if:

- the captured `MUT` remains loaned to the whole `forEach` call
- each invocation sees the updated value from prior invocations
- no outer access is allowed until the call completes

### 12.2 Re-Entrancy

If a callback could recursively trigger another call that tries to capture the
same `MUT`, the compiler must define whether:

- this is rejected
- or nested loans are serialized

Reject this initially; nested loans of the same `MUT` can be considered later.

### 12.3 Capturing Mutable Collections

If a captured binding is `MUT List` or `MUT Map`, the semantics must be clear:

- the callback is mutating the one live mutable binding
- no freeze occurs unless the collection crosses another ordinary function
  boundary
- no alias to the live mutable buffer can escape

This is feasible under the non-escaping rule but should be tested explicitly.

Flat values add a concrete hazard here. A `MUT List`/`Map` is a flat block in
the binding's slot, and an in-place `append`/insert can **reallocate** the
block (an `arena_alloc` grow) and rewrite the binding's slot pointer. So the
borrow capture (§11.2) must point at the *slot* (double indirection), or the
callback's grow must write the new block pointer back to the parent slot;
capturing a snapshot of the block pointer would leave the outer binding looking
at a stale, possibly freed block. This is exactly the class of register/pointer
hazard that the arena grow path is sensitive to, so collection mutation through
a captured `MUT` must be tested under entropy poisoning, not just acceptance.

### 12.4 Interaction With Resources

This feature should not implicitly permit:

- capturing resources
- storing resources through a `MUT`-capturing callback
- hiding a resource move inside a mutable captured record without existing
  ownership checks

Resource and thread restrictions remain unchanged.

## 13. Validation Strategy

Any implementation should add:

- valid tests for `forEach` with captured `MUT Integer`
- valid tests for `forEach` with captured `MUT List` and `MUT Map` when legal
- invalid tests for assigning a `MUT`-capturing lambda to a variable
- invalid tests for returning a `MUT`-capturing lambda
- invalid tests for passing a `MUT`-capturing lambda to an unknown function
- invalid tests for thread or resource interactions
- runtime validation showing the outer mutable binding reflects callback
  mutations after the call returns

## 14. Implementation Sequence

1. Introduce compiler metadata for non-escaping callback positions on selected
   built-ins.
2. Extend closure analysis to classify captures as:
   - copyable value capture
   - forbidden capture
   - non-escaping mutable capture
3. Implement type-check rules for temporary `MUT` transfer during the proven
   non-escaping call.
4. Add IR support that does not misrepresent the feature as a general escaping
   closure — lower the capture as a borrow of the parent slot (no copy, no
   free), reusing the existing borrow and `ActiveCleanup::OwnedValue`-exclusion
   machinery (§11.2).
5. Add acceptance and runtime tests for the first allowed surface, ideally
   `forEach`.
6. Write the developer manual page `src/man/lambda/package.txt` and wire it into
   the man system (see §16). Update it as the feature lands so it documents what
   actually works, including the new non-escaping `MUT` capture.
7. Only after that, decide whether additional built-ins should opt in.

## 15. Summary

The work is not to allow ordinary closures to capture `MUT` in general. It is
to:

- allow `MUT` capture only for compiler-known non-escaping callbacks
- model it as a temporary call-bound loan of the mutable binding — a borrow of
  the parent slot under the flat-value model (§11.2) — released when the call
  ends
- keep ordinary escaping closures under the current prohibition

That addresses the motivating problem without forcing MFBASIC to adopt a full
general mutable-closure or borrow/lifetime system.

## 16. Developer Manual Page (`mfb man lambda`)

Lambdas have no user-facing reference page today. This work must add one so a
developer can learn the full lambda contract — including the new non-escaping
`MUT` capture rule — from `mfb man lambda`.

### 16.1 Wiring

The page is a topic-only page (no per-function sub-pages), so it follows the
`errors` / `unicode` pattern, not the per-function builtin pattern:

1. Create `src/man/lambda/package.txt`. The first section must be a `NAME` block
   whose line reads `lambda - <one-line summary>` (the man loader parses that
   line); follow with the usual `SYNOPSIS` / `DESCRIPTION` / further sections.
2. In `build.rs`, declare the page path and emit a `cargo:rerun-if-changed` line
   for it, exactly as `errors_page` / `unicode_page` do, so edits trigger a
   rebuild. No `*_FUNCTION_PAGES` / `*_TOPIC_PAGES` generation is needed.
3. In `src/man/mod.rs`, add
   `parse_package(include_str!("lambda/package.txt"), "mfb man lambda")` to the
   `PACKAGES` vector. Do **not** add a `lambda` arm to `generated_pages`; with no
   arm it resolves to no sub-pages (empty `functions`), which is correct for a
   topic page.

`scripts/update_man.sh` and any man-page index/test should be re-run so the new
page is picked up.

### 16.2 Required content

The page documents lambdas as they actually behave; it is not aspirational.
Until the `MUT` work lands, it must describe the current prohibition; once it
lands, it must describe the non-escaping exception. It must cover at least:

- **Syntax** — `LAMBDA(params) -> expression` single-expression form and the
  multi-line body form ending in `RETURN`; parameter type annotations; how the
  result type is inferred.
- **Function types** — the `FUNC(argTypes) AS ReturnType` type, binding a lambda
  to a `LET` of function type, and passing/returning function values.
- **What you can do** — pass lambdas to higher-order built-ins (`forEach`,
  `transform`, `filter`, `reduce`, …); bind them to `LET`; return them; store
  them in records/collections (subject to the capture rules below).
- **Capture rules** — closures capture copyable `LET` bindings **by value** (an
  independent copy, not a live reference); `MUT` capture is rejected everywhere
  except in compiler-proven non-escaping callback positions (e.g. `forEach`),
  where the binding is loaned for the duration of the call; resource and other
  non-copyable captures are always rejected. Name the diagnostic
  (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`) so users can map errors to the rule.
- **What is not possible** — capturing `MUT` in an escaping closure (assigning
  it to a variable, returning it, storing it, sending it to a thread, or passing
  it to an unknown function); observing a captured value's later mutations
  through a by-value capture; capturing resources.
- **Examples** — at minimum the allowed `forEach` accumulator and the rejected
  `makeCounter`-style escaping closure from §10, each with a short note on why.

Keep the page consistent with `specifications/mfbasic.md` (§13–§14 ownership,
capture, and drop rules) and with this plan; if they ever disagree, the spec and
this plan are authoritative and the page must be corrected.
