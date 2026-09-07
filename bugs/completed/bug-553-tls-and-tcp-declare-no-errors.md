# bug-553: `tls` and `tcp` declare `errors: vec![]` on 28 members that raise

Last updated: 2026-09-06
Effort: medium (the audit is the work — 28 members' real raise sets)
Severity: MEDIUM (documentation correctness; NOT a miscompile risk today — see "Why this is not urgent")
Class: Registry metadata / documentation correctness

Status: **FIXED** (2026-09-06, `0fbccd28d`)

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

**None of them is.** Measured over the whole `src/codegen/builtins/` tree:

    $ for d in src/codegen/builtins/*/; do ... grep 'errors: vec!' ...; done
    fs       empty=40  nonempty=1     # the 1 is fs::close
    process  empty=15  nonempty=0
    udp      empty=9   nonempty=1     # the 1 is udp::close

Every one of the three declares errors on its `close` member and nowhere else,
which is the same shape `tcp`/`tls` were already in. There was no model to copy;
the audit had to be done from the lowerings, and the condition is tree-wide
(21 of 32 packages carry at least one `errors: vec![]`) rather than specific to
this pair. The pair is the most visible instance, not the only one.

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

## Fixed

Twenty-eight `errors: vec![]` literals — thirty registry implementations, because
`tcp`/`tls` `localAddress` each build their two overloads from one shared
`overload()` helper — replaced with the set each member's lowering can raise.
The union of that with the four already-correct `close` rows is pinned by
`codegen::builtins::tcp_tls_error_declaration_tests`, whose two tables carry the
derivation for every row.

`mfb man` went from **2 of 22** members rendering an Errors section to **22 of
22**, and every one of the twenty diffs is purely additive (`diff | grep -c '^<'`
is 0 for all of them). `tcp::close` and `tls::close` render byte-identically —
they were already right, and the audit did not churn them.

### The method, and the two places a closure alone gets it wrong

`emit_fail(symbol, "ErrX", …)` is the single raise primitive in both packages and
the shared socket layer, so a call-graph closure from each member's lowering entry
gives a candidate set. Keyed by **(file, function)**, never by bare name — the two
packages share `lower_read`, `lower_write`, `lower_close`, … and a bare-name map
merges them into the claim that `tcp::close` raises `ErrTlsFailed`. Running the
closure a second time with every ambiguous unqualified call resolved to *all*
candidates (1,300+ reachable functions instead of 27–89) produced the **identical**
error set for all 22 members, which is what says the strict resolution was not
silently dropping edges.

That closure is an over-approximation, in two ways that each cost a wrong row:

1. **A raise site inside a Rust `if` on a lowering parameter is not in every
   caller's set.** `lower_net_read_helper`'s `ErrEncoding` is inside `if text`
   and `tcp::read` passes `text = false`, so the closure's `tcp::read` row claimed
   an error the member cannot raise. Likewise `lower_net_write_helper`'s
   `ErrInvalidArgument` (`if !text` — the bug-497 payload-header check, so the
   `String` overload does not have it), and `lower_net_endpoint_helper`'s
   `if listen` split: `ErrAddressNotFound` vs `ErrAddressInvalid` are its two
   arms, `ErrInvalidArgument` is connect-only, and its `ErrTimeout` label is
   *emitted* on the listen path but nothing branches to it — so `tcp::listen`
   raises three errors, not six.
2. **A helper that `bl`s another by SYMBOL is an edge no Rust call expression
   shows.** `lower_tls_poll_list_helper` branch-links `_mfb_rt_tls_tls_poll` and
   propagates its error, so the list overload's set is the scalar's plus
   `ErrInvalidArgument`/`ErrTimeout` — while the scalar overload has no
   `ErrTimeout` at all, because readiness is a query and an expired deadline is a
   `FALSE`.

### The earlier groundwork's `tcp::close` gap does not exist

The 2026-09-05 note recorded `tcp::close` as proof that a second, non-`emit_fail`
contribution (a generic resource guard) had to be characterised before the audit
could finish. It does not: all three of `ErrResourceClosed`, `ErrResourceMoved`
and `ErrCloseFailed` come from `emit_fail` in
`fs::gen_handle::lower_fs_close_helper`, and the reason a bare-name scan missed
them is the same name-collision trap it recorded elsewhere. The one thing that
helper contributes and `tcp::close` does *not* declare is `ErrWriteFailed` —
which sits inside `if flush_on_close`, and `tcp`/`udp` pass `false`. So
`tcp::close`'s pre-existing three-error list was already exactly right, and it is
the standing proof that the parameter-gated reading is the correct one rather
than evidence of a missing emitter. (`ErrResourceMoved` is checked in exactly
three places tree-wide — that close helper, the `LINK` thunk, and scope cleanup —
so no other `tcp`/`tls` member can raise it.)

### Why no golden moved

The registry `errors` field feeds `cli/man.rs`'s Errors table, the `raise_error`
`debug_assert!`, and the `errorcode` name-validity test. It does **not** feed
codegen: `RegistryPackage::get_mfb` renders records/unions/enums/helpers and
`Body::Mfb` bodies only, and `tcp`/`tls` have none of those, so their synthesized
companion source is empty and no line number can shift. The per-package
`_mfb_str_error_*` message pool in `data_objects.rs` is a separate hand-maintained
list keyed on call names, untouched here. Measured: `artifact-gate.sh all` and
`test-accept.sh` both clean, zero `.run`/`.ir`/`.ncodesum` goldens moved.

## Follow-up, not fixed here

`cli/man.rs::function_errors` renders the **union** over a member's overloads, so
`mfb man tcp poll` now lists `ErrTimeout`/`ErrOutOfMemory` even though only the
list overload can raise them. That is pre-existing renderer behaviour shared with
every other multi-overload member in the tree (`process::spawn`, `csv::parse`, …),
the descriptors themselves are now per-overload exact, and both `poll` pages'
prose already draws the distinction ("unlike the scalar form there is no value
that could mean *nothing*"). Making the table per-overload is a man-renderer
product decision, not part of this correction.
