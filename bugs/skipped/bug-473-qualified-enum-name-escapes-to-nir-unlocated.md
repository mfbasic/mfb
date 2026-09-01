# bug-473: a package-qualified enum name in a `CASE` escapes the front end and fails in NIR with an unlocated internal error

Last updated: 2026-08-31
Effort: —
Severity: —
Class: Correctness (diagnostics)

Status: **RETIRED 2026-08-31 — superseded by bug-480.** Not fixed, not invalid:
absorbed. The defect is real and still reproduces at 744c7c175; it is now
Defect B of `bugs/bug-480-package-name-resolution.md`, which owns the fix.

## Why it was retired

The bug was filed as an enum-specific diagnostics problem: `CASE net::PingStatus.Ok`
dies with an unlocated `error: NIR local reference 'net.PingStatus' does not resolve`,
and the accepted spelling was the bare `PingStatus.Ok`.

Investigating it inverted the framing twice.

1. **The qualified form is not the mistake — it is the required spelling.** The
   language rule is two lines and covers every kind of name: *defined locally, no
   prefix; imported, prefix required.* So this bug is not "a wrong spelling is
   reported badly", it is "the correct spelling does not work".
2. **It is not about enums.** Records, unions, union variants and enum members
   are all in one flat, package-blind namespace. The rule already holds for
   functions, constants and resource types; value types are the whole remaining
   gap, and splitting them across separate tickets would have split one fix.

Both defects are the same missing package dimension on names, so they merged into
one document rather than three cross-referencing ones.

## Where the material went

| what | now lives in |
|---|---|
| the reproduction below, and the NIR escape | bug-480, Defect B |
| the two-line rule, its clarifications, the compliance table | bug-480, "The Governing Rule" |
| the bug-441/plan-97 lineage | bug-480, "The Governing Rule" |
| the 1139 + 339 corpus census | bug-480, Blast Radius |
| the Phase 1 / Phase 2 split | bug-480, Phases |
| `IMPORT http` + `IMPORT process` failing to build | `bugs/bug-481-http-and-process-cannot-be-imported-together.md` (still open on its own) |

## The original reproduction, preserved

macos-aarch64, `target/release/mfb`. A plain `mfb init` project.

```basic
IMPORT net
IMPORT io

FUNC main AS Integer
  LET result = net::ping("127.0.0.1", 1000)
  MATCH result.status
    CASE net::PingStatus.Ok
      io::print("up in " & toString(result.rttMs) & " ms")
    CASE net::PingStatus.Timeout
      io::print("no answer")
    CASE ELSE
      io::print("unreachable")
  END MATCH
  RETURN 0
END FUNC
```

```
$ mfb build /tmp/p108-ping
Building p108_ping (executable) for macos-aarch64
error: NIR local reference 'net.PingStatus' does not resolve
```

No file. No line. No error code. No caret. Dropping the qualifier builds and
runs — which is the inversion described above: the spelling that works is the one
the rule forbids.

The same shape for a union variant: `CASE json::JsonBool` gives
`error: NIR local reference 'json.JsonBool' does not resolve`.

## References

- `bugs/bug-480-package-name-resolution.md` — the successor; owns the fix.
- `bugs/bug-481-http-and-process-cannot-be-imported-together.md` — the same root
  cause as a hard build failure.
- `bugs/bug-466-unknown-field-type-escapes-to-codegen.md` — the same class
  (an unresolved name passed downward instead of refused).
- `src/codegen/builtins/net/func_ping.rs` — where the qualified spelling shipped
  in two man examples (rewritten to the bare form by plan-108-C; under the rule
  the original was right and the correction was wrong, so plan-108-C's edit must
  be reverted as part of bug-480's Phase 2 sweep).
- Found during: plan-108-C, compiling `net`'s man examples for the first time.
