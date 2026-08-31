# bug-473: a package-qualified enum name in a `CASE` escapes the front end and fails in NIR with an unlocated internal error

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (diagnostics)

Status: Open
Regression Test: `tests/syntax/net/qualified-enum-name-rejected/` (new)

Writing a package's enum with its package prefix — `net::PingStatus.Ok` rather
than the accepted `PingStatus.Ok` — is not caught by any front-end gate. It
survives resolution and type checking, reaches NIR, and dies with:

```
error: NIR local reference 'net.PingStatus' does not resolve
```

No file. No line. No error code. No caret. Nothing naming the enum, the `CASE`,
or the spelling that would work. It reads like an internal compiler assertion,
and the fix — deleting four characters — is not discoverable from it.

This is the same **class** as bug-466 (a field access on an un-imported package's
record escaping into `native plan has no storage class for type 'Unknown'`): a
name the front end cannot resolve is passed downward instead of refused, and the
back end reports it in its own vocabulary. Different trigger, same hole.

**The single correct behavior a fix produces:** `net::PingStatus.Ok` is refused
by the front end with a located, coded diagnostic naming the enum and, ideally,
the unqualified spelling that works. Nothing reaches NIR.

## Failing Reproduction

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

### The one-character-class difference that works

Drop the package qualifier and the identical program builds and runs:

```basic
    CASE PingStatus.Ok
    CASE PingStatus.Timeout
```

```
Building p108_ping (executable) for macos-aarch64
Wrote executable to /tmp/p108-ping/build/p108_ping.out
$ ./build/p108_ping.out
up in 0.10 ms
```

So the unqualified form is the accepted spelling — as `term` already documents
for its own enums (`LineStyle.Light`, `term/func_draw_box.rs:49`) — and the
qualified form is simply wrong. It just is not *reported* as wrong.

## Expected vs Actual

| | |
|---|---|
| Expected | a located, coded diagnostic at the `CASE` line naming `PingStatus` and the unqualified spelling |
| Actual | `error: NIR local reference 'net.PingStatus' does not resolve` — no location, no code, no name a developer recognises |

## Impact

Low frequency, high confusion. `pkg::Name` is the spelling for every *other*
package-scoped thing — functions, records, resource types — so reaching for it
on an enum is the natural mistake, and the language's own consistency invites it.
What the developer gets back gives no indication that they wrote something
wrong, let alone what.

It also shipped in the product's own documentation: `mfb man net ping` carried
`CASE net::PingStatus.Ok` in two of its three examples until plan-108-C corrected
them. Nobody noticed, because nothing compiles man examples (see the systemic
gap mfb-62 is filing).

## Suggested Fix

Resolution of a `pkg::Name` in a `CASE` scrutinee position should either

1. **resolve it** — accept `net::PingStatus.Ok` as a synonym for
   `PingStatus.Ok`, which would make enums consistent with every other
   package-scoped name and is arguably the better language answer; or
2. **refuse it** with a located diagnostic naming the enum and the accepted
   spelling.

Either is a large improvement on the current behaviour; (1) removes the trap
entirely, (2) is the smaller change. What must not remain is the unresolved name
reaching NIR.

Worth checking while fixing: whether the same escape happens for a qualified
enum name in other positions (a `LET` initialiser, a comparison, an argument),
and whether qualified *union* variant names behave the same way.

## References

- `src/codegen/builtins/net/func_ping.rs` — where the wrong spelling shipped in
  two man examples (corrected by plan-108-C).
- `src/codegen/builtins/term/func_draw_box.rs:49` — `LineStyle.Light`, the
  accepted unqualified form, in a page that always compiled.
- `bugs/bug-466-unknown-field-type-escapes-to-codegen.md` — the same class with a
  different trigger; a fix for either should consider the other.
- Found during: plan-108-C, compiling `net`'s man examples for the first time.
