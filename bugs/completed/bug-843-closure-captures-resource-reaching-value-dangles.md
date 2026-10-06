# bug-843: a lambda may capture a record/list that holds a resource; once it escapes, the closure uses a freed resource record (reads/writes another file)

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety (owner-confirmed: closure capture = List append semantics)

Status: Fixed (bs17-agent-f)
Regression Test: tests/rt-behavior/memory/closure-capture-resource-append-rt, tests/syntax/resources/closure-capture-resource-return-order-invalid

**Owner decision (2026-10-05): capture works exactly like `collections::append` of a resource into a List.**
A Resource is a single item that owns its own scope. A `RES` is only an alias to it. A
closure never owns a Resource, and nothing is transferred. Capturing a resource, directly
or through a value that reaches one, copies a `RES` alias into the closure, the same way
appending it to a List copies an alias into the List. If the closure's scope is higher
than the Resource's scope, the Resource's scope is raised to the closure's scope, exactly
as it is raised when the alias is appended to a longer-lived List. Otherwise nothing
changes. The Resource is closed once, at the end of its own (possibly raised) scope,
under the same rules that already apply to a Resource held by a List. Closure copies,
closures stored in collections, closures returned or passed across threads all follow
whatever the List rule already does for the same movement. There is no new rule.

The §14/§15 statements that a closure cannot capture a resource date from when the `RES`
alias owned the scope. They are rewritten to describe capture-as-append, and the resource
case of `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` is removed (diagnostics registry updated). The
regression test is the repro: the escaped closure must operate on its own file and never on
a freed record or an unrelated file. A sibling test does the same through a `List OF RES`
built with `append` to prove the two paths behave identically.

A lambda that captures an ordinary binding whose type *reaches* a resource — a record with a
`RES` field (`h AS H` where `H.f AS RES fs::File`) or a `List OF RES fs::File` — compiles. If
the closure escapes the scope that owns the resource (returned, or returned inside a list), the
owning scope closes the resource and reclaims its record at exit, but the closure environment
still holds the pointer. Calling the closure later dereferences the freed record: immediately
it reports a bogus `ErrResourceMoved` ("moved to another thread by `thread::transfer`" — no
thread exists), and once the arena block is reused by a newly opened file, the closure silently
operates on **that other file** (observed: `fs::size` returns the size of an unrelated file).
This is a use-after-free that can read or write the wrong file descriptor.

Original proposal (superseded by the owner decision above): the capture is rejected at compile time with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`
(as a direct `RES` capture already is), since a captured value that reaches a resource is a
resource capture.

References:

- src/docs/spec/language/14_memory-semantics.md §14.4: "An ordinary closure cannot capture `MUT`
  or resource bindings." §14.8: the compiler must diagnose "Capturing resource handles in
  ordinary closures" and "other non-copyable values in ordinary closures". §14.1: "Functions and
  lambdas are copyable only when their captured environment is copyable."
- src/docs/spec/language/15_resource-management.md §15 (line ~68): "A resource handle cannot be
  printed, compared, serialized, or captured by a lambda or ordinary closure."
- Found during hardening batch B03, topic language/14_memory-semantics.

## Failing Reproduction

```
' src/main.mfb  — mfb init p && mfb build p && ./p/build/p.out
IMPORT io
IMPORT fs
TYPE H
  f AS RES fs::File
END TYPE
FUNC mk() AS FUNC() AS Integer
  RES f AS fs::File = fs::createTempFile()
  LET h AS H = H[f]
  RETURN LAMBDA() -> fs::size(h.f)
END FUNC
FUNC main() AS Integer
  LET g = mk()
  RES other AS fs::File = fs::createTempFile()
  fs::writeAll(other, "0123456789")
  io::print("size via dangling closure = " & toString(g()))
  RETURN 0
