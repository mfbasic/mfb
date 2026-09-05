# bug-535: a `RES` bind off a thread channel, with no other call into that package, fails the build with a bare internal error

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `tests/` — new `rt_thread_accept_only_helper` fixture (Phase 1)

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
| Linux / Windows | — | `validate` runs per target; expected identical, confirm in Phase 3 |

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

- [ ] Land `spikes/api-review/bug-535-unused-runtime-helper/` (done).
- [ ] Add the fixture: the worker program above, asserted to build. Confirm it
      fails with the documented message.
- [ ] Test a plain `Bind` with no other package call for **every** builtin
      resource — `fs`, `udp`, `process`, `audio`, `canvas` as well as
      `tcp`/`tls`. Record which fail. The `thread::accept` route only reaches
      the sendable ones, so some may need a different producer to reach.
- [ ] Test a user-declared resource with the same shape.
- [ ] Confirm the stateful form (`RES f AS fs::File STATE Cursor`) is covered by
      the fix design's `STATE` stripping, by testing it.

Acceptance: the fixture fails for the documented reason; the per-resource table
is measured; the stateful case has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Add the plain-resource arm to `used_helpers`, with `STATE` stripped.
- [ ] Verify the "requires undeclared helper" arm still fires — deliberately
      break a module and confirm it is caught. An accounting fix that disables
      the opposite check is a worse bug.

Acceptance: every Phase 1 case builds; the undeclared-helper arm still fires on
a deliberately broken module.
Commit: —

### Phase 3 — the diagnostic frame + validation

- [ ] Give the validator's errors a rule code and a source location, so the
      next genuine failure names the user's program rather than the NIR.
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] Build the spike on Linux and Windows — `validate` runs per target.
- [ ] Update `.ai/codegen-invariants.md` if it records the helper-set rule.

Acceptance: full suite green on all three platforms; the reproduction builds and
runs everywhere.
Commit: —

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
  **Recommend keeping it here.** The opaque message is why this bug reads as a
  compiler crash rather than a program error, and every other failure mode of
  this validator has the same defect.

## Summary

Found while verifying an unrelated documentation item, which is the point: the
shape needed to trigger it is a worker that receives handles and does nothing
else, and nearly every real program calls one more function and escapes. The
fix is a few lines mirroring an arm that already exists for unions — twice
patched for the same class — and the real work is Phase 1's per-resource sweep,
because `tcp` and `tls` are almost certainly not the only two.
