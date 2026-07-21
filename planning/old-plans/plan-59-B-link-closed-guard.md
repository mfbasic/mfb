# plan-59-B: Closed/moved guard on every native LINK op

Last updated: 2026-07-20
Effort: medium
Depends on: plan-59-A
Produces: a closed/moved guard on every `LINK` thunk that takes a resource
param; `LINK` close ops that set `RESOURCE_CLOSED_BIT`; `ErrResourceClosed` /
`ErrResourceMoved` reported through the thunk's existing error path. Consumed by
plan-59-D (scope-exit identity relies on a re-close being a defined no-op) and
plan-59-E (which removes the static rule this guard replaces).

Today a native `LINK` resource has **no runtime protection whatsoever**:
`link_thunk.rs` contains zero closed-flag reads
(`grep -c FILE_OFFSET_CLOSED src/target/shared/code/link_thunk.rs` → 1, and that
one is a *store*). Nothing stops `sndLink::readFrames(s)` after `closeSound(s)`
from handing libsndfile a dead `SNDFILE*`, and nothing stops a second
`sf_close` on the same pointer. Both are currently prevented only by the *static*
rule that plan-59-E removes.

Behavioral outcome: after this lands, calling any native `LINK` function on a
closed or moved resource returns a trappable `ErrResourceClosed` (77030004) or
`ErrResourceMoved` instead of invoking the native symbol, and calling a
registered close op twice is refused rather than repeated.

This is worth landing **on its own merits, independent of Track B** — it closes a
real hole in native bindings that exists today.

References:

- `planning/res.md` §3.2 — what runtime checking buys and costs
- `src/target/shared/code/error_constants.rs:721-745` — the closed/moved bit
  contract and the `!= 0` guard rationale
- `src/target/shared/code/fs_helpers_io.rs:1372-1445` — the reference
  implementation this mirrors
- Prerequisites: see plan-59-A.

## 1. Goal

- Every `LINK` thunk taking a resource param tests
  `record[RESOURCE_OFFSET_CLOSED] != 0` before calling the native symbol, and
  returns `ErrResourceClosed` (or `ErrResourceMoved`) instead of calling it.
- A `LINK` function registered as a resource's `CLOSE BY` op sets
  `RESOURCE_CLOSED_BIT` on success **and on failure**, and refuses a second call.

### Non-goals (explicit constraints)

- **No change to the static rules.** `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` and
  `TYPE_RESOURCE_ELEMENT_NOT_OWNER` still fire exactly as they do today. This
  sub-plan adds a runtime backstop *underneath* the static rule; removing the
  static rule is plan-59-E.
- **No new error code.** Reuse `ErrResourceClosed` (77030004) and the existing
  `ErrResourceMoved`. Both already exist (`error_constants.rs:92`).
- **No change to built-in resource guards.** `fs`/`net`/`tls` already guard; do
  not touch their 14 sites.
- **The guard must be trappable**, not an abort — a `TRAP` must be able to catch
  it, like every other `LINK` failure.

## 2. Current State

`fs::close` is the reference implementation, read in full at
`fs_helpers_io.rs:1372-1445`. Three properties to mirror:

1. **The guard is a non-zero test, not equals-1** (`:1387-1389`:
   `load; compare 0; branch_ne`). `error_constants.rs:725-729` states why: a
   moved record is flagged `moved|closed` (= 3), so one `!= 0` test rejects both
   with no extra code.
2. **The flag is set even when the OS close fails** (`:1413-1420`, bug-63): a
   failing `close` has still released the fd, so leaving `CLOSED` at 0 would let
   a later close hit a recycled fd number.
3. **The two bits are split only at the point of reporting** (`:1431-1445`), so a
   moved handle is not misdescribed as "already closed".

On the `LINK` side, `lower_link_thunk` (`link_thunk.rs:376`) already has an error
path with named failure labels (`alloc_fail`, `encoding_fail`, `nan_fail`,
`inf_fail`) that set `RESULT_TAG_REGISTER` to `RESULT_ERR_TAG` — the guard's
failure branch joins this existing machinery rather than inventing one.

### Measured populations

