# MFBASIC `MUT` Lambda Capture Plan

Last updated: 2026-06-15

This document records the current limitation around lambda capture of `MUT`
bindings, why the limitation exists under the current ownership model, and the
 main design options for fully addressing it.

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

## 6. Design Options

### Option A: Keep The Current Rule

Do nothing. Continue rejecting `MUT` capture in all lambdas and closures.

Pros:

- simplest semantics
- no new ownership category
- no new verifier or package metadata rules
- no risk of hidden aliasing or lifetime leaks

Cons:

- poor ergonomics for common callback-style local accumulation
- forces users toward `reduce` even when a local mutable accumulator reads more
  clearly
- keeps the language stricter than necessary for obviously local cases

This is safe but unsatisfying.

### Option B: Allow `MUT` Capture Only For Non-Escaping Callbacks

Add a compiler-known non-escaping callback category.

Under this model:

- `forEach(items, LAMBDA(x) -> total = total + x)` can be legal
- `LET f = LAMBDA(x) -> total = total + x` stays illegal
- returning a `MUT`-capturing lambda stays illegal
- storing it in a record, union, list, or map stays illegal
- passing it to unknown functions stays illegal
- sending it to a thread stays illegal

The compiler would treat the captured `MUT` as temporarily transferred for the
duration of the known call and restored after the call returns.

Pros:

- solves the motivating ergonomic problem
- matches the move-in / mutate / move-out intuition
- preserves the current no-escaping-mutable-cell rule
- avoids a general borrow or lifetime system

Cons:

- requires a new notion of non-escaping callback positions
- requires compiler/runtime guarantees that selected built-ins do not retain,
  forward, or concurrently invoke callbacks
- creates two closure categories: ordinary first-class closures and
  non-escaping callback lambdas

This is the recommended first implementation path.

### Option C: Ban Function Escape Routes More Broadly

One idea is to forbid function return types, or otherwise reduce the ways a
lambda can escape.

This is not sufficient by itself.

Even if function return types are banned, lambdas can still escape through:

- assignment to locals
- storage in records or collections
- passing to another function that retains them
- exported APIs that accept function values

So a rule like "functions cannot be return types" is too blunt and still does
not solve the real problem, which is escape.

Pros:

- reduces some escape paths

Cons:

- regresses existing or future higher-order patterns
- still does not prove non-escape
- does not directly solve local `forEach` style mutation

This is not a recommended fix.

### Option D: General Mutable Closure Capture

Allow ordinary closures to capture `MUT` bindings in general.

To do this correctly, the language would need to define:

- closure-owned mutable cells
- closure copying vs aliasing semantics
- move/drop behavior for captured mutable state
- whether multiple closures may share a mutable captured cell
- interaction with threads
- package metadata and verifier rules for mutable closure environments

This is effectively a larger language feature:

- either a hidden boxed mutable-cell model
- or a borrow/lifetime system
- or both

Pros:

- most expressive
- supports closure factories such as counters and iterators

Cons:

- much larger semantic surface
- materially complicates ownership, verification, packages, and runtime
- easy to get subtly wrong

This should be treated as a future major feature, not the first fix.

## 7. Recommended Direction

Implement Option B first:

- support `MUT` capture only in compiler-proven non-escaping callback
  positions
- preserve the current prohibition for ordinary escaping closures

This keeps the language's ownership story coherent:

- normal first-class closure capture remains capture-by-value of copyable
  bindings
- mutable-cell capture exists only as a temporary call-bound ownership
  transfer
- no live mutable cell may outlive its lexical owner

In short:

- allow local callback mutation where the compiler can prove safety
- do not broaden to general mutable closures yet

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

## 10. Source-Level Behavior Under The Recommended Model

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
thread::start(worker, LAMBDA(x AS Integer) -> total = total + x)
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

## 11. Compiler And IR Changes Required For Option B

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

Possible lowering model:

- callback block or lambda object with explicit mutable capture slots
- call-site setup that transfers ownership of the mutable binding into the
  callback invocation context
- restoration after the non-escaping call finishes

The important point is semantic clarity:

- do not simulate this as an ordinary heap closure unless the language is ready
  to define ordinary mutable closures

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

Rejecting this in the first implementation is likely simpler.

### 12.3 Capturing Mutable Collections

If a captured binding is `MUT List` or `MUT Map`, the semantics must be clear:

- the callback is mutating the one live mutable binding
- no freeze occurs unless the collection crosses another ordinary function
  boundary
- no alias to the live mutable buffer can escape

This is feasible under the non-escaping rule but should be tested explicitly.

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

## 14. Recommended Implementation Sequence

1. Introduce compiler metadata for non-escaping callback positions on selected
   built-ins.
2. Extend closure analysis to classify captures as:
   - copyable value capture
   - forbidden capture
   - non-escaping mutable capture
3. Implement type-check rules for temporary `MUT` transfer during the proven
   non-escaping call.
4. Add IR support that does not misrepresent the feature as a general escaping
   closure.
5. Add acceptance and runtime tests for the first allowed surface, ideally
   `forEach`.
6. Only after that, decide whether additional built-ins should opt in.

## 15. Recommendation Summary

The best first fix is not to allow ordinary closures to capture `MUT` in
general.

The best first fix is:

- allow `MUT` capture only for compiler-known non-escaping callbacks
- model it as temporary transfer of the mutable binding into the callback and
  back out when the call ends
- keep ordinary escaping closures under the current prohibition

That addresses the motivating problem without forcing MFBASIC to adopt a full
general mutable-closure or borrow/lifetime system immediately.