END FUNC
```

- Observed (exit 0): `size via dangling closure = 10` — the size of `other`, not of the
  closure's own (empty, already-closed) temp file.
- Expected: compile error `2-203-0019 TYPE_LAMBDA_CAPTURE_UNSUPPORTED` at the `LAMBDA`.

Variants (all in /tmp/hardening/B03/memory/src/):

- `cap_rr2.mfb` — same, no later open: `g(1)` fails `7-703-0009 ErrResourceMoved` (wrong code;
  nothing was transferred — the flag word is read from a reclaimed record).
- `cap_rl5.mfb` / `esc_mapfn.mfb` — capture `xs AS List OF RES fs::File`, return the lambda
  directly or inside `List OF FUNC`: same `ok 10` after churn.
- `cap_write.mfb` — closure reports size 11 of a freshly written victim file.

Contrast (correct today):

- Direct capture `LAMBDA(...) -> fs::size(f)` with `RES f` → `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`
  ("Lambda captures resource local `f`") (`d_capres.mfb`).
- Non-escaping capture of `h` then `fs::close(f)` then calling the lambda in the same scope →
  correct `7-703-0004 ErrResourceClosed` (`cap_rr3.mfb`), because the record is still live.
- Storing the same record into a module-level `MUT` list routes ownership to teardown and works
  (`esc_mod.mfb`, prints 0).

## Root Cause

`src/ir/verify/compat.rs:check_closure_captures` rejects a capture only when
`is_resource_type(type_)` (top-level resource) or `!is_copyable(type_)`.
`src/ir/verify/resources.rs:is_copyable` answers `ParameterType::Res(_) => true` (a RES element
is a copyable non-owning reference, §14.1/§15.6), so a record with a `RES` field and a
`List OF RES T` are "copyable", and neither check fires. The closure environment is not a
container-routing destination in the resource-escape analysis, so the resource's cleanup scope
stays the creating function; its drop closes the handle and reclaims the record
(`src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call`), leaving
the environment's copy dangling.

## Goal

- Capturing a resource in a closure (directly or through a value that reaches one) behaves
  exactly like appending it to a List. An alias is copied in, the Resource's scope is raised
  to the closure's scope when that is higher, and nothing is transferred or owned by the
  closure.
- The repro's escaped closure operates on its own file. No freed record or unrelated file is
  ever reached, and the Resource is closed exactly once at the end of its own scope.
- The spec (§14, §15) and the diagnostics registry describe capture-as-append.

### Non-goals (must NOT change)

- `RES`-element copyability for collections/records (§14.1, §15.6) — only *closure capture*
  changes. Do not "fix" by making `List OF RES` non-copyable globally.
- Do not paper over it by floating the resource into the closure environment silently unless the
  owner chooses that design (it would contradict §15 "cannot be captured").

## Blast Radius

- `check_closure_captures` — fixed by this bug.
- Other escape routes for a resource-reaching value (module-level `MUT`, `RETURN`) — unaffected:
  routed by §15 container rules (verified `esc_mod.mfb`).
- Thread data plane — unaffected: `TYPE_THREAD_RESOURCE_PLANE_REQUIRED` already rejects.

## Fix Design

Add a "reaches a resource" walk (reuse the predicate the thread-plane check uses for resource
reach) to `check_closure_captures`, and emit the resource-capture message naming the reached
resource type. Alternative (rejected unless owner wants it): treat closure capture as a float of
each reached resource into the environment owner — large, and contradicts §15's flat ban.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a compile-fail test for the record-field and `List OF RES` capture shapes; add an rt
      test that the repro no longer builds.
- [ ] Audit every capture shape (record, list, map value, union variant, nested FUNC capturing
      such a value).

Acceptance: tests fail for the documented reason.
Commit: —

### Phase 2 — the fix

- [ ] Extend `check_closure_captures` with the reach walk.

Acceptance: Phase 1 tests pass; `forEach` non-escaping callbacks receiving such values as
*arguments* still compile (§15 table row `forEach([Holder[f]], LAMBDA(h AS Holder) -> ...)`).
Commit: —

### Phase 3 — full validation

- [ ] Full suite; update spec §14.4/§14.8 wording to say "reaches a resource".

Commit: —

## Validation Plan

- Regression test(s): compile-fail fixture above.
- Runtime proof: repro no longer builds.
- Doc sync: §14.4 / §14.8 / §15 line ~68 to name "a value that reaches a resource".
- Full suite: project CI.

## Open Decisions

- Whether a `forEach` direct callback may capture a resource-reaching LET (non-escaping, so safe)
  — recommend allow (mirrors the MUT exception) vs reject uniformly.

## Summary

Silent use-after-free reachable from safe source. The risk is in the capture predicate only; the
fix is a type-walk plus a diagnostic.

## Resolution

Capture follows `collections::append`, as the owner decided. Nothing new was
invented: every movement reuses the rule the `List` path already has.

**Front end.**
- `ir::verify::check_closure_captures` no longer rejects a resource capture; a
  `MUT` capture and a non-copyable non-resource capture are still
  `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`.
- `ir::resource_escape::scan_collection_expr` treats a `LAMBDA` as a container whose
  elements are its captures (`ir::lower::lambda_captured_names`, the list lowering
  builds the environment from). So a resource captured by a closure stored in an
  outer binding floats there, and a returned closure meets the ordering gate exactly
  as a returned list does. An explicitly typed `FUNC` binding counts as a container
  for that gate (the twin of `LET xs AS List OF RES File = [f]; RETURN xs`).
  **The repro is therefore refused with `TYPE_RESOURCE_RETURN_ORDER`** — the
  `List` twin `LET h = H[f]; RETURN [h]` is refused with the same diagnostic
  (measured on main: `l4` probe). A closure over a parameter, or over a container
  declared before the resource, compiles and runs.

**Codegen** (`src/codegen/resource/closure_carry.rs`, module doc). A `FUNC` type
does not say what its closure captured, so across a function boundary the closure
is asked: in a module where some `LAMBDA` captures a resource-reaching value
(`TypeModel::closure_capture_types`), a function value is a *closure leaf* of the
type walk (`resource::walk`), and every environment routine answers four resource
operations — release (RETURN), close-unowned (discarded result), module float
(store into a global), adopt (binding of a call result) — each running the
`List` path's per-resource step over its captures. A holder of function values
gets a dynamic owned-list (`{record, next, kind, tag}` nodes) because its type
cannot name the kinds. A captured resource-union box is copied into the
environment (`ClosureEnvWord::ResourceUnionBox`); the lambda body's binding of a
captured resource is a non-owning alias. A module with no such capture lowers
byte-identically (artifact gate: 0 diffs).

**Found and fixed on the way** (own commit): `simplifycfg` collapsed a loop label
onto the function's entry label, so `_mfb_module_resource_drain` re-ran its
prologue on every node and every `-O2`/`-O3` program that stores a resource in a
global hung at exit (pre-existing on main).

### Runtime proof

- Fixture `closure-capture-resource-append-rt`, every closure movement beside its
  `List` twin; identical output at `-O0/-O1/-O2/-O3` (md5 of stdout equal across
  levels). The record-capture case alone, built by main, dies with a bogus
  `7-703-0009 ErrResourceMoved`; the debugger (`be any`, `bt`, `show error`) stops
  in `put` called from `lambda`, reading the flag word of a reclaimed record. After
  the fix, `bp put` + `show mem x0 48` shows the closure's record live
  (`closed = 0`) with its owner word raised to the enclosing function's scope token
  (depth 1 instead of the inner block's 2).
- `--debug -O0`: `double_free_skips 0`. Per-iteration live-byte growth (100 vs 200
  iterations) is 112 B for each closure case — the 96-byte record tombstone a
  raised resource leaves, as a `List`-held resource does — against 4,208–4,448 B
  for the `List` twins on main itself (pre-existing; see below).
- Matrix probes (two levels of return, a closure capturing a closure, a closure in a
  returned record, `forEach` capture, `Map OF K TO FUNC`, inline `TRAP`, a closure
  stored into a global through a parameter, resource union, thread handle, closures
  built in a loop, a capture closed explicitly → `ErrResourceClosed` 77030004):
  all match their `List` twins.

### Known gaps shared with the `List` path (not regressions)

- A `List`/closure adopted through an inline `TRAP` (`LET xs = mk() TRAP ...`) is
  never adopted, so its resource is never closed (fd leak) — same on main for a
  `List OF RES`. Taken up under bug-842 (unclaimed temporaries).
- A floated or adopted `List OF RES` element leaks its I/O buffer and record
  (~4.3 KB per resource, measured on main); a resource-union binding's drop leaks
  its variant's I/O buffer (4,096 B per drop, measured on main). Closures avoid the
  buffer leak (their drain reclaims buffers and `STATE`) and keep only the record
  tombstone.

## Golden impact

- `tests/syntax/functions/lambda-capture-invalid/golden/build.log` and
  `tests/syntax/functions/lambda-mut-capture-invalid/golden/build.log`: each loses
  its `Lambda captures resource local ...` diagnostic block (the rule the owner
  retired). Measured: `scripts/test-accept.sh <mfb> <out> '*lambda*' '*closure*'
  '*resource*' '*capture*'` → 150 tests, exactly these 2 mismatches. The
  `lambda-mut-capture-invalid` source comment that claimed resources are never
  capturable was corrected without moving any line.
- `scripts/artifact-gate.sh <mfb> all` → 1629 tests, 2282 goldens, 0 diffs (codegen
  for modules without resource-reaching captures is unchanged, and no committed
  `-O2` golden hit the simplifycfg case).