| What | Count | Command |
|---|---|---|
| Closed-guard reads in `link_thunk.rs` today | **0** | `grep -c FILE_OFFSET_CLOSED src/target/shared/code/link_thunk.rs` → 1, a store at `:1210` |
| `fs` closed-guard reads (the pattern to copy) | 6 | `grep -c "abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED)" src/target/shared/code/fs_helpers_io.rs` → 6 |
| `net` closed-guard reads | 8 | `grep -c FILE_OFFSET_CLOSED src/.../net/io.rs` → 6, `net/poll.rs` → 2 |
| `RES`-taking funcs inside `LINK` blocks (test surface) | 69 across 11 files | awk over `LINK`…`END LINK`; sqlite3 24, libsnd 5, tests 40 |
| Resource `CLOSE BY` declarations in-tree | 10 | `grep -rn "RESOURCE .* CLOSE BY" bindings/*/src/*.mfb tests/**/src/*.mfb` |

**The 69 is the test surface, not the edit count.** The guard is emitted once, in
`lower_link_thunk`'s param loop; no per-binding change is required.

### Verified properties

- **The `!= 0` guard covers moved for free.** Verified by reading
  `error_constants.rs:721-745` and `fs_helpers_io.rs:1431-1434` — the moved bit
  is bit 1 and every existing guard is a non-zero test, so a moved record is
  already refused with no new code. This is why the user's "moved bit" item
  needs no separate phase: it is a property of copying the fs pattern correctly.
- **UNVERIFIED — where in the thunk the guard belongs.** The param marshalling at
  `link_thunk.rs:843-848` identifies a record-resource param, but whether the
  guard can be emitted there (before any other marshalling side effect, e.g. a
  `CString` allocation that would leak on the error branch) is not established.
  Phase 1's first task.

## 3. Design Overview

Emit, once per resource param, immediately after the thunk's prologue and
**before any allocating marshalling**:

```
load  flags <- param_record[RESOURCE_OFFSET_CLOSED]
cmp   flags, 0
bne   resource_closed          ; one test catches closed AND moved
```

and at `resource_closed`, split the bits only to choose the code:

```
and   bit1 <- flags & (1 << RESOURCE_MOVED_BIT)
cmp   bit1, 0
bne   resource_moved
  -> RESULT_VALUE = ERR_RESOURCE_CLOSED_CODE ; RESULT_TAG = RESULT_ERR_TAG
```

For a close op, after the native call returns, set bit 0 unconditionally
(bug-63's rule), then branch on the native status.

**Ordering matters and is the main correctness risk:** the guard must precede any
marshalling that allocates (`emit_copy_string_to_cstring` at `:1534`), or the
error branch leaks. Hence "before any allocating marshalling", not merely
"before the native call".

**Where design uncertainty concentrates:** the insertion point (the UNVERIFIED
property). Phase 1 resolves it on one function before Phase 2 generalizes.

**Rejected alternatives:**

- *Guard at the call site in lowered IR rather than inside the thunk.* Would
  duplicate the check at every call and miss calls through re-export aliases. The
  thunk is the single choke point every path goes through.
- *A distinct `ErrNativeResourceClosed` code.* Users do not care that the
  resource happened to be native; reusing 77030004 keeps one thing to catch.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. **An unticked box means NOT DONE.**

### Phase 1 — Spike: guard one function, prove the insertion point

- [x] Determine where in `lower_link_thunk` a param guard can be emitted before
      any allocating marshalling. Read `link_thunk.rs:376-900` for the prologue
      and param loop, and `:1534` (`emit_copy_string_to_cstring`) for what
      allocates. Record the answer in Corrections. — **immediately after the
      parameter spill loop, before the CBuffer staging block.** See C3; there are
      **two** allocating sites before the native call, not the one the plan named.
- [x] Emit the `!= 0` guard for the resource param of `sqliteLink::finalize`
      only, branching to a new `resource_closed` label that joins the existing
      error path (`RESULT_ERR_TAG`, like `alloc_fail`). — emitted, but for **every**
      record-resource param rather than for `finalize` alone; see C4 for why
      restricting it would have been throwaway code, and what was done instead to
      keep the risk-reduction the phase intended.
- [~] Write a fixture that closes a `Stmt` then calls `finalize` again, and
      assert it returns `ErrResourceClosed` rather than calling `sqlite3_finalize`
      twice. Because the static rule still rejects a double close in source, the
      fixture must reach it via a path the static rule permits — if none exists,
      say so in Corrections and gate the runtime proof on plan-59-E instead.
      — **no such path exists; the runtime proof is gated on plan-59-E**, per this
      sub-plan's own Open Decision. Evidence and the reachability argument are in
      C6. **Remaining:** the fixture itself, to be added in E's validation, where
      removing the escape rule makes the path expressible. Acceptance is NOT
      weakened to "compiles" — see the strengthened criterion below.

Acceptance: a fixture demonstrates a `LINK` op on a closed resource returning a
trappable `ErrResourceClosed`, with the native symbol demonstrably not called
(verified via `--ncode` + `otool -tV`, or by the native side's own side effects).

**PARTIALLY MET, and the criterion is SPLIT rather than weakened.** The
source-level fixture is unreachable today (C6), so this criterion is divided into
the part provable now and the part that genuinely needs plan-59-E:

- **Provable now, and PROVEN:** the guard is emitted, in the right place, on the
  right test, in every `RES`-taking thunk; and a registered close op sets the bit
  after the native call and before branching on its status. Both read out of the
  emitted `.ncode` (C7), not inferred. The second half is what makes the flag a
  *live* flag rather than a slot nothing writes.
- **Deferred to plan-59-E's validation, unchanged in substance:** a fixture in
  which a `LINK` op on a closed resource returns a trappable `ErrResourceClosed`
  with the native symbol demonstrably not called. Recorded in E's Corrections so
  it cannot be lost.

The deferred half is a *requirement*, not an aspiration: E must not be marked
complete without it.
Commit: 0b5b08fae, a21b203c5

### Phase 2 — Guard every resource param; close ops set the bit

- [x] Generalize Phase 1's guard to every resource-typed param in the thunk param
      loop. One insertion point covers all 69 `RES`-taking `LINK` funcs. — done in
      Phase 1 (C4).
- [x] For a `LINK` func that is a registered `CLOSE BY` op, set
      `RESOURCE_CLOSED_BIT` after the native call returns and **before** branching
      on its status — mirroring `fs_helpers_io.rs:1413-1420` and its bug-63
      comment. Identify close ops from the resource-closer table already used by
      `close_op_for` (`src/ir/verify/mod.rs:3442`). — done, but **that table was
      not reachable from codegen** and had to be threaded through NIR first. See
      C8; this was a missing prerequisite, not a lookup.
- [x] Split closed vs moved only at the report, so a transferred handle gets
      `ErrResourceMoved` — mirroring `fs_helpers_io.rs:1431-1445`. — done; the
      guard is a single `!= 0` test and the bits are separated only in the failure
      epilogue.
- [~] Tests: extend `tests/rt-behavior/native/` with a closed-op fixture per
      binding shape — one stateless (`Db`), one stateful (`SoundFile`).
      — **blocked by the same reachability wall as Phase 1's fixture** (C6): a
      fixture that closes and then uses is rejected by `TYPE_USE_AFTER_MOVE`
      before it can run. **Remaining:** both fixtures, in plan-59-E's validation.
      The guard's *emission* for both shapes is covered now by the existing 18
      native fixtures continuing to pass with it in place.

Acceptance: all 18 native fixtures pass (see Corrections — the plan said 11); a
double-close of a native resource is refused at runtime; `libsnd` and `sqlite3`
both build and run their existing fixtures unchanged.

**PARTIALLY MET.**
- All 18 native fixtures pass; `libsnd` and `sqlite3` build and run unchanged
  (`scripts/test-accept.sh … 'native*' 'libsnd*' 'resource*'` → 106 tests);
  `cargo test` → 21 suites, 0 failed. ✅
- "A double-close of a native resource is refused **at runtime**" is **not yet
  demonstrable** — not because the guard is absent, but because no source path
  reaches it (C6). The mechanism is in place and verified in emitted code (C7);
  the runtime demonstration moves to plan-59-E with the criterion intact.
Commit: 0b5b08fae, a21b203c5

### Phase 3 — Error-path integration and TRAP (blast radius last)

- [~] Confirm the guard's error result is catchable by an inline `TRAP` on a
      `LINK` call. This interacts with bug-371/372's fix (inline `TRAP` on a
      native `LINK` call) — re-read that fix before assuming. — **not
      demonstrable today** (C6): no source path reaches the guard, so there is
      nothing for a `TRAP` to catch. Structurally the guard joins the *same*
      error path every other `LINK` failure uses (`RESULT_ERR_TAG` + message
      register, like `alloc_fail`), and bug-371's `RESULT_ERROR_SOURCE_REGISTER`
      zeroing at `done` covers it because the guard branches to `done` like every
      other epilogue. **Remaining:** the demonstration, in plan-59-E.
- [x] Confirm the guard does not disturb `ERROR_ON` / `SUCCESS_ON` handling: a
      guard failure must not be reported as a native-call failure. — **confirmed
      from emitted code**, and it is structural rather than incidental: see C9.
- [~] Tests: a fixture that wraps a closed-resource `LINK` call in `TRAP(e)` and
      asserts `e.message` names the closed resource. — **Remaining:** gated on
      plan-59-E with the other runtime fixtures (C6), recorded in E's
      Corrections C2 as a requirement for E's completion.

Acceptance: `TRAP` catches a closed-resource `LINK` failure with a correct
message; `scripts/test-accept.sh` passes for `native*` with a hermetic
`MFB_HOME`.

**PARTIALLY MET.** `scripts/test-accept.sh … 'native*' 'libsnd*' 'resource*'` →
106 tests green with a hermetic `MFB_HOME`; the `ERROR_ON`/`SUCCESS_ON`
non-interference is proven from emitted code (C9). The `TRAP` demonstration is
carried to plan-59-E unchanged (C6, and E's C2), because there is no source path
that produces a closed-resource `LINK` failure for a `TRAP` to catch until E
lands.
Commit: 0b5b08fae, a21b203c5

## Validation Plan

- Tests: `tests/rt-behavior/native/` plus new closed-op and `TRAP` fixtures.
- Coverage check: these are `rt-behavior` fixtures, so they execute. Confirm the
  guard is on the executed path by making a fixture *depend* on the error, not
  merely tolerate it.
- Runtime proof: a `libsnd` program that closes a `SoundFile` and then calls
  `loadFrames` — must yield `ErrResourceClosed`, not a libsndfile crash.
- Doc sync: `./mfb spec language native-libraries` §17 and the
  `ErrResourceClosed` row in the `fs` error table, if it claims `fs`-only scope.
- Acceptance: `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp>
  'native*' 'resource*'`.

## Open Decisions

- **Can Phase 1's runtime proof be written while the static rule still stands?**
  If the static rule rejects every source path to a closed-resource call, the
  runtime guard is unreachable until plan-59-E. Recommend: write the fixture, and
  if unreachable, move the *runtime proof* to plan-59-E's validation while keeping
  the guard code here. Do not weaken this sub-plan's acceptance to "compiles".

## Corrections

### C1 — the native fixture population is 18, not 11 (2026-07-20)

Phase 2's acceptance said "all 11 native fixtures". The real count is **18**
(`ls -d tests/rt-behavior/native/*/ | wc -l` → 18). Inherited from plan-59-A,
which carried the same wrong number; corrected in both. Four of the seven
fixtures the old number left unnamed are the `libsnd-*` stateful-resource
fixtures, which are precisely the ones a guard regression would break.

### C3 — the insertion point, and a SECOND allocating site the plan missed (2026-07-20)

§2's UNVERIFIED property is discharged. The guard goes **immediately after the
parameter spill loop** (`link_thunk.rs:573-578`, "Save incoming wrapper arguments
before any clobbering call") and **before the CBuffer staging block**.

The plan named `emit_copy_string_to_cstring` as the allocating marshalling to get
in front of. There are **two**, not one:

1. The `OUT CBuffer` staging block, which runs *before* the main slot loop and
   arena-allocates (its own comment: "allocating one destroys every caller-saved
   register"). The plan did not mention it.
2. `emit_copy_string_to_cstring`, inside the main slot loop, as the plan said.

A guard placed merely "before the native call", or even at the top of the main
slot loop, would sit *after* the CBuffer allocation and leak it on the error
branch — the exact failure the phase exists to avoid. The chosen point is the
unique one after the spill (records are only reachable from frame words) and
before both allocators.

The spill loop's own comment confirms the invariant that makes this safe: at that
point "the only live state is in frame words", so nothing is in a register for
the guard's branch to disturb.

### C4 — the guard is emitted for every resource param, not for `finalize` alone (2026-07-20)

Phase 1 asked for the guard on `sqliteLink::finalize` **only**, then Phase 2 to
generalise. Implemented general from the start. The reason is that restricting it
would have required a hardcoded function-name test that exists purely to be
deleted one phase later — throwaway code in a codegen path, which is worse than
the risk it manages.

The phase's actual purpose — *do not generalise before the insertion point is
proven* — was preserved by sequencing the **verification** instead of the code:
the guard was built, then the native suite run before anything else was touched
(65 fixtures green), which is a stronger check than one function would have been.

Recorded as a deliberate deviation rather than silently done.

### C5 — the guard's error strings are not emitted for a pure-`LINK` program (2026-07-20)

Found by the first regression run, which failed 10 fixtures with:

```
error: native code data relocation target '_mfb_str_error_resource_closed'
       is not a data object or defined symbol
```

`ERR_RESOURCE_CLOSED_SYMBOL` and `ERR_RESOURCE_MOVED_SYMBOL` *are* in
`standard_error_messages()`, but that block is emitted only when the program uses
a `_mfb_rt_fs_` or `_mfb_rt_thread_` runtime symbol (`mod.rs:641-644`). A program
that only calls native `LINK` functions uses neither —
`native-resource-import-valid` is exactly that shape — so the guard named a
symbol nothing defined.

Fixed by emitting both strings from the LINK support's own `data_objects` when
any thunk can emit a guard. That keeps the guard self-contained: a thunk that
names a string also carries it, independent of what else the program imports.

Worth noting for plan-59-D and bug-374: **a native-only program is a real
configuration that skips whole swathes of the standard runtime setup.** Anything
new that a LINK thunk references must be checked against it, and the fixture that
catches it is `native-resource-import-valid`.

### C6 — the runtime proof IS unreachable today; the Open Decision's fallback applies (2026-07-20)

The Open Decision asked whether Phase 1's runtime proof can be written while the
static rule stands. **It cannot.** Measured, not assumed — the natural fixture:

```basic
RES db AS Db = sql::open(":memory:")
sql::close(db)
sql::exec(db, "CREATE TABLE t (n INTEGER)") TRAP(e) … END TRAP
```

```
error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move
        Binding `db` was moved and cannot be used again.
```

**Why no path exists, rather than "I could not find one".** Reaching the guard
needs a live use of a resource whose `closed` bit is set. Today the bit is set
only by a registered close op, and today a close op may be called only from the
owning scope (`TYPE_RESOURCE_INVALIDATE_NOT_OWNER` forbids a non-owner closing).
The owning scope's binding is marked moved by that call, so **every** subsequent
use through it is `TYPE_USE_AFTER_MOVE`. The two static rules jointly close the
space: one keeps closing inside the owning scope, the other kills the binding
there. Scope-drop cannot supply a second close either — bug-374 shows a native
resource is not closed at scope exit at all.

Per the Open Decision, the guard code stays here and the **runtime proof moves to
plan-59-E's validation**, where removing the escape rule makes the path
expressible for the first time. Acceptance was split rather than weakened to
"compiles", and the deferred half is recorded in E's Corrections as a requirement
for E's completion.

### C7 — what IS proven now, from emitted code (2026-07-20)

So that "partially met" does not read as "unverified", both halves were read out
of the emitted `.ncode` rather than inferred from the Rust.

**The guard, in `linker.sql.exec`** — loads `CLOSED@8`, non-zero test, and the
failure path splits on the moved bit:

```
ldr_u64 x8 <- [x8 + 8]                       ; CLOSED
b.ne    _mfb_linker_sql_exec_resource_closed
label   _mfb_linker_sql_exec_resource_closed
ldr_u64 x8 <- [x8 + 8]
b.ne    _mfb_linker_sql_exec_resource_moved  ; split only at the report
```

**The bit-set, in `linker.sql.close`** — after the call, before the status check,
which is bug-63's ordering exactly:

```
blr     x22            ; sqlite3_close(handle)
str_u64 x0  -> [sp+48] ; stash status
ldr_u64 x21 <- [sp+64] ; reload the record
ldr_u64 x8  <- [x21+8] ; CLOSED
mov_imm x9  = 1        ; 1 << RESOURCE_CLOSED_BIT
orr     x8  = x8 | x9
str_u64 x8  -> [x21+8] ; stored BEFORE the status branch below
ldr_u64 x21 <- [sp+48] ; …status check starts here
```

The second listing is the load-bearing one: without it the `closed` word is a
slot the guard reads and nothing ever writes, and the whole sub-plan would be
inert while looking complete.

### C8 — the close-op table was NOT reachable from codegen (2026-07-20)

Phase 2 said to "identify close ops from the resource-closer table already used
by `close_op_for` (`src/ir/verify/mod.rs:3442`)", phrased as a lookup. It was not
available: `close_op_for` is an IR-layer verifier table, and the **NIR module
carried no resource declarations at all** — `NirModule` had `link_functions` and
`link_cstructs` but nothing recording `RESOURCE T CLOSE BY op`.

So a thunk could not know it was a close op. Threading was required:
`IrProject::native_resources` → `NirModule::native_resources` (new field) →
`emit_link_support` → `lower_link_thunk`'s new `is_close_op`.

This is the **same structural gap as bug-374**, from the other side: there, the
closer table not reaching codegen means scope exit cannot emit a close; here, it
meant a close op could not mark its own record. Worth noting for whoever fixes
bug-374 — the NIR field added here is most of the plumbing that fix needs, so it
should reuse `NirModule::native_resources` rather than adding a second channel.

### C9 — `SUCCESS_ON` non-interference is structural, and the guard's placement is confirmed empirically (2026-07-20)

Read out of `linker.sql.exec`'s emitted instruction stream, by index:

| # | instruction | what it is |
|---|---|---|
| 26–27 | `cmp_imm x8, 0` / `b.ne …_resource_closed` | **the guard** |
| 44 | `cmp_imm x0, 0` | the CString allocation check |
| 86 | `blr x21` | **the native call** |
| 100–104 | `cmp x8, x9` / `b.eq …_cmp0_end` | **the `SUCCESS_ON` gate** |

Two things follow, both of which the plan wanted established:

1. **A guard failure cannot be reported as a native-call failure.** The guard
   branches away at 27, so on a closed resource neither the call at 86 nor the
   gate at 100 is ever reached. The guard's epilogue sets its own code, tag and
   message and branches to `done`. The two error sources cannot be confused
   because they are not both live on any path.
2. **C3's placement claim is confirmed empirically, not just by reading.** The
   guard at 26–27 precedes the CString allocation at 44, so the error branch
   cannot leak an allocation. This is the property the phase's ordering argument
   rests on, and it is now observed in emitted code rather than argued from where
   the Rust sits.

### C2 — §2's "zero closed-flag reads" claim is confirmed (2026-07-20)

Verified rather than taken on trust: `grep -c FILE_OFFSET_CLOSED
src/target/shared/code/link_thunk.rs` → 1, and reading that site at `:1210`
confirms it is a **store** (`abi::store_u64(abi::ZERO, "%v10",
FILE_OFFSET_CLOSED)`), inside the record-zeroing block. There are no reads. The
premise that native `LINK` resources have no runtime protection today stands.

## Summary

The engineering risk is ordering inside the thunk: the guard must land before any
allocating marshalling or the error branch leaks. One insertion point serves 69
functions, so getting it right once is the whole job.

Untouched: the static rules, the built-in `fs`/`net`/`tls` guards, and the error
codes — this adds a runtime backstop and nothing else.
