# bug-562: a `String` callback whose body is the `toString` identity SIGSEGVs

Last updated: 2026-09-07
Effort: small-to-medium (the change is one arm; the AUDIT is the work)
Severity: **HIGH** (a crash on ordinary source; no diagnostic)
Class: Memory / callback ABI

Status: **Fixed** (pending land)
Regression Test:
- `tests/rt-behavior/collections/callback-string-return-identity-rt` — the crash,
  plus every callback shape below, 40 repetitions with a digest-drift check.
  Pre-fix: `[exit 139]` with no output at all.
- `tests/codegen/codegen_string_return_freshness.rs::being_used_as_a_callback_never_removes_the_return_copy`
  — the invariant, as an owner count. Pre-fix: 0 copies as a callback vs 1 called
  directly.
- `…::a_callback_whose_result_is_already_fresh_is_not_copied_twice` — the positive pin.
- `tests/runtime/rt_scope_drop_leaks.rs::a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss`
  — the caller-side half, as peak RSS at N and 2N. Pre-fix 25.6 -> 50.2 MB; after
  1.0 -> 1.0 MB.

Found while fixing bug-536 shape B-2. **Reproduces identically on the base commit
and after that change.**

## Failing reproduction — fourteen lines

```
IMPORT io
IMPORT collections

FUNC identish(s AS String) AS String
  RETURN toString(s)
END FUNC

SUB main()
  MUT xs AS List OF String = []
  MUT k AS Integer = 0
  WHILE k < 3
    xs = collections::append(xs, "n" & toString(k))
    k = k + 1
  END WHILE
  LET c AS List OF String = collections::transform(xs, identish)   ' [exit 139]
  io::print("c=" & collections::get(c, 0))
END SUB
```

Observed: `[exit 139]` (SIGSEGV). Expected: `c=n0`.

## Root cause

The `FunctionRef` ABI **owns and frees** a callback's return value. That is
precisely why plan-86 K1 excludes callback-referenced functions from the
param-borrow elision — the exclusion FORCES a copy, so the value the HOF frees is
one it owns.

`identish` escapes that force: it returns a `Call`, not a bare `Local`, so it is
**not** a param-borrow function and K1's exclusion never applies to it. Meanwhile
`toString`'s `String` arm is the identity, so it hands the HOF the caller's own
list-element block — which the HOF then frees. The list is left holding freed
memory.

So the defect is the *interaction* of two correct-looking pieces: the identity arm
and an ABI that frees.

## The fix, and why it is not a one-word change in practice

bug-536 shape B-2's `function_returns_fresh_string` currently **excludes**
callback-referenced functions. That exclusion is deliberately conservative — it
keeps callback lowering byte-identical rather than changing a second ABI in one
step — and it is recorded as the one place that predicate is knowingly weaker than
it should be.

**Dropping the `callback_referenced` arm is the fix**: it turns the exclusion from
"no obligation" into "the callee copies", which is exactly what K1's exclusion
achieves for the borrow shape.

One word of code, and then the real work: a **callback-ABI audit**. Every shape
that reaches a `FunctionRef` return has to be enumerated and shown to hand the HOF
a block it owns — the identity arm is the one that was found, not necessarily the
only one. A partial fix here produces a double free rather than a leak.

## What a fix must produce

The repro prints `c=n0` and exits 0, `collections::transform` and every other HOF
still free exactly once, and a callback that returns a genuinely fresh block is
not copied twice.

Positive pin required: an existing callback shape must be measurably unchanged —
byte-identical codegen for a callback that already worked.

---

## The fix, as landed

`function_returns_fresh_string` (`src/codegen/engine/function/function_lowering.rs`)
no longer excludes callback-referenced names. Three lines of code; the rest of the
change is the audit, the fixtures and the doc comment.

The predicate is consulted from BOTH ends of the same contract — the callee takes
on "hand back a solely-owned block on every return path", the call site takes the
licence to free the result at statement end — so dropping the arm turns the
exclusion from "no obligation" into "the callee copies", which is exactly what
plan-86 K1's exclusion achieves for the borrow shape.

## The callback-ABI audit

### What the ABI actually frees

Only one block: the per-iteration `String` **argument**.
`emit_load_collection_payload`'s `String` arm `arena_alloc`s a fresh block for
each element (a packed `String` has no standalone header to point at), and
`free_collection_loop_item` frees it after the callback returns — a no-op for
every other element type, which allocate nothing. Its doc comment states the
assumption this bug broke:

> The callback receives it by value and does not take ownership, so the block is
> dead the moment the callback returns and freeing it here is safe. **A callback
> that *returns* something derived from it returns a separate allocation.**

Nothing enforced that last sentence for a return that was not a bare parameter.
The ABI does **not** free the callback's *result* — that is bug-569, and it is why
this fix converts the crash into the same per-element leak every already-working
`String` callback has always had, rather than into a clean program.

### Every shape that can reach a `FunctionRef` return

*What a callback can BE*, exhaustively:

