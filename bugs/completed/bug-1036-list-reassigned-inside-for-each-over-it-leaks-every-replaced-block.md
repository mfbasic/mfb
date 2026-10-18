# bug-1036: a list reassigned inside a `FOR EACH` over it leaks every replaced block

Last updated: 2026-10-18
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed
Regression Test: tests/runtime/rt_for_each_self_update.rs:a_for_each_over_a_graph_list_frees_every_block_its_body_replaces

## Cause (coordinator, bug-staging-24)

- Rule: no intentional memory leak; a block may outlive its binding only where the semantics require it, then it is freed (owner decision 2026-10-10): a list reassigned inside a `FOR EACH` over it keeps the iterated block alive until the loop ends, then frees it.
- Where: `src/codegen/engine/control/builder_control.rs`, the `NirOp::Assign` arm, skips the old-value free for a name in `for_each_iterable_locals` ("That old block leaks, but the pattern is rare"); `try_inplace_set_assign` has the mirror guard.
- Fix: the replaced block is not freed at the assignment (the loop still reads it) but is released when the `FOR EACH` ends, every replaced block once; the "leaks, rare" exclusion becomes that deferral. `live_bytes` 0 at exit.

A `MUT` list that is reassigned in the body of a `FOR EACH` over that same list keeps every
block the reassignment replaced. The code does this on purpose: the loop's iterator still
reads the old block, so the reassignment skips the free, and nothing frees it later.

**Owner decision (2026-10-10):** there is no intentional memory leak. A block may outlive
its binding only where the semantics require it, and then it is freed afterwards — here,
when the loop ends.

**Correct behavior:** every replaced block is freed once the `FOR EACH` that was reading it
is done; `live_bytes` is 0 at exit.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44, macOS aarch64): `mfb build -q --debug`, run with `N=10`. Result: stdout `10`,
`alloc_calls 912`, `free_calls 502`, `live_bytes 501600`. Expected `live_bytes 0`.
`/tmp/g933/data.txt` is a 6-byte file.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC which(label AS String, h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      LET text AS String = fs::readAll(f)
      RETURN 1
    CASE tcp::Socket(s)
      RETURN 2
  END MATCH
  RETURN 0
END FUNC

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC

FUNC once() AS Integer
  RES a AS Handle = openOne()
  RES c AS Handle = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  MUT t AS Integer = 0
  FOR EACH h IN hs
    RES v AS Handle = h
    FOR j = 1 TO 40
      hs = collections::append(hs, c)
    NEXT
    t = t + which("v", v)
  NEXT
  RETURN t
END FUNC
```

The same loop over a `List OF Integer` (`xs = collections::append(xs, j)` inside
`FOR EACH x IN xs`) does not leak: 82 allocations, 82 frees, `live_bytes 0`, same compiler.
So the leaking path is narrower than "any list"; which element types reach it was not
traced. Part of the 501,600 bytes may be bug-1028's leak (a list of resource unions that
is iterated keeps its block); separate the two before fixing.

## Root Cause

`src/codegen/engine/control/builder_control.rs`, the `NirOp::Assign` arm, excludes a name
in `for_each_iterable_locals` from the old-value free, with the comment "That old block
leaks, but the pattern is rare." `try_inplace_set_assign` has the mirror guard.

## Goal

- The program above ends with `live_bytes 0` and no double free.
- The comment that accepts the leak is gone.

### Non-goals

- Freeing the block while the iterator still reads it (that is a use-after-free).

## Additional reproduction (2026-10-11)

`c13` is the same leak with a `List OF RES fs::File` in place of a resource union list: `once`
iterates `hs` with `FOR EACH h IN hs` and reassigns `hs = collections::append(hs, c)` inside
the body. Measured with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
dcf790e9a, macOS aarch64, `mfb build -q --debug`, harness `/tmp/bs21-agent-a/bin/probe.sh`,
`arena.0` counters alloc / free / live):
`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c13`: exit 0, stdout
`10`, 132 / 122 / 960. `N=20`: stdout `20`, 262 / 242 / 1920. The live bytes grow by 96 per call.
`/tmp/g933/data.txt` is a 6-byte file ("hello\n").

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE HB
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC which(label AS String, h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      LET text AS String = fs::readAll(f)
      RETURN 1
    CASE tcp::Socket(s)
      RETURN 2
  END MATCH
  RETURN 0
END FUNC

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
FUNC whichF(label AS String, f AS fs::File) AS Integer
  LET text AS String = fs::readAll(f)
  RETURN 1
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  MUT hs AS List OF RES fs::File = [a]
  MUT t AS Integer = 0
  FOR EACH h IN hs
    hs = collections::append(hs, c)
    t = t + whichF("e", h)
  NEXT
  FOR j = 1 TO 20
    hs = collections::append(hs, c)
  NEXT
  RETURN t + whichF("z", collections::get(hs, 20)) - 1
END FUNC
```

## Fix (bs24-agent-f, 2026-10-18)

**Where the leak still lived.** Both programs above end at `live_bytes 0` on
bug-staging-24 before this fix (`mfb build --debug`, N=10 and N=20): a `List OF RES …`
is now a freeable-flat value, so a `FOR EACH` whose body writes it walks a copy made
at entry (plan-142-F) and never reaches the guard. The guard is still reached by a
binding the loop cannot copy: a graph-typed list (an element type that reaches a type
cycle, `List OF Node` with `kids AS List OF Node`). Measured with this branch's
compiler before the fix (`mfb build --debug`, the `List OF Node` program in the test):
`N=10` 352 allocs / 272 frees / 6880 B live, `N=20` 13760 B live.

**Fix.** `NirOp::Assign` no longer skips the old-value free for a live `FOR EACH`
iterable. `lower_for_each` registers, for a loop that walks the binding's own block
and whose body writes it (`ops_write_local`), a `for_each_replaced` slot (zeroed at
entry) as an `OwnedValue` cleanup outside the body's scope, and records it in
`for_each_entry_blocks`. The reassignment (`emit_defer_walked_block`) compares the
old block with each live loop's walked block, outermost first: a match is stored in
that loop's slot and the free is skipped; any other block is freed at once. The loop
frees its slot at `end_label` (normal end, `EXIT FOR`); `RETURN`/`FAIL`/errors run the
cleanup with the other active ones; an outer loop's `EXIT FOR`/`CONTINUE FOR`
unwinds through it. The "leaks, but the pattern is rare" comment is gone.
`try_inplace_set_assign`/`G7` still decline under such a loop (an in-place write
would change the block the iterator reads); their fallback is this reassignment.

**RED → GREEN.** `cargo test --release --test rt_for_each_self_update --
a_for_each_over_a_graph_list_frees_every_block_its_body_replaces`: before, every
case (append, replace, exit_for, continue_for, return, nested, nested_after_write)
printed the right values and leaked (e.g. append 105 allocated / 73 freed / 2976 B
live); after, all pass with alloc = free, 0 B live, 0 double frees.

**Goldens expected to move.** None: the new code is emitted only for a `FOR EACH`
over a plain, non-copied (graph-typed) local whose body assigns it, and a scan of
every `tests/**/*.mfb` and builtin `.mfb` source finds no such loop (the five
`FOR EACH x IN y` + `y = …` fixtures under `tests/rt-behavior/collections/` are
freeable-flat lists, which take the entry copy).
