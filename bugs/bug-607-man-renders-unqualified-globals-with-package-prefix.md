# bug-607: `mfb man testing` / `mfb man general` render unqualified globals as `testing::expectEqual` and `general::len`

Last updated: 2026-09-13
Effort: small–medium
Severity: LOW
Class: Documentation (renderer)

Status: Open
Regression Test: none yet — see Phase 1

`testing` and `general` are *unqualified-global* packages: their members are
written as bare names (`expectEqual(a, b)`, `len(s)`), and a package-qualified
spelling is not something a developer can type. The `testing` overview says so
itself — "you write them as bare names (`expectEqual(actual, expected)`), never
`testing::expectEqual`" — and `src/cli/man.rs:render_all_markdown` already skips
both packages in `mfb man --all` for exactly that reason.

But their own pages render every Functions-table row and every Declaration in the
qualified form:

```
mfb man testing expectEqual   # `testing::expectEqual(actual AS T, expected AS T) AS Nothing`
mfb man general --all         # │ general::len │ …   and   `general::len(value AS String) AS Integer`
```

**The single correct behavior a fix produces:** for an unqualified-global
package, the Functions table and every Declaration/Overloads line use the
callable bare name (`expectEqual(...)`, `len(...)`). The package name remains
only as the documentation grouping in the page header.

Found by plan-125-B Phase 3's Codex review of `testing`
(`planning/plan-125-findings/B-phase3/man-pkg-testing.md`, finding 1). Checking
it showed `general` renders the same way. **Filed, not fixed**, by user
instruction during a documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```
mfb man testing expectEqual | grep 'expectEqual('
mfb man general len | grep 'len('
mfb man testing | grep 'testing::'
```

Observed at `worktree-P-125` HEAD: the qualified `testing::`/`general::`
spellings. Expected: bare names.

## Root cause

Hypothesis, to confirm in Phase 1: the function-page and package-page renderers
in `src/cli/man.rs` format every member as `{pkg}::{name}`, with no branch on the
package's `unqualified_global` flag. `render_all_markdown` consults that flag,
but only to skip the whole package. Confirm by grepping for the format site and
for `unqualified_global` uses in `src/cli/man.rs`.

## Non-goals

- Changing how the members resolve or which spelling the compiler accepts.
- Removing the pages or the `mfb man testing` / `mfb man general` entry points.

## Blast-radius audit

- Every renderer site that prints a qualified member name: the Functions table,
  Declaration, Overloads, See also, and the `mfb man <pkg>` index row. Enumerate
  in Phase 1.
- `scripts/man-census.sh` and any test that pins a rendered `general::`/`testing::`
  line — e.g. `tests/cli/cli_man_summary_plain.rs` — will shift with the fix.

## Fix

Phase 1 — a renderer test asserting that `mfb man testing expectEqual` shows
`expectEqual(` and no `testing::` (RED); enumerate the sites. Commit:

Phase 2 — render bare names for unqualified-global packages (GREEN); update the
proven-wrong pins; run the `cli::man` tests. Commit:
