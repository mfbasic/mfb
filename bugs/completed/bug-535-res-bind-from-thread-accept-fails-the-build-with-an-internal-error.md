# bug-535: a `RES` bind off a thread channel, with no other call into that package, fails the build with a bare internal error

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed (`b93de7ed0`)
Regression Test: `src/target/shared/validate/mod.rs` unit tests,
`tests/cli_thread_accept_res_bind.rs`, `tests/rt_thread_accept_res_drop_closes.rs`

A legal MFBASIC program fails to build with a message about the compiler's own
intermediate representation:

```
error: NIR declares unused runtime helper 'tcp'
```

No error code. No source location. No diagnostic frame. Just a sentence naming
an internal data structure the author has never heard of.

The program is a worker that accepts a socket off a thread's resource channel
and lets the `RES` binding drop:

```
ISOLATED FUNC worker(t AS ThreadWorker OF RES tcp::Socket TO Integer, n AS Integer) AS Integer
  RES s AS tcp::Socket = thread::accept(t, 1000)
  RETURN 1
END FUNC
```

The same shape fails for `tls`. It is exactly the pattern the `thread` package
intro recommends — "a server may accept on one thread and hand each connection
to a worker" — reduced to the worker side.

Adding **any** other call into the package (`tcp::listen`, `tcp::read`) makes it
build again, which is why this has survived: almost every realistic program has
one. The failure is reserved for the minimal case — a worker that only receives
handles and does not otherwise touch the package — and for anyone reducing a
program while debugging.

The single correct behavior a fix produces: the program compiles. A resource
whose only use is a scope-exit drop still counts as a use of its package's
runtime helper.

References:

- `src/target/shared/validate/mod.rs:111` — the error
- `src/target/shared/validate/mod.rs:57-97` — `used_helpers` collection
- `src/target/shared/validate/capabilities.rs:55-75` — `collect_bind_types`,
  which already carries two prior patches for the same class (plan-74, bug-328)
- Spike: `spikes/api-review/bug-535-unused-runtime-helper/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-535-unused-runtime-helper
```

- Observed (macOS aarch64, release):

```
Building mfb_project (executable) for macos-aarch64
error: NIR declares unused runtime helper 'tcp'
```

- Expected: `Wrote executable to …/build/mfb_project.out`.

The narrowing, all measured:

| program | result |
| --- | --- |
| `RES s AS tcp::Socket = thread::accept(t, 1000)` in the worker, no other `tcp::` call | **fails** ✗ |
| same, with `tls` instead of `tcp` | **fails** ✗ (`unused runtime helper 'tls'`) |
| same worker body removed — only the `Thread OF RES tcp::Socket` type declared | builds ✓ |
| same, plus a `tcp::listen` call anywhere in the program | builds ✓ |
| `MUT s AS List OF RES tls::Socket = []` with no `tls::` call | builds ✓ |

So the trigger is specifically a **`Bind` of a plain built-in resource** whose
only consumer is the codegen-emitted scope drop.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| linux-x86_64 / linux-aarch64 / linux-riscv64 / windows-x86_64 | cross-build, release | fails ✗ — identical message on all four (measured) |

## Root Cause

`src/target/shared/validate/mod.rs` builds a `used_helpers` set and then
requires it to equal the module's declared `runtime_helpers`, in both
directions:

```rust
for helper in &module.runtime_helpers {
    if !used_helpers.contains(helper) {
        return Err(format!("NIR declares unused runtime helper '{}'", helper.name()));
    }
}
```

`used_helpers` is populated from two sources:

1. `validate_function` — helpers reachable from an explicit **NIR call**.
2. A special case for **resource-union** binds
   (`mod.rs:71-97`): for each `Bind` whose type is a union, each variant's close
   op is counted as a used helper. The loop is gated on
   `if type_.kind != "union" || !bind_types.contains(&type_.name) { continue; }`.

A `RES s AS tcp::Socket = thread::accept(...)` is a `Bind` of a **plain**
built-in resource. Its close op (`tcp.close`) is emitted by codegen at scope
exit, not as an NIR call — the same reason the union case needed its special
case — but the plain case has no equivalent arm. The `tcp` helper is declared
(the module needs it to emit that close) and never counted as used, so the
"unused" branch fires.

This is the third instance of one bug class. `capabilities.rs:55-75` carries the
scar tissue:

- bug-328 — `collect_bind_types` had to descend the shared NIR seam.
- plan-74 — a stateful resource-union bind (`Stream STATE Cursor`) had to have
  its `STATE` suffix stripped, "or a valid stateful union bind trips the
  'declares unused runtime helper' guard."

Both fixes widened the *union* path. The plain-resource path was never covered
because, until a `Bind` could arrive from `thread::accept` with no other call
into the package, there was no way to reach it.

## Goal

- The reproduction builds and runs.
- A plain built-in resource `Bind` counts its close op's helper as used, exactly
  as a resource-union bind already does.
- If the check does fire for a genuine reason, it reports as a diagnostic with a
  rule code and a source location, not as a bare `error:` string.

### Non-goals (must NOT change)

- The check itself. Requiring declared and used helpers to agree is a real
  invariant — the opposite arm, "NIR runtime call requires undeclared helper",
  catches a link failure — and must keep firing.
- The union path, or the plan-74 `STATE`-stripping. Both are correct and are
  the model for the fix.
- `thread::accept`'s behavior.
- **Tempting wrong fix, forbidden:** dropping the "declares unused" arm so the
  program builds. It is the arm that catches a helper declared and then
  optimized away, which is a real defect class. The set is under-counted, not
  over-strict.
- **Also forbidden:** special-casing `thread::accept`. `accept` is where this
  was *found*; the defect is that a plain resource `Bind` is not counted, and
  any other producer of one has the same hole.

## Blast Radius

- `src/target/shared/validate/mod.rs` — `used_helpers` collection; fixed here.
- `src/target/shared/validate/capabilities.rs:collect_bind_types` — collects
  `Bind` types already; likely reusable, since it walks every `Bind` and only
  the *filter* downstream is union-only.
- **Every builtin resource, not just `tcp`/`tls`.** `fs::File`, `udp::Socket`,
  `process::Process`, `audio::*`, `canvas::*` all have codegen-emitted close
  ops. Phase 1 must test a plain `Bind` of each with no other call into its
  package; the two confirmed failures are `tcp` and `tls` only because those are
  what `thread::accept` can produce today.
- **User-declared resources** (`RESOURCE … THREAD_SENDABLE`) — same question,
  different close path. Must be checked, not assumed.
- The error's *presentation* — `mod.rs` returns a `String` that surfaces as a
  bare `error:`. Every failure mode of this validator has the same problem, so
  whatever fires next will be equally opaque. Worth fixing alongside.
- `.ai/codegen-invariants.md` — records NIR invariants; check whether the
  helper-set rule is documented there and update it.

## Fix Design

Add the plain-resource arm alongside the union arm in `mod.rs`. For each `Bind`
type collected by `collect_bind_types`, if the type resolves to a built-in
resource, look up its close op with
`crate::codegen::builtins::resource_close_function` and add that helper to
`used_helpers` — the identical shape the union arm already uses per variant,
minus the variant loop.

The `STATE` suffix must be stripped first, exactly as plan-74 does for the union
case; a `RES f AS fs::File STATE Cursor` bind names the same resource and would
otherwise miss the lookup and reintroduce the bug in its stateful form.

