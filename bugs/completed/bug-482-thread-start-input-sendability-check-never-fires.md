# bug-482: `thread::start`'s input-sendability check never fires — a capturing lambda crosses the thread boundary and runs on the worker

Last updated: 2026-09-01
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed 2026-09-01
Regression Test: `tests/syntax/threads/thread-start-input-not-sendable/`

`thread::start(f, data)` is required to reject a `data` argument whose type is
not thread-sendable — spec §16 lists functions, lambdas, `Thread`, `ThreadWorker`
and opaque resource handles as not sendable by default, and
`src/ir/verify/resources.rs:486` has the check that enforces it
(`require_thread_sendable(&format!("Call to \`{display}\` input"), input)`,
line 598).

**The check never fires.** Every non-sendable `In` tested below compiles clean
and links. The worst case builds and *runs*: a **capturing** lambda is passed
across the boundary and invoked on the worker thread, where it dereferences a
closure environment that lives in the **parent's** arena. Arena state is
per-thread — a spawned thread gets its own block — so this is a cross-arena read
that happens to survive only because the parent is parked in `thread::waitFor`
and has not reclaimed the memory. It is a use-after-free waiting for a schedule
where the parent finishes first or the arena is reused.

**The single correct behavior a fix produces:** `thread::start(f, data)` emits
`TYPE_THREAD_NOT_SENDABLE` whenever `data`'s type is not thread-sendable per
`is_thread_sendable` (`src/ir/verify/resources.rs:416`), on every entry path —
imported-package entry and same-project entry alike.

This is silent: there is no diagnostic, no warning, and the program produces the
*expected* answer on the happy path, so nothing signals that a boundary rule was
skipped.

References:

- `mfb spec language threads` §16 — "Thread boundary types must be
  thread-sendable. … Functions, lambdas, `Thread`, `ThreadWorker`, and opaque
  resource handles are not sendable by default."
- `src/ir/verify/resources.rs:416` `is_thread_sendable` — `ParameterType::Func(..)
  | ParameterType::ThreadHandle { .. } => false` (line 440).
- `.ai/net-tls.md`, `.ai/codegen-invariants.md` — arena state is per-thread.
- Found while scoping `planning/plan-115-A-unified-thread-entry.md`, which must
  modify the `imported_entry` gate this bug sits behind. **plan-115 lists this
  bug as a prerequisite.**

## Failing Reproduction

Two projects: a worker package `wpkg`, and an executable that starts it.

```
mkdir -p /tmp/b482/wpkg/src /tmp/b482/consumer/src /tmp/b482/consumer/packages
```

`/tmp/b482/wpkg/project.json`:

```json
{
  "name": "wpkg", "version": "0.1.0", "mfb": "1.0", "kind": "package",
  "description": "sendability repro",
  "sources": [{ "root": "src", "role": "lib", "include": ["**/*.mfb"] }],
  "targets": ["native"]
}
```

`/tmp/b482/wpkg/src/lib.mfb` — two entries whose `In` is non-sendable:

```basic
EXPORT ISOLATED FUNC w(t AS ThreadWorker OF Nothing TO Integer, f AS FUNC() AS Integer) AS Integer
  RETURN f()
END FUNC
EXPORT ISOLATED FUNC wh(t AS ThreadWorker OF Nothing TO Integer, h AS Thread OF Nothing TO Integer) AS Integer
  RETURN 1
END FUNC
```

`/tmp/b482/consumer/project.json` declares `wpkg` via
`"source": "file:packages/wpkg.mfp"`, `kind: "executable"`, entry `main`.

**Case 1 — a capturing lambda crosses the boundary and runs on the worker:**

```basic
IMPORT thread
IMPORT io
IMPORT wpkg

FUNC main AS Integer
  LET captured AS Integer = 42
  LET t AS Thread OF Nothing TO Integer = thread::start(wpkg::w, LAMBDA() -> captured + 1)
  LET r AS Integer = thread::waitFor(t)
  io::print("worker returned " & toString(r) & " (expected 43)")
  RETURN 0
END FUNC
```

```
$ mfb build wpkg && cp wpkg/wpkg.mfp consumer/packages/ && mfb build consumer
Wrote executable to consumer/build/consumer.out
[exit 0]
$ ./consumer/build/consumer.out
worker returned 43 (expected 43)
[exit 0]
```

