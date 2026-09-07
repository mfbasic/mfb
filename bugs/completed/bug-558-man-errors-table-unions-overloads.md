# bug-558: a member's man page merges every overload's errors into one table, so it names errors the overload you are calling cannot raise

Last updated: 2026-09-06
Effort: small-to-medium (the renderer change is small; deciding the LAYOUT is the work)
Severity: LOW-MEDIUM (documentation correctness; no miscompile — see "Why this is not urgent")
Class: Documentation correctness / man renderer

Status: **FIXED** (2026-09-06, `b1b336077`)
Regression Test: `src/cli/man.rs` —
`a_multi_overload_errors_table_names_the_overloads_that_raise_each_error`,
`every_errors_overload_number_names_a_rendered_signature`,
`an_overload_that_declares_no_errors_is_named_as_infallible`, and the two
containment pins `a_single_overload_member_errors_table_is_unchanged` and
`a_package_overview_errors_table_has_no_overload_column`

## Correction to the blast radius (measured by RENDERING, 2026-09-06)

The "8 members" census counted `errors:` lines per FILE, and three of the four
`canvas` entries are not multi-overload members at all:

- `canvas::groupStats` and `canvas::sceneHashes` render NO page — the several
  `errors:` lines in `func_group_stats.rs` / `func_scene_hashes.rs` belong to
  separate `internal_only` members that share one file.
- `canvas::loadFont` is a SINGLE-overload member; its file's second `errors:`
  belongs to the `internal_only` `fontFromBytes`.

Five of the eight render an affected page: `tcp::poll`, `tcp::write`,
`tls::poll`, `tls::write`, `canvas::getSize` — and those are exactly the five
whose new Overloads column varies from row to row. `canvas::getSize` is the
one-overload-declares-nothing shape, and its page now names the infallible
signature outright ("Overload 2 raises no errors.") rather than leaving it
absent from a two-row table.

Rendered delta over all 605 pages `mfb man` produces: exactly the 94 pages with
an Overloads section change (55 grow the column; 39 change only in that their
signatures are numbered). All 449 single-overload function pages, all 31
overviews and all 31 types pages are byte-identical.


## USER DECISION (2026-09-06) — number the overloads, add a column

Ruling on the three layout options: **none of them.** Instead —

> number the overloaded functions, add a column and list the number(s) the
> error(s) apply to.

