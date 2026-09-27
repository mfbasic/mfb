# bug-750: a user function that hands back a handle it was passed gets that handle closed at the caller's inner scope

Last updated: 2026-09-27
Effort: large
Severity: HIGH
Class: Correctness (resource closed without the program asking)

Status: Open
Regression Test: none yet — see Fix

Found by the bug-643 lane. bug-643 fixed the `collections` helpers; this is the same
adoption defect for a **user** function, which bug-643's static rule cannot see
through.

A binding of a call that returns a resource (`RES y = f(…)`) or a resource
collection (`LET p = f(…)` of `List OF RES T`) ADOPTS what comes back: its scope
closes it once at exit (§15.6, "the caller's binding scope adopts them"). That is
right for a handle the callee produced. When the callee hands back a handle its
caller passed in, the handle already has an owner that encloses the binding, and
a binding in an INNER scope closes it when that scope exits — before its owner does.

## Reproduction

```basic
IMPORT io
IMPORT fs
IMPORT errorCode

FUNC state(RES h AS fs::File) AS String
  LET line = fs::readLine(h) TRAP(e)
    IF e.code = errorCode::ErrResourceClosed THEN
      RECOVER "CLOSED"
    END IF
    RECOVER "open"
  END TRAP
  RETURN line
END FUNC

FUNC ident(RES h AS fs::File) AS RES fs::File
  RETURN h
END FUNC

FUNC same(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN xs
END FUNC

SUB main()
  fs::writeText("/tmp/b750.txt", "")
  RES a AS fs::File = fs::open("/tmp/b750.txt", "read")
  IF TRUE THEN
    RES y AS fs::File = ident(a)
  END IF
  io::print("a: " & state(a))
  RES b AS fs::File = fs::open("/tmp/b750.txt", "read")
  LET lb AS List OF RES fs::File = [b]
  IF TRUE THEN
    LET p = same(lb)
  END IF
  io::print("b: " & state(b))
END SUB
```

Observed (macos-aarch64, `a0082e4ae` and after bug-643): `a: CLOSED`, `b: CLOSED`.
Expected: `a: open`, `b: open`.

The same holds for a mixed return (`RETURN collections::append(xs, fs::open(p))`,
where the caller must adopt the fresh handle but not `xs`'s), and for a
`collections` helper over another call's result (`take(makeFiles(), 1)` keeps
adopting, by bug-643's rule, because that call's handles may be fresh).

## Root cause

`src/codegen/engine/control/builder_control.rs`, the `NirOp::Bind` lowering:
a resource binding from a call registers an owning `ActiveCleanup::Resource`, and a
`List OF RES` binding from a call seeds an owned-list from every element
(`emit_owned_list_seed_from_collection`). Neither knows which of the returned
handles the callee produced. The callee knows — the obligations it transfers are
exactly the `RETURN`-deactivated cleanups and owned-lists — but nothing carries
that across the call.

## Fix — needs a design decision

Options, none small:

1. **Runtime exclusion by argument reachability.** At the adopting bind, skip every
   returned handle reachable from a resource-carrying argument of the call (a named
   local or parameter): such a handle's owner encloses the binding. Needs a walker
   over every resource-carrying type (resources, resource unions, lists, maps,
   records, and recursive types, which need a runtime per-type walk like
   `_mfb_rt_graph_copy`), and costs O(result × argument) per call.
2. **Transfer channel.** The callee's `RETURN` publishes the obligations it
   transfers (its owned-list, or the one handle) in a per-thread slot, and every
   call site of a resource-returning function consumes the slot immediately:
   a binding adopts exactly those, a discarded temporary drains them. Precise and
   O(transferred), but an ABI change across every resource-returning call,
   including `LINK` thunks and packages.
3. **Obligation counting** on the resource record: every registered obligation
   increments, every drain/cleanup decrements and closes at zero, so the outermost
   owner closes. Touches every resource cleanup path.

Recommended: 2 (it is what §15.6 already describes — "releases the close
obligations for the referenced resources" — made literal), as a plan.
