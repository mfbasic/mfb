# bug-572: a capturing `LAMBDA` leaks its closure environment on every call that takes it

Last updated: 2026-09-07
Effort: small
Severity: MEDIUM (bounded per call site, unbounded across repeats)
Class: Memory / closures

Status: **FIXED** (2026-09-07, `93a4f8aee`)
Regression Test:
- `tests/runtime/rt_scope_drop_leaks.rs` — four RSS cases at N and 2N
  (`a_capturing_lambda_argument_does_not_leak_its_environment`,
  `a_captureless_lambda_argument_stays_flat` (the contrast),
  `every_admitted_callback_position_frees_its_closure` covering `reduce`'s binary
  combiner, `forEach`'s by-ref capture and `mapValues`' `.mfb` body), plus
  `a_retained_callback_position_is_left_alone` — an RSS EQUALITY, not flatness,
  because `http::route` must keep leaking — and
  `every_closure_argument_shape_still_produces_the_right_value`, 25 runs over
  eleven shapes.
- `tests/codegen/codegen_closure_temp_drop.rs` — five comparative owner counts,
  three of them negative (the retained `http::route` position, the three escapes,
  and the user HOF that returns its parameter).
- `src/codegen/registry/mod.rs::every_registry_function_parameter_callback_is_synchronous`
  — the enumeration, as a partition of the whole registry rather than a list.

Found while auditing bug-569's callback enumeration, as the residual under a
capturing-lambda callback. It is independent of the callback's return type — it
reproduces with a `Boolean` predicate, which allocates no result block at all.

## Failing reproduction

```
IMPORT io
IMPORT collections

SUB main()
  LET cap AS String = "CAPTURED"
  LET xs AS List OF String = ["n0", "n1", "n2", "n3", "n4", "n5", "n6", "n7"]
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 50000
    LET c AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap))
    acc = acc + len(collections::get(c, 0))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`), macos-aarch64, release:

| N (outer passes) | peak RSS |
|---|---|
| 50 000 | 13 MB |
| 100 000 | 25 MB |

The contrast is one token: dropping the capture (`len(s) < 8`, so the lambda is
capture-less and lowers to a `FunctionRef` rather than a `Closure`) is **1.0 MB
flat** at both counts.

## Why it matters beyond the megabytes

§14.4 says "a closure environment is owned by the function value. Dropping the
function value drops its captured values in reverse capture order." The function
value here is a temporary that dies at the end of the statement, and nothing drops
it — so the environment block, and the copies of the captured values inside it,
are never reclaimed.

## What a fix must produce

The loop above runs at constant RSS; a capture-less lambda stays flat; and a
closure that ESCAPES the statement (returned, stored in a collection, or passed to
`thread::start`) is not freed at the statement end. `collect_value_used_locals`
(`function_lowering.rs`) already exists to answer exactly that escape question for
a closure binding, so the shape of the gate is already in the tree.

## The fix (2026-09-07)

A capturing `LAMBDA` written as a call ARGUMENT now has an owner: the call it was
written for frees it once that call has returned.

`register_pending_temp` could not do this, and must not be made to.
`pending_temp_is_freeable` requires `is_freeable_flat_value`, which is false for
`Func` — and has to stay false, because a `Func` element in a collection is a
shared POINTER (bug-73), so the flat `arena_free` of a surrounding value must
never chase it. So the closure is recorded on its own list
(`pending_closure_temps`) and drained per CALL rather than per statement: the
call is the entire reason the closure exists and the only window in which it is
live. The free itself is the existing `emit_closure_drop` — captures, then the
env block, then the 16-byte object, each null-guarded, with the free-and-null
that bug-440 established.

### The gate: two arms, and what each proves

The question is whether the callee RETAINS the callback. `is_non_escaping_closure`
already answers it for a named closure BINDING, from `collect_value_used_locals`;
this asks the same question one frame down, about a parameter:

* **A callee with a NIR body** — a user `FUNC`, or a monomorphised `.mfb` builtin
  like `collections::mapValues`, whose body invokes `f(e.value)` directly. The
  parameter must be a non-isolated `FUNC` **and** its name must never be read as a
  VALUE anywhere in the body. A `Call`'s target is a `String`, not a `NirValue`, so
  an invoke does not count as a use — which is exactly what makes the predicate
  usable here. A name shadowed by an inner local reads as "used" and declines,
  which is the fail-closed direction.
* **A native builtin**, which has no body to read. The registry's declared
  parameter type is the whole of the evidence, so the licence is an explicit
  ALLOW-list (`SYNCHRONOUS_CALLBACK_PARAMETERS`) partitioned against
  `RETAINED_CALLBACK_PARAMETERS`, and a test asserts the two together are the
  ENTIRE registry set — a new `FUNC` parameter reds it and forces a decision
  rather than defaulting into either half.

