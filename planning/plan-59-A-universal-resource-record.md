# plan-59-A: Universal native resource record

Last updated: 2026-07-20
Overall Effort: x-large (1d–3d)
Effort: medium
Depends on: nothing
Produces: an 80-byte resource record — and therefore a `closed` flag at
`RESOURCE_OFFSET_CLOSED` — for **every** native `LINK` resource, stateless
included. `lower_link_thunk` unconditionally wraps a resource-typed return.
Consumed by plan-59-B (which has nowhere to put a guard without it).

Today a native `LINK` function that returns `AS RES T STATE S` is wrapped in an
80-byte resource record, but one that returns a bare `AS RES T` is **not** — the
raw native handle *is* the value (`src/target/shared/code/link_thunk.rs:1193`,
gated on `function.return_state_type.is_some()`). A stateless native resource
therefore has no record, no `closed` flag, and no place to put one.

This sub-plan makes the record universal. Behavioral outcome: after this lands,
every value bound by `RES x AS T` for a native `LINK` resource `T` is a pointer
to an 80-byte record whose word at offset 8 is a live `closed` flag, and
`sqlite3`'s `Db`/`Stmt` behave at the ABI exactly as `libsnd`'s stateful
`SoundFile` already does.

References:

- `planning/res.md` §1, §3, §8, §9 — Track B, the source design discussion
- `./mfb spec language resource-management` §15, §15.5, §15.6
- `src/target/shared/code/error_constants.rs:663-745` — the record layout, the
  offset-8 invariant, and the closed/moved bit definitions
- `planning/old-plans/plan-53-*` — the stateful native-resource record this
  generalizes

## Prerequisites

These are a precondition on the whole plan-59 feature, not a dependency to
negotiate. Every other letter points here.

| Must be true | Command | Status |
|---|---|---|
| The borrow→pointer terminology purge has landed (E rewrites these rules by name) | `grep -c TYPE_RESOURCE_INVALIDATE_NOT_OWNER src/rules/table.rs` → `1` | MET (commit `a6f4bf282`) |
| Track A (plan-52-A..D) is complete and archived — res.md §9: A does not need B, but B follows A | `ls planning/old-plans/plan-52-* \| wc -l` → `4` | MET |
| res.md §5 Q4 is decided: losing static use-after-close **through aliasing calls** is accepted | Recorded decision, project owner, 2026-07-20 | MET |
| Tree is green before starting | `cargo test` → all suites ok | MET (re-run 2026-07-20 at execution start: 3137 passed, 0 failed; 21 suites ok; exit 0) |

Everything in plan-59 is written against the world where these hold. There are no
fallbacks for the world where they do not.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only the
> one that blocked you.

## Dependency graph

```
A ← nothing;  B ← A;  C ← nothing;  D ← B;  E ← C + D
```

`A` and `C` can both start today. `C` is a checker/spec change independent of the
runtime work, but it **must** land before `E`, because `E` is what lets a
parameter escape and `C` is the rule that keeps an opaque `STATE` from being
laundered into a concrete one once it can.

Execution: topological order, re-checking each letter's stated preconditions.

## 1. Goal