So the **Overloads** section numbers its signatures (1., 2., …) and the **Errors**
table grows a column naming which numbered overload(s) each error belongs to.
That is better than the three options as written: it keeps ONE table (so a page
does not change shape with its data, which was option 1's cost), it needs no
heading level for the single-overload majority (option 2's cost), and the
cross-reference is a short number rather than a repeated signature (option 3's
cost).

Consequences a fix owns:

- The numbering must be **stable and shared**: the Errors column is meaningless
  unless its numbers are the same ones the Overloads list prints, produced by one
  enumeration rather than two.
- For a **single-overload** member the column carries no information. Suppress it
  there rather than printing "1" on every row — otherwise every page in the
  corpus churns for nothing.
- `a_member_with_no_declared_errors_omits_the_errors_section` must keep passing:
  no errors declared still means no Errors section at all.
- The renderer is the ONLY instrument that sees this. Render the affected pages
  (`tcp::poll`, `tcp::write`, `tls::poll`, `tls::write`, `canvas::getSize`,
  `canvas::groupStats`, `canvas::loadFont`, `canvas::sceneHashes`) and read them.

## The finding

`cli/man.rs::function_errors` renders the **union** of `errors:` across every
implementation of a member, under a single **Errors** heading, while the
**Overloads** section above it lists the signatures separately. A reader has no
way to tell which of the listed errors belongs to which signature.

Found while landing bug-553, which is what made it visible: before that, `tcp`
and `tls` declared `errors: vec![]` on almost everything, so there was nothing
to merge.

    $ mfb man tcp poll
    Overloads
      tcp::poll(sock AS tcp::Socket, [timeoutMs AS Integer]) AS Boolean
      tcp::poll(socks AS List OF RES tcp::Socket, [timeoutMs AS Integer]) AS tcp::Socket
    Errors
      77010001  ErrOutOfMemory      Allocation failed.
      77030004  ErrResourceClosed   Resource handle is already closed.
      77050002  ErrInvalidArgument  Argument value is not valid …
      77050008  ErrTimeout          Operation did not complete before its deadline.

The descriptors are per-overload EXACT
(`src/codegen/builtins/tcp/func_poll.rs:140,154`):

| overload | declares |
| --- | --- |
| `poll(sock, …) AS Boolean` | `ErrInvalidArgument`, `ErrResourceClosed` |
| `poll(socks, …) AS tcp::Socket` | `ErrInvalidArgument`, `ErrOutOfMemory`, `ErrResourceClosed`, `ErrTimeout` |

So the page tells a reader of the **scalar** form — the common one, and the one
listed first — to handle `ErrTimeout` and `ErrOutOfMemory`. It raises neither.
`ErrTimeout` is the sharp one: readiness on a scalar socket is a QUERY, and an
expired wait is `FALSE`, not a raise. A program written from this page has a
handler that can never run, which is the shape `TYPE_INLINE_TRAP_DEAD_HANDLER`
exists to catch in the compiler and which nothing catches in prose.

## Blast radius — measured, 8 members

    $ # files declaring >1 overload with DIFFERING error sets
    8 of the multi-overload members across src/codegen/builtins/

    canvas::getSize [2,0]     canvas::groupStats [0,0,0,0,1,0,1,0,0]
    canvas::loadFont [1,2]    canvas::sceneHashes [2,1]
    tcp::poll [2,4]           tcp::write [4,3]
    tls::poll [5,6]           tls::write [7,6]

`canvas::getSize` and `canvas::groupStats` are the worst shape: one overload
declares errors and another declares NONE, so the page shows an Errors table for
a signature that is infallible.

## Why this is NOT urgent, and why that shapes the fix

The `errors:` field feeds `mfb man` and the compiler's own
`inline_builtin_is_infallible` verdict, and the DESCRIPTORS are already correct
per-overload — bug-553 verified each against its lowering. Only the RENDERER
merges them. So nothing is miscompiled and no error list needs re-deriving; this
is one function in `cli/man.rs`.

That also means the tempting minimal fix — "just show the union, it is a
superset" — is exactly what the code does today and what this bug is about.

## What a fix must produce

A reader can tell which errors belong to the signature they are calling.

The layout is a real choice and should be decided before coding:

1. **A per-overload Errors table**, keyed the way the Overloads section already
   keys the signatures. Most precise; makes a single-overload member's page
   (the large majority) grow a heading level it does not need.
2. **One table with an "Overloads" column** naming which signatures raise each
   error. Keeps one table; the column is empty noise for single-overload members
   unless suppressed.
3. **One table, with per-overload rows only when the sets DIFFER** — identical
   sets keep today's rendering. Smallest diff to the corpus of pages, at the cost
   of a page whose shape depends on its data.

Whichever wins, the renderer must keep rendering nothing when a member declares
no errors at all (`a_member_with_no_declared_errors_omits_the_errors_section`
pins that today).

## Non-goals

- Changing any `errors:` descriptor. They are correct; bug-553 derived all 34
  `tcp`/`tls` implementations from their lowerings, including the two
  parameter-gated cases (`lower_net_read_helper`'s `ErrEncoding` under `if text`,
  `lower_net_write_helper`'s `ErrInvalidArgument` under `if !text`) that make the
  overloads legitimately differ.
- Making overloads declare the same errors so the union is honest. That would be
  re-deriving prose from the renderer's convenience, which is backwards.

## References

- `src/cli/man.rs` — `function_errors`
- `src/codegen/builtins/tcp/func_poll.rs:140,154`
- `bugs/completed/bug-553-*` — where the per-overload lists came from