- Observed: builds clean, exit 0; the worker thread invokes the parent's closure
  and reads its captured environment across the arena boundary.
- Expected: `error[… TYPE_THREAD_NOT_SENDABLE]: Call to \`thread.start\` input
  requires a thread-sendable type, got \`FUNC() AS Integer\`.`

**Case 2 — a `Thread` handle as the data argument** (unambiguously
`ParameterType::ThreadHandle`, and unambiguously inferable from the `LET`
annotation, so no type-inference excuse applies):

```basic
IMPORT thread
IMPORT wpkg

FUNC main AS Integer
  LET a AS Thread OF Nothing TO Integer = thread::start(wpkg::w, LAMBDA() -> 7)
  LET b AS Thread OF Nothing TO Integer = thread::start(wpkg::wh, a)
  RETURN thread::waitFor(b)
END FUNC
```

- Observed: `mfb build consumer` → `Wrote executable`, `[exit 0]`.
- Expected: `TYPE_THREAD_NOT_SENDABLE` naming `Thread OF Nothing TO Integer`.

**Case 3 — a non-capturing lambda / a bare function value** (`thread::start(wpkg::w,
helper)` where `PUBLIC FUNC helper AS Integer`): also accepted, also exit 0.
Less dangerous (a no-capture closure descriptor has `env = 0`) but equally
un-diagnosed.

### Contrast cases that DO work

These bound the bug — the sibling sendability checks are live, so the failure is
specific to the `In` argument, not to sendability enforcement generally:

| Check | Site | Fires? |
| --- | --- | --- |
| `Thread` handle's `Msg`/`Out` planes, at the declared type | `resources.rs:528-529` | ✓ (type-driven walk, not call-gated) |
| `thread::send` message type | `resources.rs:608` | ✓ |
| `thread::transfer`/`accept` resource plane + `STATE` | `resources.rs:643,657` | ✓ |
| **`thread::start` input (`In`)** | **`resources.rs:598`** | **✗ never** |

The `In` type is the one boundary type that appears *only* in the entry
function's signature and never in the `Thread OF Msg TO Out` handle type, so the
type-driven walk at `resources.rs:520-548` cannot cover it. Line 598 is its only
guard, and line 598 is unreachable in practice.

### Reproduction re-confirmed 2026-09-01 at `781a82f07`

`/follow-plan 115` re-ran Case 1 against a `target/release/mfb` rebuilt from
main's tip, because plan-115 gates on this bug and a stale report would have
been a false gate. It still reproduces verbatim — build exit 0, no diagnostic,
and the capturing lambda runs on the worker:

```
$ ./target/release/mfb build /tmp/b482/wpkg
Wrote package to /tmp/b482/wpkg/wpkg.mfp
$ ./target/release/mfb build /tmp/b482/consumer
Wrote executable to /tmp/b482/consumer/build/consumer.out
[exit 0]
$ /tmp/b482/consumer/build/consumer.out
worker returned 43 (expected 43)
[exit 0]
```

(Note for the repro-follower: the consumer's `project.json` declares the
dependency under `"packages"`, not `"dependencies"` — the latter yields
`IMPORT_PACKAGE_NOT_DECLARED` before the boundary rules are ever reached.)

Corroborating static evidence that the `In` guard has never fired:
`grep -rn "Call to .thread.start. input" tests/ | wc -l` → **0**. No golden in
the corpus carries that message, while the sibling type-driven walk's
"Thread message type requires …" is pinned in
`tests/syntax/threads/func_thread_start_invalid/golden/build.log`.

The `imported_entry` early-return named under H1 is still verbatim present, now
at `src/ir/verify/resources.rs:691-696` (the line numbers in this report have
drifted from 588-595). H1 vs H2 remains unconfirmed — the symptom was
reproduced, not instrumented.

## Root Cause — CONFIRMED 2026-09-01: **both** H1 and H2, in series

Instrumented on current main. Neither hypothesis alone explains the bug, and
fixing either alone leaves it effectively unfixed — which is why the report
below could not choose between them.

**H1 is true, and it kills the rule outright.** Probe at the gate:

```
B482-PROBE first="04187eb9abe9cc82.wpkg.w" in_functions=true imported_entry=false
           fnkeys=["$lambda0", "04187eb9abe9cc82.wpkg.w", "main"]
```

`TypeEnv::build` (`src/ir/verify/mod.rs:737-753`) inserts **every** function of
`IrProject::functions`, and an imported package's functions are lowered into
that same list under the package-keyed name the entry's `FunctionRef` carries.
So the gate's premise — "absent from `functions` ⇒ came from an import" — is
exactly backwards, `imported_entry` is `false` for an imported entry and a
same-project one alike, and the whole arm returns early. Everything behind it
was dead.

**H2 is also true, and it is why H1's fix is not enough.** Probe at the
`return_type` guard, on the same repro, showing both verify passes:

```
   source pass: file="src/main.mfb" line=18 return_type=None
                arg1=Some("FUNC() AS Integer")
   native pass: file="src/main.mfb" line=18 return_type=Some("Thread OF Nothing TO Integer")
                arg1=Some("FUNC() AS Integer")
```

There are two verify passes: `ir::verify_source_diagnostics` on the source-lowered
IR (`src/cli/build/mod.rs:612`) and `ir::verify_semantics` on the **merged** IR
(`src/target/shared/nir/lower.rs:110`). In the *source* pass `infer_type(call)`
is `None` — that pass does not hold the imported entry's signature — so
`let Some(return_type) = … else { return }` returns **before** the `In` rule,
even with the gate gone. Only the merged/native pass reached it.

**Why that distinction is not academic.** The native pass reports through
`verify_semantics`'s `Result` channel, which carries no span, so the diagnostic
renders unlocated (`error: TYPE_THREAD_NOT_SENDABLE: …`, no file, no rule code) —
and that pass is skipped entirely when any dump flag is present. Measured on the
fixture before the H2 half of the fix:

| invocation | exit | errors |
| --- | --- | --- |
| `mfb build <fixture>` | 1 | 1 (unlocated) |
| `mfb build -ast <fixture>` | 0 | 0 |
| `mfb build -ir <fixture>` | 0 | 0 |
| `mfb build -ast -ir <fixture>` | 0 | 0 |

`scripts/test-accept.sh` builds every `tests/syntax/` fixture with exactly
`-ast -ir` (`scripts/test-accept.sh:518`, `console_flags="-ast -ir"`). **A rule
that fires only in the native pass therefore cannot be pinned by a syntax
golden at all** — the regression test would have recorded `[exit 0]` and no
diagnostic, and passed forever while the hole stayed open.

## Fix (landed)

Two edits in `check_thread_boundary_sendability`:

1. **Delete the `imported_entry` gate.** The entry's provenance has no bearing on
   whether `data` crosses the boundary safely, so there is nothing to gate on.
2. **Hoist the `In` rule above the `return_type` guard.** The `data` argument's
   sendability is a property of the argument alone; the call's own type says
   nothing about it. The handle-plane rules (message/resource/output) genuinely
   need `return_type` and stay below the guard.

`is_thread_sendable` is unchanged, as the report requires.

After the fix, under the harness's own `-ast -ir` invocation, all four
non-sendable shapes are rejected at their call sites with `2-203-0063`, and the
sendable control case (`In = Integer`) is not — proving the rule narrowed to
non-sendable types rather than swallowing every `thread::start`.

## Original hypotheses (kept for the record)

Both sit in `check_thread_boundary_sendability`
(`src/ir/verify/resources.rs:561`).

**H1 (likely) — the `imported_entry` gate is never true.**

```rust
let imported_entry = matches!(
    args.first(),
    Some(IrValue::FunctionRef { name, .. }) if !self.functions.contains_key(name)
);
if !imported_entry {
    return;
}
```
`src/ir/verify/resources.rs:588-595`

The gate assumes an imported entry's `FunctionRef` name is absent from
`self.functions`. If imported signatures are also installed into that map (or
the name is canonicalized to a bare form before this point — cf.
`src/ir/lower.rs:3047`, which rewrites `self::worker` to bare `worker`), the gate
is false for *every* entry and the function returns before reaching line 598.
Confirm by logging `imported_entry` and `name` for Case 2.

**H2 — the early return above it.**

```rust
let Some(return_type) = self.infer_type(call, locals) else { return; };
if matches!(return_type, ParameterType::Unknown) { return; }
```
`src/ir/verify/resources.rs:576-579`