- Every native `LINK` resource value is an 80-byte record pointer, so
  `RESOURCE_OFFSET_CLOSED` is a real, writable flag for stateless resources
  (`sqlite3`'s `Db` and `Stmt`) exactly as it already is for stateful ones
  (`libsnd`'s `SoundFile`).

### Non-goals (explicit constraints)

- **No guard behavior yet.** This sub-plan creates the slot; it does not read it.
  Nothing may start rejecting an operation. That is plan-59-B.
- **No change to `RESOURCE_RECORD_SIZE` (80) or `RESOURCE_OFFSET_CLOSED` (8).**
  The compile-time asserts in `error_constants.rs:717-744` and in
  `audio/mod.rs`, `tls/mod.rs`, `tls/macos.rs` must remain satisfied untouched.
- **No change to the `.mfp` ABI export encoding**, and no change to any
  user-visible signature. `FUNC open(path) AS RES Db` keeps its source spelling.
- **No change to the native calling convention.** The native symbol still
  receives the raw handle; only the MFBASIC-side value representation changes.

## 2. Current State

`lower_link_thunk` (`src/target/shared/code/link_thunk.rs:376`, 2499-line module)
produces a thunk per `LINK` function. Near the end it decides the returned value:

- `link_thunk.rs:1193` — `if function.return_resource && function.return_state_type.is_some()`
  allocates an 80-byte record, stores the handle at `FILE_OFFSET_FD`, zeroes
  `FILE_OFFSET_CLOSED` and the six buffer words, and leaves `STATE@16` null.
- Otherwise the value flows through `emit_return_passthrough`
  (`link_thunk.rs:1431`) or `emit_link_expr` (`:1695`) and
  `RESULT_VALUE_REGISTER` holds the **raw native handle** — verified by reading
  `link_thunk.rs:1140-1195`, not merely cited.

On the parameter side, `link_thunk.rs:843-848` already distinguishes a
record-resource param from a scalar one using the `stateful_native_resources`
set built at `:196-199` — that set is `filter(|f| f.return_resource && f.return_state_type.is_some())`,
so it is exactly the population that gets a record today.

### Measured populations

| What | Count | Command |
|---|---|---|
| `LINK` funcs returning a resource **with** `STATE` (have a record today) | 7 | `grep -rhn "AS RES [A-Za-z]* STATE" bindings/*/src/*.mfb tests/rt-behavior/native/*/src/*.mfb \| wc -l` → 7 |
| `LINK` funcs returning a resource **bare** (no record today — the gap) | 14 | `grep -rhn "AS RES [A-Za-z]*$" bindings/*/src/*.mfb tests/rt-behavior/native/*/src/*.mfb \| wc -l` → 14 |
| `RES`-taking funcs inside `LINK` blocks (params that become record pointers) | 69 across 11 files | awk over `LINK`…`END LINK` regions; per-file table in `/tmp/linkpop.txt`, totals: sqlite3 24, libsnd 5, tests 40 |
| Closed-flag guard sites in `link_thunk.rs` today | 0 | `grep -c FILE_OFFSET_CLOSED src/target/shared/code/link_thunk.rs` → 1 (a *store*, at `:1210`; zero reads) |

The stateless case is the **majority** of native resource producers — 14 vs 7 —
and includes the whole `sqlite3` binding.

### Verified properties

- **A stateless native resource genuinely has no record.** Verified by reading
  `link_thunk.rs:1140-1195` end to end, not by citation: the `else` arms set
  `RESULT_VALUE_REGISTER` from the passthrough/expr paths, and the record
  allocation is gated on `return_state_type.is_some()`.
- **`FILE_OFFSET_STATE` (16) stays null-safe for a stateless resource.** The
  record's `STATE@16` is null and `emit_resource_state_init` only runs for a
  binding that names `STATE` — so a stateless record simply never populates it.
  Verified by reading the `plan-53-A` comment block at `link_thunk.rs:1183-1192`
  and `builder_value_semantics.rs:115-132`.
- **UNVERIFIED — the close-op unwrap.** A stateless resource's registered close
  op is itself a `LINK` func taking `RES x AS T`. Once `T` is a record, that
  thunk must load `FD@0` before calling the native symbol. Whether the existing
  `stateful_native_resources` param path at `:843-848` covers this once the set
  is widened is *not* established by reading; it is Phase 2's first task.

## 3. Design Overview

One change, in one place: widen the record wrap from "returns a resource **with
STATE**" to "returns a resource", and widen the matching param-side set so the
thunk unwraps `FD@0` for every native resource param rather than only stateful
ones.

Concretely, the `stateful_native_resources` set at `link_thunk.rs:196-199` drops
its `&& f.return_state_type.is_some()` clause and is renamed to
`record_native_resources`, and the return-side condition at `:1193` drops the
same clause.

**Where design uncertainty concentrates:** the param-side unwrap (the
UNVERIFIED property above). Everything else is a mechanical widening of a
condition that already works for 7 functions. Phase 1 is therefore a spike that
proves a stateless resource survives a full open→use→close→drop cycle before any
broad change is made.

**Where correctness risk concentrates:** drop and reclamation. A stateless
resource's record now flows into `emit_resource_block_reclaim`
(`builder_codegen_primitives.rs:1606`), which frees buffer blocks when
`has_io_buffers` is set. A native record must never have that set — its words
24..72 are zeroed by the thunk, but the reclaim path must not be asked to free
them. That is Phase 3, scheduled last, behind tests.

**Rejected alternatives:**

- *Give only the resources that need a guard a record.* That is not knowable
  ahead of time — any native resource can be closed, so any can be used after
  close. A conditional record also keeps two representations alive, which is the
  thing this sub-plan exists to remove.
- *Put the closed flag beside the handle in a 16-byte mini-record.* Saves 64
  bytes per resource but forks the layout, breaks the offset-8 invariant's single
  meaning, and makes `thread::transfer`'s generic 80-byte copy conditional.

## 4. Detailed Design

The record for a stateless native resource is byte-identical in shape to the
stateful one:

```
  0  FD        the raw native handle (SNDFILE*, sqlite3*, …)
  8  CLOSED    bit 0 closed, bit 1 moved, 62 bits spare
 16  STATE     null — a stateless resource never populates it
 24..72        buffer words, zeroed, never used by a native resource
```

`RESOURCE_RECORD_SIZE` stays 80 so `thread::transfer`'s generic copy
(`builder_arena_transfer.rs:283-323`) stays uniform across every resource kind.

## Compatibility / Format Impact

- **Changes:** the in-memory representation of a stateless native `LINK`
  resource — from a raw handle to an 80-byte record pointer. This is internal to
  a compiled binary.
- **Unchanged:** every source-level signature; the `.mfp` ABI export encoding;
  the native calling convention (the symbol still gets the raw handle);
  `RESOURCE_RECORD_SIZE`; `RESOURCE_OFFSET_CLOSED`.
- Native resources are `dlopen`-based and compiled per build, so there is no
  cross-version binary compatibility surface to break.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. Use `- [~]` for partially done with a line on
> what remains, and `- [x] ~~text~~ — moot: <evidence>` rather than deleting.
> **An unticked box means NOT DONE.**

### Phase 1 — Spike: one stateless resource end to end

Falsifies the design's one unproven premise (the param-side unwrap) before any
broad change. Smallest thing that can prove the record works.

- [x] In `link_thunk.rs`, drop `&& f.return_state_type.is_some()` from the
      `stateful_native_resources` filter at `:196-199` **only**, leaving the
      return-side condition at `:1193` alone. Confirm this alone does not
      compile-break: the set is consumed at `:843-848`. — compiles clean, and
      the resulting *behavioral* break is itself the proof of C4 below.
- [x] Now drop the same clause from `:1193`. Build
      `tests/rt-behavior/native/native-link-sqlite-rt` (10 `RES`-taking `LINK`
      funcs — the densest stateless fixture) and run it. — passes with its
      golden **unchanged**, i.e. byte-identical observable output.
- [x] Read the emitted thunk for `sql::close` with `--ncode` + `otool -tV` and
      confirm it loads `FD@0` before the native call rather than passing the
      record pointer. (Note: lldb cannot break on mfb binaries.) — confirmed at
      both levels; see C4.
- [x] Record the result in Corrections — including if the param path already
      handles it, which would make Phase 2's first task moot. — see C4; the
      param path does already handle it, but Phase 2's first task is a **rename**
      and is NOT moot.

Acceptance: `native-link-sqlite-rt` opens a DB, prepares and finalizes a
statement, and closes cleanly with the same observable output as before the
change; the disassembled `sql::close` thunk demonstrably passes `FD@0`, not the
record pointer, to `sqlite3_close`.
**MET** — the fixture passes against its **unchanged** golden (so "same
observable output" is byte-equality, not a judgement call), and both the `.ncode`
plan and `otool -tV` show the `FD@0` dereference before the call. Evidence in C4.
Commit: —

### Phase 2 — Widen the record to every native resource

Generalizes the spike across all 14 bare-returning funcs.

- [ ] Rename `stateful_native_resources` → `record_native_resources` at
      `link_thunk.rs:196`, `:224`, `:381`, `:843`, and its doc comment at
      `:190-195` (which currently says "the resource TYPES that are represented
      as 80-byte records" — now true of all of them).
- [ ] Verify the zeroing block at `:1207-1223` runs for stateless returns too, so
      `CLOSED` starts at 0 and the buffer words are not arena poison.
- [ ] Confirm `emit_resource_state_init` is not invoked for a bare
      `RES x AS T` binding, leaving `STATE@16` null
      (`builder_value_semantics.rs:115-132`).
- [ ] Tests: add `tests/rt-behavior/native/native-stateless-record-rt` proving a
      stateless `Db` round-trips open → prepare → finalize → close → scope-drop
      with no leak, mirroring `native-link-free-rt`'s shape.

Acceptance: all 18 fixtures under `tests/rt-behavior/native/` pass (see
Corrections — the plan said 11), and the new fixture shows a stateless native
resource surviving a full lifecycle. `cargo test` green.
Commit: —

### Phase 3 — Drop and reclamation (largest blast radius, last)

The record now reaches the reclaim path; make sure it never frees native memory.

- [ ] In `emit_resource_block_reclaim` (`builder_codegen_primitives.rs:1606`),
      confirm `has_io_buffers` is false for every native `LINK` resource — a
      native record's words 24..72 are zeroed, but this path must not be asked to
      free them. Add a test if the guarantee is only positional.
- [ ] Confirm scope-drop calls the registered close op with the record (which the
      close-op thunk then unwraps), not with `FD@0` directly — the two must not
      double-unwrap.
- [ ] Run the acceptance suite for the native area and diff goldens: any
      `.ncode` golden covering a stateless native resource will change shape.
      Re-baseline **only** after confirming each diff is the record wrap and
      nothing else.
- [ ] Tests: extend `native-link-free-rt` to assert no arena growth across a
      1000-iteration open/close loop, mirroring plan-52-B's retention check.

Acceptance: `scripts/test-accept.sh` passes for `native*` and `resource*` with a
hermetic `MFB_HOME`; the retention loop shows bounded arena use; no golden
outside the native area changed.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/native/` (18 fixtures), plus the new
  `native-stateless-record-rt`.
- Coverage check: the native fixtures are `rt-behavior`, so they *execute* — a
  green run means the record path really ran, not merely compiled. Confirm the
  new fixture appears in the harness's run list (it discovers by directory scan;
  no Rust-side registration needed — verified during the borrow purge).
- Runtime proof: `native-link-sqlite-rt` performing a real sqlite3 open/prepare/
  finalize/close against a temp DB.
- Doc sync: `./mfb spec language native-libraries` §17 describes the native
  resource representation — update if it states the stateful-only record.
- Acceptance: `cargo test` and `scripts/test-accept.sh target/debug/mfb <tmp>
  'native*' 'resource*'` with `MFB_HOME=$(mktemp -d)`.

## Open Decisions

- ~~**Does a native record need `has_io_buffers` explicitly false, or is it false
  by construction?**~~ **DECIDED (owner, 2026-07-20): believed false by
  construction — but explicitly *not* with certainty.** So Phase 3 must **prove**
  it with a test rather than assert it; do not downgrade that task to a code
  comment on the strength of this decision. Only `fs::` open helpers set
  `has_io_buffers`, which makes it a *positional* fact rather than an enforced
  one, and a positional fact is exactly the kind that drifts. If the test shows a
  native record can reach the buffer-free path, that is a Correction and a real
  defect — a native handle's words 24..72 would be handed to `arena_free`.

## Corrections

### C1 — the native fixture population is 18, not 11 (2026-07-20)

Phase 2's and Phase 3's acceptance criteria, and the Validation Plan, all said
"11 fixtures under `tests/rt-behavior/native/`". The real count is **18**:

```
$ ls -d tests/rt-behavior/native/*/ | wc -l
18
```

Corrected in place. This matters because an acceptance criterion naming 11 of 18
would have passed while 7 fixtures went unrun — and 4 of the 7 unnamed ones
(`libsnd-load-sound-rt`, `libsnd-open-file-info-rt`, `libsnd-playback-rt`,
`libsnd-read-samples-rt`) are exactly the stateful-resource fixtures this
sub-plan's widening puts at risk. plan-59-B inherited the same wrong number from
here; corrected there too.

### C2 — plan-59-E's `closeSound` citation points into an uncommitted working tree (2026-07-20)

plan-59-E Phase 3's acceptance cites "the `bindings/libsnd` case at
`src/lib.mfb:317`". At HEAD, `bindings/libsnd/src/lib.mfb` contains no
`closeSound` at all — it exists only in an uncommitted working-tree change
present when execution began, which also adds `openSound`, `loadFrames`, and
`seekFrames`, and rewrites `sndError`'s signature. So E's headline runtime proof
was written against a dirty tree rather than against HEAD.

Not resolved here (it is E's to resolve), but recorded now because it changes
what E must do: E cannot assume `closeSound` exists, and must either land that
binding change as part of its own work or re-pin the citation to a fixture it
creates. Noted in E's Corrections as well.

### C4 — Phase 1 result: the param path already handles it, PROVEN (2026-07-20)

§2's UNVERIFIED property is discharged. Three pieces of evidence, in the order
they were produced:

**1. The intermediate state breaks, which is the positive result.** With only the
filter widened (`:196`) and the return side left alone, the tree compiles but
`native-link-sqlite-rt` fails:

```
-1=alice@1.50 / 2=bob@2.50 / done / [exit 0]
+Error: 7-703-0008  Native `LINK` binding call failed its `SUCCESS_ON` gate. [exit 255]
```

That failure is the *proof*: the param path began dereferencing `FD@0` on a value
that was still a raw `sqlite3*`. Had the param path been gated on statefulness
somewhere else, widening the set alone would have changed nothing and the fixture
would have stayed green. A deliberately-broken intermediate state was the cheapest
available discriminator, and it is why the plan sequenced these two edits apart.

**2. With both edits, output is byte-identical.** `native-link-sqlite-rt` passes
against its **unchanged** golden — not a re-baselined one. No golden was touched.

**3. The `FD@0` load is in the emitted code, at both levels.** `.ncode` for
`linker.sql.close`:

```
str_u64 x0  -> [sp+64]      ; park the incoming param (record pointer)
ldr_u64 x21 <- [sp+64]
ldr_u64 x21 <- [x21 + 0]    ; FD@0
str_u64 x21 -> [sp+72]
ldr_u64 x0  <- [sp+72]      ; x0 = raw handle
ldr_u64 x22 <- [x19 + 3848] ; GOT slot for sqlite3_close
blr x22
```

and `otool -tV` at `0x100005440` shows the same seven instructions with GOT
offset `0xf08` = 3848, confirming the slot identity rather than assuming it.

**Consequence for Phase 2.** Its first task is **not** moot: it is a rename
(`stateful_native_resources` → `record_native_resources`) plus a doc-comment fix,
and the comment is now actively wrong in a load-bearing way — it says "a native
func produces `AS RES R STATE S`", which is no longer what qualifies a type for
the set. Renaming is what stops the next reader concluding the set is
stateful-only. The task stands as written.

### C3 — the param-side unwrap is type-keyed, and already covers bare params (2026-07-20)

§2's UNVERIFIED property asked whether the existing param path at `:843-848`
covers the close-op unwrap once the set is widened. Reading `link_thunk.rs:841-856`
in full: the condition is keyed on the resource **type**
(`stateful_native_resources.contains(base_resource_name(t))`), not on the
declaration, and its comment states outright that it fires for a bare
`RES db AS SoundFile` param too. So widening the set should carry the param side
with it, unmodified.

This is a reading, not a proof, and does **not** discharge Phase 1 — Phase 1's
disassembly task stands. Recorded here so that if Phase 1 confirms it, Phase 2's
first task is already known to be a rename rather than new logic.

## Summary

The real engineering risk is the param-side unwrap: 69 `RES`-taking `LINK` funcs
across 11 files all start receiving a record pointer where they previously
received a raw handle, and the native symbol on the other side still wants the
handle. That conversion happens in one place, which is why Phase 1 proves it on
one fixture before Phase 2 widens it.

Untouched: the record layout, the offset-8 invariant, every source-level
signature, and all guard behavior — nothing starts rejecting an operation until
plan-59-B.
