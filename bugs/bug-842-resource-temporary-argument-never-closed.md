# bug-842: a resource produced inline in a non-statement expression (call argument, member read, list/record literal argument) is never closed

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/resources/<new>-temporary-argument-resource-closed-rt (proposed)

A resource returned by a producer call that is never bound with `RES` — because it
is used directly as a call argument (`useF(fs::open(p, "read"))`), read through a
member (`openTagged(p).state.pos`, the spec's own example), or placed in a list /
record literal passed as an argument (`useList([mk()])`, `useH(Holder[mk()])`,
`collections::forEach([Holder[mk()]], LAMBDA ...)`) — is **never closed**: not at the
statement, not at the enclosing scope exit, not at program end. No diagnostic, exit 0.
With `fs::File` this is a file-descriptor leak: a 20000-iteration loop of
`useF(fs::open(...))` dies with `7-702-0001 Read operation failed` under
`ulimit -n 256`, while the bound-then-passed form runs clean.

Class: CODE BUG. The single correct behavior: an unowned resource produced by an
expression that nothing claims is closed exactly once — at the end of the statement
that produced it (the same rule the spec already gives a discarded call statement) —
or, if the language prefers, the form is rejected at compile time. It must never leak.

References:

- `src/docs/spec/language/15_resource-management.md` line 13: "A resource is closed exactly once."
- same, line 3: "Resources are closed automatically by lexical drop (§14.7) when their cleanup scope exits, on every exit path"
- same, lines 51-53: "A call statement that discards a resource result closes it when nobody owns it (a resource the callee produced or released)"
- same, line 210: `LET here = openTagged(p).state.pos            ' \`.state\` resolves from the call too` (the shape leaks)
- same, line 357: "A resource element placed into a collection may be any reference to the resource — a named `RES` binding or a temporary alike."
- Found during spec-hardening batch B03 (topic language/15_resource-management).

## Failing Reproduction

Tracer resource: `strdup` produces, the close op `write(1, p, 4)` prints the name, so
every close is visible in stdout. Project needs
`"libraries": {"c":[{"os":"macos","type":"system","source":"libSystem.B.dylib"}]}`.

```
IMPORT io
IMPORT collections
RESOURCE Tr CLOSE BY c::release
LINK "c" AS c
  FUNC open(name AS String) AS RES Tr
    SYMBOL "strdup"
    ABI (name CString) AS p CPtr
    ERROR_ON p = NOTHING
    RETURN p
  END FUNC
  FUNC release(RES t AS Tr) AS Nothing
    SYMBOL "write"
    ABI (fd CInt32, t CPtr, n CInt64) AS status CInt64
    CONST fd = 1
    CONST n = 4
    SUCCESS_ON status = 4
  END FUNC
END LINK
FUNC mk(s AS String) AS RES Tr
  RETURN c::open("~" & s & "\n")
END FUNC
TYPE Cur
  pos AS Integer
END TYPE
TYPE Holder
  handle AS RES Tr
END TYPE
FUNC mkS(s AS String) AS RES Tr STATE Cur
  RES t AS Tr STATE Cur = mk(s)
  t.state.pos = 42
  RETURN t
END FUNC
FUNC useIt(RES t AS Tr) AS Integer
  RETURN 1
END FUNC
FUNC useList(xs AS List OF RES Tr) AS Nothing
  io::print("useList")
END FUNC
FUNC useH(h AS Holder) AS Nothing
  io::print("useH")
END FUNC
FUNC main() AS Nothing
  LET n = useIt(mk("T4"))
  useList([mk("T1")])
  useH(Holder[mk("T2")])
  LET z = mkS("Q1").state.pos
  io::print("z=" & toString(z))
  io::print("end")
END FUNC
```

`mfb build && ./build/<name>.out` (also with `-O0`):

- Observed:
  ```
  useList
  useH
  z=42
  end
  ```
  (no `~T4`, `~T1`, `~T2`, `~Q1` line ever — four resources never closed)
- Expected: each of `~T4`, `~T1`, `~T2`, `~Q1` printed exactly once (at the end of its
  statement, or at the latest at `main`'s exit).

fd-leak form (built-in `fs::File`, project /tmp/hardening/B03/resources/leakfd):

```
IMPORT io
IMPORT fs
FUNC useF(RES f AS fs::File) AS Integer
  RETURN 1
END FUNC
FUNC main() AS Nothing
  MUT n AS Integer = 0
  FOR i = 1 TO 20000
    n = n + useF(fs::open("/tmp/hardening/B03/resources/in.txt", "read"))
  NEXT
  io::print("n=" & toString(n))
END FUNC
```

`(ulimit -n 256; ./build/leakfd.out)` → `Error: 7-702-0001` / `Read operation failed.` exit 255.

Contrast cases that work today (all measured in the same session):

- `RES f AS fs::File = fs::open(...)` then `useF(f)` in the same loop → `n=20000`, exit 0 (leakfd2).
- A discarded call **statement** `fresh()` / `make()` → closed at the statement (`~FR` printed).
- `RES k AS Tr = ident(mk("Q3"))` → `~Q3` closed at scope exit.
- `LET xs AS List OF RES Tr = [mk("L1")]`, `LET hold = Holder[mk("H1")]` → closed at scope exit.

## Root Cause

`src/codegen/engine/control/builder_control.rs:lower_ops_inner`, arm `NirOp::Eval`:
only a *statement* whose value is a `Call`/`CallResult`/`RuntimeCall` reaches
`src/codegen/resource/owner.rs:emit_close_discarded_value`. A producer call nested
inside another expression (argument, member access, literal element that is itself an
argument) yields an unowned record (owner word `0`) that no binding claims
(`emit_resource_claim` runs only at a bind/assignment) and no cleanup is registered
for, so nothing ever closes it. Bound forms are immune because the bind claims the
record and registers an `ActiveCleanup`.

## Goal

- Every unowned resource produced by a sub-expression and not claimed by a binding,
  container bind, or `RETURN` is closed exactly once by the end of the enclosing
  statement; the repro prints all four tracer names once; leakfd survives
  `ulimit -n 256`.

### Non-goals (must NOT change)

- A resource the callee was handed and returned (live owner) must NOT be closed by the
  caller's temporary cleanup (`ident(b)` stays open; `RES k = ident(mk())` closes once).
- A temp that is claimed downstream (stored into a bound container, returned) keeps
  its float/return semantics.

## Blast Radius

- `NirOp::Eval` discard path — already correct (bug-755/bug-805).
- Call-argument producer temps, `.state`/member read on a producer call, literal
  aggregates passed as arguments — affected (measured above).
- Likely also: producer temps in `IF`/`WHILE` conditions and string concatenations
  (not measured — guess).

## Fix Design

Treat an unclaimed resource-carrying sub-expression value like a discarded result:
after the enclosing statement, run the `emit_close_discarded_value` owner-word check
(close only if owner is still `0`) over each such temporary. Rejected alternative:
a compile-time rejection — the spec explicitly shows `openTagged(p).state.pos` as legal.

## Validation Plan

- Regression test: tracer fixture above as an rt-behavior golden; fd-loop variant.
- Doc sync: §15 should state the statement-end rule for unclaimed temporaries.

## Summary

Silent resource leak on a common, spec-endorsed shape; the risk is in not
double-closing a temp later claimed by an owner — the owner-word check already
provides the guard.
