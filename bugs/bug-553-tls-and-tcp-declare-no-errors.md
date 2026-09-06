# bug-553: `tls` and `tcp` declare `errors: vec![]` on 28 members that raise

Last updated: 2026-09-05
Effort: medium (the audit is the work — 28 members' real raise sets)
Severity: MEDIUM (documentation correctness; NOT a miscompile risk today — see "Why this is not urgent")
Class: Registry metadata / documentation correctness

Status: Open

## The finding

    $ grep -c 'errors: vec!\[\]' src/codegen/builtins/tls/*.rs src/codegen/builtins/tcp/*.rs
    28   # 13 of 15 tls members, 15 of 16 tcp members

Both packages raise in normal operation — `ErrConnectionClosed`, `ErrTimeout`,
`ErrTlsFailed`, `ErrNetworkFailed` are all reachable from ordinary calls, and
bug-483 (`a5ba6bc80`) has just changed which one `write` gives on a departed
peer — yet almost every member declares that it raises nothing.

The declaration is what `mfb man <pkg> <member>`'s **Errors** section renders, so
the shipped documentation understates the failure surface of the two packages a
network program depends on most.

## Why this is NOT urgent, and why that matters for how it is fixed

An undeclared error is what caused three dead-handler MISCOMPILES this month —
`toString` (bug-486), `replace` (bug-533) and `strings::left`/`right`/`padLeft`/
`padRight` (`6d9f4b79a`), where `inline_builtin_is_infallible` proved a raising
member infallible, the compiler warned `TYPE_INLINE_TRAP_DEAD_HANDLER`, **deleted
the live handler**, and the program aborted.

**That cannot happen here.** `inline_builtin_is_infallible` reaches its
registry-data branch through `native_bare_target`, which returns `None` for an
`abi_function` body — and every `tls`/`tcp` member is one. So the empty lists are
inert for the infallibility verdict; they are wrong in the docs, not in codegen.

Confirming that before fixing matters, because it changes the shape of the fix: a
**wrong** error list is worse than an empty one. An empty list understates; a
wrong list would state a member raises something it cannot, and — for any member
that ever moves to an inline-lowerable body — would feed the verdict bad data.

## What a fix must produce

For each of the 28 members, the set of errors it can actually raise, derived from
its lowering rather than from its prose. The three sibling packages that already
do this correctly (`fs`, `process`, `udp` — check which) are the model.

Two properties worth pinning once the lists exist:

- Every error a member's lowering can raise appears in its `errors:` list.
- No member declares an error its lowering cannot raise.

The second is the one an audit gets wrong, and neither is checked today: the
assertion that would catch a mismatch is a `debug_assert!` in `raise_error_bare`,
which never executes because CI builds release (**bug-550**).

## Non-goals

- Do not guess a list from the man-page prose; the prose is what is being
  corrected.
- Do not change any runtime behaviour. This is metadata only; every `.run` golden
  must stay byte-identical.

## Blast radius

`mfb man tls <member>` / `mfb man tcp <member>` Errors sections for 28 members,
and the `.ir`/`.ncodesum` goldens of every `tls`/`tcp` importer (filling a
registry field shifts the synthesized companion source's line numbers).

References: `src/codegen/builtins/tls/`, `src/codegen/builtins/tcp/`;
`src/codegen/builtins/mod.rs:inline_builtin_is_infallible` and
`native_bare_target` (why this is inert today); **bug-550** (the assertion that
would police it never runs); `bugs/completed/bug-483-*` (`a5ba6bc80`), which found
this while measuring the three backends' write-to-departed-peer codes.

Found by the bug-483 fix, which recorded it rather than expanding scope: 28
members is its own audit, and the packages are a mirror pair, so the two must be
corrected together or they diverge.
