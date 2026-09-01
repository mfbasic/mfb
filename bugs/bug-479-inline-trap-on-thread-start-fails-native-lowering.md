# bug-479: an inline `TRAP` on `thread::start` fails native lowering with an unlocated error

Last updated: 2026-08-31
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/rt-behavior/threads/thread-start-inline-trap-rt/` (new, Phase 1)

Attaching an inline `TRAP` to `thread::start` — the idiom `mfb man errors`
teaches for handling one call's failure at the call site — fails the build:

```
error: native inlined field size not available for type 'ThreadWorker OF String TO Integer' while lowering bind $trap_res0 AS Result OF Thread OF String TO Integer
```

The inline `TRAP` desugars to a `bind $trap_resN AS Result OF Thread OF …`
(`mfb man errors` → "Local handling with an inline TRAP"), and the `Result`
marshaller has no arm for a thread handle in its payload, so it falls through to
the "no inlined field size" error. It is **unlocated** — no file, no line, no
rule code — so the build output does not say which statement is at fault.

`thread::start` is fallible (`ErrResourceExhausted` when a thread cannot be
created), so this is not an exotic shape: it is the documented way to handle the
documented failure of a documented call, and it does not compile.

**The single correct behavior a fix produces:** an inline `TRAP` on
`thread::start` compiles and behaves like an inline `TRAP` on any other fallible
call — the success value binds to the `Thread` handle and the handler runs on
failure — for every channel shape (`Thread OF Msg TO Out`,
`Thread OF RES Res TO Out`, `Thread OF Msg RES Res TO Out`).

## Lead: the error names a DIFFERENT type than the bind (2026-08-31, coordinator)

Re-reproduced pre-dispatch on a second channel shape, which the doc's single
example (`Thread OF String TO Integer`) does not cover. A ready-made repro
project is staged at `/tmp/mfb59-r479` (an executable importing a package with
an `EXPORT ISOLATED FUNC`). **Path note (plan-115-C):** that repro originally
took its worker from `examples/network-server/worker`, a package that no longer
exists — plan-115 made any `ISOLATED FUNC` a thread entry, so the example was
collapsed into one project and its entries now live in
`examples/network-server/src/wire.mfb` as `PUBLIC ISOLATED FUNC`. For an
*imported-package* worker with a `RES tcp::Socket` channel, use
`tools/thread-package-sources/*` or declare one locally; the bug is about the
inline `TRAP` lowering, not about where the entry comes from.

```
error: native inlined field size not available for type
       'ThreadWorker OF RES tcp.Socket TO Integer'
       while lowering bind $trap_res0 AS Result OF Thread OF RES tcp.Socket TO Integer
```

So the defect spans channel shapes — it is not specific to `Thread OF Msg TO Out`.

**The part worth looking at before writing any fix:** the two types in that
message are **not the same type**. The bind is `Result OF **Thread** OF …`; the
size lookup that fails is for `**ThreadWorker** OF …`. `src/codegen/registry/mod.rs:2145`
states the two never interchange — *"kind (parent `Thread` vs worker
`ThreadWorker`) must match"*. `thread::start`'s own signature is
`ISOLATED FUNC(ThreadWorker OF Msg RES Res TO Out, In) AS Out` (`registry/mod.rs:3951`),
so a `ThreadWorker` spelling is in scope at this call — plausibly the entry
function's *parameter* type is what reaches the marshaller, rather than the
`Thread` the `Result` actually carries.

That distinction decides the fix, and it is exactly the bait this document's
References already warn about by pointing at bug-464 ("Read its Root Cause
before assuming a size constant is all this needs"):

* If the marshaller is correctly asked about the `Result`'s payload and simply
  lacks an arm, adding the `Thread` arm is right.
* **If it is being asked about `ThreadWorker` when the payload is `Thread`, then
  adding a `ThreadWorker` size arm makes the error disappear while marshalling
  the wrong type** — a silent miscompile at a thread boundary, which is strictly
  worse than today's hard failure.

Establish which before adding anything. The two candidate raise sites are
`src/codegen/memory/marshal/record.rs:238` and
`src/codegen/collection/layout/builder_collection_layout.rs:694` — determine
which one fires here (they share the message text) rather than assuming.

Cover all three channel shapes in the fixture set, per this document's
"single correct behavior": `Thread OF Msg TO Out`, `Thread OF RES Res TO Out`
(reproduced above), and `Thread OF Msg RES Res TO Out`.

References:

- `mfb man errors` → "Local handling with an inline TRAP" — the idiom this breaks.
- `mfb man thread start` — `thread::start` is fallible.
- `bugs/completed/bug-464-sockets-and-listeners-not-thread-sendable.md` — the same *class* of defect one layer over: a transfer copy that only knew how to move a 4-slot resource header. Read its Root Cause before assuming a size constant is all this needs.
- Found while adding `--thread` to `examples/network-server`; the example uses the function-level-`TRAP` workaround below and cites this bug in a comment.

## Failing Reproduction

Any executable importing a package with an `EXPORT ISOLATED FUNC`. Minimal:

```
' packages/probewk/src/lib.mfb
EXPORT ISOLATED FUNC msgOnly(worker AS ThreadWorker OF String TO Integer, tag AS String) AS Integer
  RETURN 7
END FUNC
```

```
' src/main.mfb
IMPORT thread
IMPORT probewk

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(probewk::msgOnly, "W") TRAP(e)
    RETURN 1
  END TRAP
  RETURN 0
END FUNC
```

- Observed: `error: native inlined field size not available for type 'ThreadWorker OF String TO Integer' while lowering bind $trap_res0 AS Result OF Thread OF String TO Integer`, exit 1. No file, no line, no rule code.
- Expected: builds; on a failed start the handler runs.

Measured matrix (macOS aarch64, release `mfb` at `9782bc60b`), one probe project
per row:

| Channel shape | `TRAP` form | Result |
| --- | --- | --- |
| `Thread OF String TO Integer` (message only) | inline | fails ✗ |
| `Thread OF RES fs::File TO Integer` | inline | fails ✗ |
| `Thread OF RES tcp::Socket TO Integer` | inline | fails ✗ |
| `Thread OF RES tcp::Socket TO Integer` | **function-level** | works ✓ |
| `Thread OF RES tcp::Socket TO Integer` | none | works ✓ |

So it is **not** channel-specific and **not** resource-specific — every inline
`TRAP` on `thread::start` fails, and only the function-level `TRAP` is a way to
handle the error at all. Note the error names the **`ThreadWorker`** spelling
while the bind it is lowering is a **`Thread`**; that asymmetry is a clue and
should be explained, not just routed around.

## Root Cause

`src/codegen/memory/marshal/record.rs:emit_inlined_block_size` (the error text is
at `:238`) dispatches a `Result` payload's inlined-block size over: `String`,
collections, data unions / `ParameterType::ResultOf`, nested records (an explicit
rejection), and then `else` → this error. A `Thread`/`ThreadWorker` handle
matches none of them.

`src/codegen/collection/layout/builder_collection_layout.rs:694` raises the same
text from the collection-layout path and is the likely second site.

**Do not assume this is a missing size constant.** A thread handle is not a flat
self-contained block: `THREAD_BLOCK_SIZE` is 120 bytes containing *pointers* to
its inbound/outbound message queues and its two resource queues
(`THREAD_OFFSET_RESOURCE_INBOUND_QUEUE` = 104,
`THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE` = 112 — see `.ai/resources-packages.md`).
Marshalling it as a flat inlined block the way a `String` or a collection is
marshalled would copy the pointers, not the queues. That is the same mistake
bug-464 documents for the resource-transfer copy, and a wrong size in this
marshaller is a memory-corruption class defect, not a diagnostic nit.

The likely correct shape is that a `Result` payload holding a thread handle
should carry the **handle** (one pointer, the way a resource does) rather than an
inlined block — i.e. the missing arm is "treat it as a handle", not "here is its
byte size". Confirm which before writing code.

## Goal

- Every row of the matrix above compiles.
- A failed `thread::start` under an inline `TRAP` runs the handler and does not leak or double-free the partially-created thread block (cross-check `bugs/bug-469-failed-thread-start-segfaults-at-scope-exit.md`, which is the adjacent runtime defect on the same failure path).
- The unlocated `error:` is replaced by a located, rule-coded diagnostic for any shape that genuinely cannot be marshalled.

### Non-goals (must NOT change)

- The function-level `TRAP` and no-`TRAP` forms already work; they must keep working byte-identically.
- Do not "fix" this by rejecting the inline `TRAP` at parse/type time with a nicer message. That closes the diagnostic hole while leaving the documented idiom unusable — the idiom must work.
- Do not add a flat size for a thread handle without establishing that a flat copy is correct. See Root Cause.
- `thread::transfer`/`accept`'s own copy path is bug-464's and is out of scope.

## Blast Radius

- `src/codegen/memory/marshal/record.rs:emit_inlined_block_size` — the failing site.
- `src/codegen/collection/layout/builder_collection_layout.rs:694` — same text, collection-layout path. **Audit required**: does a thread handle reach it? A `List OF Thread` is already refused by `TYPE_COLLECTION_OWNERSHIP_VIOLATION` (measured), so this may be unreachable for threads — confirm rather than assume.
- Every fallible builtin returning a handle-shaped value — **audit required**: is `thread::start` the only one whose `Result` payload is a handle, or do others (`thread::waitFor` returning `Out`, any future handle-returning member) share it? The audit, not a guess, decides whether this is one arm or a class.
- `src/codegen/registry/mod.rs:1918` documents this error for `AudioOutput`, implying the class has been hit before for a resource type — **read that first**; it may already record the intended resolution.
- `examples/network-server/src/main.mfb` — carries the workaround and a comment citing this bug; remove both when it lands.

## Fix Design

Add the missing arm to `emit_inlined_block_size`, treating a thread handle the
way a resource handle is treated rather than as an inlined block. Establish first
which of the two the `Result` marshaller actually stores for a resource today,
and mirror it exactly.

Rejected: hard-coding `THREAD_BLOCK_SIZE` as the inlined size — copies queue
pointers into a `Result` payload whose lifetime differs from the thread's.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/threads/thread-start-inline-trap-rt/` covering all five matrix rows; confirm the three inline rows fail with the documented error.
- [ ] Add a fixture where the start genuinely fails (exhaust threads) and the inline handler must run — the behavior half, not just the compile half.
- [ ] Determine what the `Result` marshaller stores for a **resource** payload today, and whether a thread handle can reuse it. Record the answer here.
- [ ] Audit the two error sites and the handle-returning-builtin class above; write a verdict per site.
- [ ] Explain the `ThreadWorker`-vs-`Thread` naming asymmetry in the error.

Acceptance: the new fixtures fail for the documented reason; every question above has a recorded answer.
Commit: —

### Phase 2 — the fix

- [ ] Add the arm; make all five rows compile.
- [ ] Give any genuinely-unmarshallable shape a located, rule-coded diagnostic instead of the bare `error:`.

Acceptance: Phase 1 fixtures pass; no-`TRAP` and function-level-`TRAP` output unchanged.
Commit: —

### Phase 3 — validation + cleanup

- [ ] `scripts/artifact-gate.sh all` to 0 diffs; full `cargo test --no-fail-fast`; `cargo check --all-targets`; `scripts/test-accept.sh`.
- [ ] Remove the workaround and the bug-479 comment from `examples/network-server/src/main.mfb`; rebuild and re-run its `--thread` modes.

Acceptance: full suite green; the example uses the inline form again.
Commit: —

## Validation Plan

- Regression test: `tests/rt-behavior/threads/thread-start-inline-trap-rt/`.
- Runtime proof: a genuinely failing `thread::start` whose inline handler runs, with no leak or double free.
- Doc sync: none expected — this makes the documented idiom work rather than changing it.
- Full suite: `cargo test --no-fail-fast`, `scripts/artifact-gate.sh all`, `scripts/test-accept.sh`.

## Summary

The risk is in deciding *what* a `Result` should hold for a thread handle, not in
adding an arm. A thread block holds queue pointers, so the flat-block treatment
that fits a `String` is wrong here in a way that would corrupt memory rather than
fail loudly — which is exactly the trap bug-464 documents one layer over. The
diagnostic being unlocated is a secondary defect worth closing in the same pass.