Case 2 annotates the binding (`LET b AS Thread OF Nothing TO Integer`), so the
call's type should be known — which makes H2 less likely, but it is not
eliminated. Confirm by logging `return_type` for Case 2.

Whichever holds, note the comment at `resources.rs:586-588` — *"a local function
or a lambda was already rejected as the argument"* — is a stale premise: it
describes the **entry** argument, not the **data** argument, and it is being used
to skip the data check.

## Fix Sketch

1. Confirm H1 vs H2 by instrumenting `check_thread_boundary_sendability` and
   rebuilding Case 2.
2. Make the `In` check unconditional on the entry's provenance. The entry being
   imported or same-project has no bearing on whether `data` is sendable — the
   value crosses the same boundary either way. Deleting the `imported_entry`
   early-return is the likely correct shape; if it guards something else that
   genuinely needs it, narrow it to that instead of gating the whole arm.
3. Ensure the check runs for an un-annotated call too (`thread::start(f, x)` with
   no `LET … AS`), or record explicitly why an underived call type is permissive.

Do **not** widen `is_thread_sendable`. `Func` and `ThreadHandle` are correctly
non-sendable; the defect is that nothing consults it for `In`.

## Regression Test

`tests/syntax/threads/thread-start-input-not-sendable/` — a syntax fixture
pinning the diagnostic for all three shapes (capturing lambda, non-capturing
lambda, `Thread` handle), each expecting `TYPE_THREAD_NOT_SENDABLE`.

Add a same-project-entry variant once `plan-115-A` lands, so the check is pinned
on both entry paths.

**Do not pin this as an rt-behavior test.** The current behavior is a build that
succeeds; the fixed behavior is a build that fails, and a golden that pins a
build failure in `rt-behavior/` is a dead fixture (the harness compares failure
to failure and reports PASS). Only `tests/syntax/` may pin a diagnostic.

## Validation

- `cargo test --no-fail-fast` — full suite; `verify` tests are the ones that move.
- `scripts/test-accept.sh` — the new syntax fixture plus any thread fixture whose
  `golden/build.log` now carries the diagnostic.
- Runtime proof that the hole is closed: Case 1 must fail to build. There is no
  runtime proof of the *fixed* state beyond that, because the fix's whole point
  is that the program no longer exists.
- Audit for other callers of the same shape: `grep -n "imported_entry" src/` and
  re-read each early-return in `check_thread_boundary_sendability` for the same
  stale-premise pattern.

### Results (2026-09-01)

| Gate | Result |
| --- | --- |
| `scripts/test-accept.sh ./target/release/mfb <scratch>` | **passed, 1345 ran, 0 mismatches** |
| `cargo check --all-targets` | clean — no warnings, no errors |
| Case 1 (`/tmp/b482/consumer`) | **exit 1**, `…/src/main.mfb:7 error[2-203-0063 TYPE_THREAD_NOT_SENDABLE]: … Call to \`thread.start\` input requires a thread-sendable type, got \`FUNC() AS Integer\`.` |
| `grep -n "imported_entry" src/` | 0 hits — the stale-premise gate is gone, not narrowed |

**No behavioral golden was edited.** An intermediate version of this fix deleted
the gate wholesale, which resurrected the three redundant handle-plane rules and
drifted exactly one golden (`func_thread_start_invalid/build.log`) by
double-reporting line 18. That was a defect in the fix, not a stale golden:
removing the redundant rules (they are covered by the declared-type walk)
returned that golden to byte-identical and left the new fixture as the only
test change.

### A trap this bug leaves behind for future diagnostic work

`scripts/test-accept.sh` builds every `tests/syntax/` fixture with `-ast -ir`
(`console_flags="-ast -ir"`, `scripts/test-accept.sh:518`), and a dump build
**returns before the native pipeline**. A rule that fires only in
`ir::verify_semantics` (the merged/native pass, `src/target/shared/nir/lower.rs:110`)
is therefore invisible to the syntax golden harness *and* reports without a
span. When adding a diagnostic, confirm it fires from
`ir::verify_source_diagnostics` — compare `mfb build <fixture>` against
`mfb build -ast -ir <fixture>` and require the same exit code.