| callback source | in `module.functions`? | covered by the predicate? |
|---|---|---|
| a named user `FUNC` | yes | yes |
| a `.mfb`-package function | yes (merged) | yes |
| a capture-less `LAMBDA` | yes (`context.lambdas`), lowered as `FunctionRef` | yes |
| a capturing `LAMBDA` | yes (`context.lambdas`), lowered as `Closure` | yes — the predicate no longer consults the callback set at all |
| a builtin passed directly | **no** | n/a — the complete set is `isEven isOdd isPositive isNegative isZero isEmpty isNotEmpty isNumeric` (`builtin_function_id`), **all `Boolean`**, so none can hand back a block |

The `Closure` row was the one gap worth checking: `collect_function_ref_names`
collects `NirValue::FunctionRef` only, not `NirValue::Closure`, so a capturing
lambda is invisible to plan-86 K1's exclusion. It is safe by construction — a
lambda body is a single expression, so a body that *is* a bare parameter has no
free variables and therefore no captures, and is lowered as a `FunctionRef`. A
param-borrow capturing lambda cannot be written. If lambdas ever gain block
bodies, `collect_function_ref_names` must also collect `Closure { name }`.

*What its return can BE*, by `lower_returned_value` arm — and this is the part
that makes the fix total rather than shape-coupled, because the new arm is a
**catch-all keyed on the lowered type**, not a recognised shape. The default is
"copy"; an unrecognised shape is copied, not assumed fresh.

| arm | reached by | who owns the block the HOF gets |
|---|---|---|
| 1. `move_elided` | `RETURN <owned local>` (plan-25-C C1) | the HOF — the local's scope-drop free was removed for this path |
| 2. `value_needs_owning_copy` | `RETURN <param>`, `RETURN "literal"`, `RETURN <param-borrow call>`, `RETURN <rodata-string call>`, `RETURN <borrowed get>` | the HOF — `copy_flat_block` |
| 3. tail pending temp | `RETURN <concat>`, `RETURN <marked native producer>` | the HOF — `register_pending_temp`'s own precondition is a fresh standalone block, and `claim_pending_temp` drops the statement-scope free |
| 4. **new** — any other `String` | `RETURN toString(s)`, `RETURN <unmarked native producer>`, `RETURN <LINK symbol call>`, `RETURN <call to a function not in `functions`>` | the HOF — `copy_flat_block` |
| 5. fall through, no copy | reachable only when the type is not `String` | n/a — the ABI frees no non-`String` block |

The arms are mutually exclusive early returns, so nothing is copied twice.

### Measured, per shape

`collections::transform` over a `List OF String`, 50 repetitions, source list
re-read after every pass, on the pre-fix binary and after:

| callback body | before | after |
|---|---|---|
| `RETURN toString(s)` | **`[exit 139]`** | ok |
| `RETURN toString(toString(s))` | **`[exit 139]`** | ok |
| `LAMBDA(s AS String) -> toString(s)` | **`[exit 139]`** | ok |
| `RETURN s` | ok | ok |
| `RETURN passer(s, "other")` (param-borrow helper) | ok | ok |
| `RETURN collections::get(xs, 0)` | ok | ok |
| `RETURN "<" & s & ">"` | ok | ok |
| `MUT out = "L"` / `out = out & s` / `RETURN out` | ok | ok |
| `RETURN "short"` | ok | ok |
| `RETURN strings::upper(s)` | ok | ok |
| `RETURN <fallible call> TRAP …` | ok | ok |
| self-recursive | ok | ok |

Per HOF, with the identity callback:

| HOF | before | after |
|---|---|---|
| `collections::transform` | **`[exit 139]`** | ok |
| `collections::sortBy` | **`[exit 139]`** | ok |
| `collections::groupBy` | **`[exit 139]`** | ok |
| `collections::mapValues` | ok | ok |
| `collections::reduce` / `reduceRight` | ok (guarded by its own runtime pointer-identity checks) | ok |
| `collections::filter` / `forEach` | ok (`Boolean` / `Nothing`) | ok |

There is a second, quieter pre-fix failure mode worth recording: the identity
callback did not always fault. At 8 two-character elements it ran to completion at
a **flat** 1.0 MB and printed `acc=0` — the freed block read back empty. The
pre-fix flatness *was* the bug.

## §14 memory-semantics

The clause the fix realizes is **§14.3 Function calls and returns**, "Returning a
value moves it into the caller's return slot", under §14's opening invariant that
"each live value is owned by exactly one binding, container slot, temporary,
closure environment, thread message, or return slot". `identish`'s block had two
owners — the HOF's per-iteration temporary and the return slot — and the HOF freed
its one. §14.6 says the same thing at the container: "inserting into a container
copies or moves the inserted value into the container; it never stores a
non-owning alias".

The fix only ever ADDS a copy. It removes no free and changes no value's lifetime
or identity: §14.1 already licenses the compiler to "replace a semantic copy with
a move when it proves the source is not used afterward", and calls that "an
optimization only". Lowering was taking that elision without the proof; the fix
restores the copy the model always specified. A copy is independent by §14.1, and
block identity is not observable from source, so no program can tell.