### Why it is an allow-list: `http::route`

The first version of this gate admitted *any* non-isolated `FUNC` parameter. The
enumeration test caught the counterexample immediately:
`http::route(pattern, handler) AS http::Route` takes a non-isolated
`FUNC(Request) AS Response` at index 1 and **stores it on the record it returns**,
where the server invokes it once per request. Freeing it when `route` returned
would have been a use-after-free in the request loop, on a shape no leak test
would ever have flagged. It is on `RETAINED_CALLBACK_PARAMETERS` now, and its
codegen and its (pre-existing, unrelated) RSS are byte-identical before and after
this fix — 6 -> 11 MB at 20k/40k on both compilers.

`http::Route.handler` is separately out of reach: it is a record FIELD, and a
record constructor is not a `Call`, so no arm ever sees it.

`thread::start`'s entry is retained too, and is excluded twice over: it is
`ISOLATED FUNC`, and `ir::lower` builds every lambda's type with
`isolated = false`, so a capturing lambda cannot be typed there at all.

### Everything else keeps leaking, deliberately

`false` is the pre-fix behaviour, so an unrecognised shape leaks rather than
freeing a block someone still holds. Declined and measured to be unchanged:
a closure appended to a `List OF FUNC(…)` (the collection stores the pointer), one
passed to a user function that RETURNS it, one assigned to a global, and
`RETURN LAMBDA …`. The three-escape probe reads 11 -> 21 MB at 20k/40k on both
compilers — byte-identical, which is the evidence that nothing new is freed.

### Measured (macos-aarch64, release, `/usr/bin/time -l`, peak RSS)

| Shape | before (N / 2N) | after (N / 2N) |
|---|---|---|
| `filter` + capturing `Boolean` predicate | 13 / 25 MB | 1 / 0 MB |
| `reduce` + capturing binary combiner | 13 / 25 MB | 0 / 0 MB |
| `transform` + capturing transform | 13 / 26 MB | 0 / 1 MB |
| `mapValues` + capturing transform (`.mfb` arm) | 15 / 29 MB | 1 / 1 MB |
| `forEach` + BY-REF `MUT` capture | 7 / 13 MB | 0 / 0 MB |
| **contrast** capture-less `filter` predicate | 0 / 0 MB | 0 / 0 MB |
| **pin** `http::route` (retained) | 6 / 11 MB | 6 / 11 MB |
| **pin** append + user-passthrough escapes | 11 / 21 MB | 11 / 21 MB |

N = 50 000 calls over an 8-element collection, 2N = 100 000 (20k/40k for the two
pins). Every program printed the identical value before and after; the eleven-shape
value probe is byte-identical across compilers and stable over 30 runs.

### Golden delta: 9, and the prediction that was wrong

Zero was predicted and 9 was measured, so it was localized rather than
regenerated. The registration is gated on the enclosing call having ALREADY
decided it may free the closure, so a program that passes no capturing lambda to
an admitted position emits not one extra instruction — and the prediction was
right about every `.mfb` file in `tests/`. What it missed is that the standard
library is compiled into these fixtures too, and the `crypto` package's own
helper bodies pass a capturing lambda:

```
RETURN __crypto_hkdfExpand(prk, info, length,
  LAMBDA(mk AS List OF Byte, md AS List OF Byte) -> __crypto_hmac(algo, mk, md))
```

`algo` is captured. `__crypto_hkdfExpand` and `__crypto_pbkdf2Block` are ordinary
`.mfb` functions whose `hmac` parameter is only ever invoked, so the NIR-body arm
admits it — and the closure had been leaking one env + one capture copy + one
object on every `crypto::hkdf`, `crypto::pbkdf2` and HPKE `labeledExpand` call.

The `-ncode` dumps of both diffing fixtures were diffed per function against the
bug-571 commit: **exactly three functions changed —
`crypto::hkdf`, `crypto::pbkdf2`, `crypto::hpkeLabeledExpand` — each gaining
exactly one closure owner, and not one function changed without gaining one.**
The 9 regenerated goldens are the same 9 the gate flagged.

It is not measurable as RSS on `crypto::hkdf`: ~50 bytes per call against the
~1.6 KB per call the rest of that function still leaks (32 -> 63 MB at 20k/40k,
byte-identical before and after). The codegen count is the evidence, which is the
case for counting owners rather than megabytes.

### Not fixed here

A capturing closure that ESCAPES still leaks — appended to a collection, returned,
stored in a global, or handed to a callee that keeps it. That is the same open
question `is_non_escaping_closure` leaves for a binding it declines, and it needs
an owner with a LIFETIME rather than a call-scoped free. Also unchanged: a `MUT`
function-value rebind does not free the closure it replaces
(`is_freeable_flat_value` is false for `Func`, so the old-block free at
`NirOp::Assign` never fires), and neither does a global re-store.