The correctness risk is over-counting: adding a helper to `used_helpers` that
the module does not declare would flip the *other* arm ("requires undeclared
helper") and turn this bug into its mirror image. The new arm must therefore
mirror how codegen actually decides to emit the drop — which is the same
`resource_close_function` lookup — rather than adding a helper for any
resource-shaped type.

Separately, give the validator's errors a diagnostic frame. Every `return
Err(format!(...))` in this file produces a message with no code and no location.
That is a defect in its own right, and this bug is the evidence: the message a
user actually saw named an internal structure and nothing about their program.

Rejected: counting *declared* helpers as used when the module contains any
resource of that package. Too loose — it would mask a genuinely unused helper,
which is what the arm exists to catch.

Rejected: making codegen emit an explicit NIR call for a scope-exit drop. It
would fix the accounting by changing the IR, moving a much larger risk (drop
ordering, arena interaction) for a validator bookkeeping bug.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Land `spikes/api-review/bug-535-unused-runtime-helper/` (done).
- [x] Add the fixture: `tests/cli_thread_accept_res_bind.rs`. Confirmed failing
      for the documented reason on the pre-fix compiler.
- [x] Test a plain `Bind` with no other package call for every builtin resource.
      **Measured table below.**
- [x] Test a user-declared resource with the same shape.
- [x] Confirm the stateful form is covered — it is, and it was failing too.

The per-resource sweep, measured against the pre-fix `target/release/mfb`:

| bind | pre-fix | post-fix |
| --- | --- | --- |
| `RES s AS tcp::Socket = thread::accept(t, 1000)` | `unused runtime helper 'tcp'` ✗ | builds ✓ |
| `tcp::Listener` | `… 'tcp'` ✗ | builds ✓ |
| `tls::Socket` | `… 'tls'` ✗ | builds ✓ |
| `tls::Listener` | `… 'tls'` ✗ | builds ✓ |
| `udp::Socket` | `… 'udp'` ✗ | builds ✓ |
| `fs::File` | `… 'fs'` ✗ | builds ✓ |
| `RES f AS fs::File STATE Progress` | `… 'fs'` ✗ | builds ✓ |
| alias-only `RES g AS fs::File = f` (no `fs::` call) | builds ✓ | builds ✓ (the over-count pin) |

`audio`, `canvas` and `process` are **not reachable**: their resources are not
`THREAD_SENDABLE`, so `thread::accept` cannot produce one, and no other producer
of a plain resource bind exists that does not also call into the package. They
are covered by construction once the arm is type-driven rather than
package-driven, and by the `every_sendable_builtin_resource_bind_counts_its_helper`
unit test's shape.

A **user-declared** `RESOURCE … THREAD_SENDABLE` off `thread::accept` fails for a
DIFFERENT reason — `native inlined field size not available for type 'Db'` — at
native lowering, not in the helper accounting (a user resource registers no
runtime helper at all, so it cannot reach this check). Filed as **bug-546**;
unchanged by this fix.

A second incidental find, also unchanged by this fix and also hidden by "any
other call into the package": an alias rebind of a `tcp`/`udp` socket fails with
`data relocation target '_mfb_str_error_resource_closed' is not a data object`.
Filed as **bug-545**.

Acceptance: met.
Commit: `b93de7ed0`

### Phase 2 — the fix

- [x] Add the plain-resource arm to `used_helpers`, with `STATE` handled.
      `resource_close_function` peels `STATE` itself
      (`builtin_resource_close_function` resolves `type_.without_state()`), so no
      second textual strip was needed — pinned by
      `a_stateful_plain_resource_bind_counts_the_same_helper`, which asserts the
      fixture really does carry a `STATE` clause.
- [x] Verify the "requires undeclared helper" arm still fires — pinned twice:
      `a_resource_bind_whose_helper_is_undeclared_is_still_rejected` (on the new
      path specifically) and the pre-existing `rejects_undeclared_runtime_helper`.
      The "declares unused" arm is pinned by
      `a_genuinely_unused_helper_is_still_rejected`.

The over-count risk the Fix Design named is handled by giving the new collector
the **declarer's** aliasing gate (bug-375: a bind naming an already-live resource
closes nothing). Used and declared are compared against each other, so the used
side must recognize no MORE shapes than `runtime::required_helpers` — hence the
private NIR twin of `runtime::usage::value_aliases_live_resource` rather than
`CodeBuilder::value_aliases_live_resource`, which knows three further shapes and
would raise "requires undeclared helper" on programs that build today. Pinned by
`an_aliasing_resource_bind_counts_no_helper` and by the alias-only build test.

Acceptance: met.
Commit: `b93de7ed0`

### Phase 3 — validation

- [ ] **Deferred, filed separately.** The diagnostic frame is a bigger change
      than it looks: `validate_nir` returns `Result<(), String>` and that `String`
      is `?`-propagated through every one of the five backends' build entry
      points. Every failure mode of the validator is a compiler-internal
      invariant violation with no `NirOp::Bind` source location to attach (the op
      carries no `loc`), so the honest fix is a module-level diagnostic frame for
      the whole file plus a return-type change across all five backends — a
      refactor with its own golden risk, not a rider on an accounting fix. The
      program in this bug is VALID, so the check no longer fires on it at all.
- [x] `cargo test --release --no-fail-fast`.
- [x] Built the reproduction for `linux-x86_64`, `linux-aarch64`,
      `linux-riscv64` and `windows-x86_64` as well as the macOS host: all four
      failed pre-fix with the identical message and all four build post-fix.
- [x] `.ai/resources-packages.md` updated — it already carried the union half of
      this rule as a three-place list; the plain-resource half is now recorded
      beside it, with the "the used side must recognize no more shapes than the
      declarer" invariant.

Acceptance: met.
Commit: `b93de7ed0`

## Validation Plan

- Regression test: the worker fixture, plus one per resource that Phase 1 found
  failing, plus the stateful variant.
- Runtime proof: `spikes/api-review/bug-535-unused-runtime-helper/` building and
  printing `started`.
- Negative proof: a module with a genuinely unused helper still rejected.
- Doc sync: `.ai/codegen-invariants.md`; no man-page change (this is a compiler
  bug, not a surface one).
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`, on macOS,
  Linux and Windows.

## Open Decisions

- Whether the diagnostic-frame work (Phase 3) belongs here or in its own bug.
  **Resolved: its own bug.** The recommendation to keep it here underestimated
  it. `validate_nir` returns a `String` that is `?`-propagated through all five
  backends, and `NirOp::Bind` carries no source location, so there is nothing to
  point at even once the plumbing exists — the frame would have to be
  module-level and the change is a five-backend return-type refactor. Every
  failure mode of this file is an internal invariant violation rather than a
  user error, which is what makes it a coherent separate piece of work instead
  of a rider on an accounting fix.

## Summary

Found while verifying an unrelated documentation item, which is the point: the
shape needed to trigger it is a worker that receives handles and does nothing
else, and nearly every real program calls one more function and escapes. The
fix is a few lines mirroring an arm that already exists for unions — twice
patched for the same class — and the real work was Phase 1's per-resource sweep,
because `tcp` and `tls` were not the only two: all six sendable resources and the
stateful spelling failed, on all five targets. The sweep also turned up two
neighbouring defects hidden by the same "any other call into the package"
condition, filed as bug-545 and bug-546.
