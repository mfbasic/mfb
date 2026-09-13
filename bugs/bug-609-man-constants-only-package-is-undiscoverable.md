# bug-609: `mfb man errorCode` lists none of its 52 constants, and its types page says "list its functions"

Last updated: 2026-09-13
Effort: small–medium
Severity: LOW
Class: Documentation (renderer)

Status: Open
Regression Test: none yet — see Phase 1

`errorCode` is a constants-only package: `src/codegen/builtins/errorcode/mod.rs:register`
registers 52 `Err*` constants (`add_constant`), each with a value and a message,
and nothing else. The man renderer shows none of them:

```
mfb man errorCode --all              # the overview only — no constant names or values
mfb man errorCode ErrPathNotFound    # error: unknown errorCode function
mfb man errorCode types              # "The `errorCode` package has no public types.
                                     #  Run `mfb man errorCode` to list its functions."
```

The last line points to a page that, per its own overview, "exports no functions".
The overview routes to `mfb spec diagnostics error-codes` for the mapping, but a
developer at `mfb man` cannot see which names exist. The same gap hides
`vector`'s 42 record constants (`vector::zeroFloat3`, …), whose overview now spells
the naming rule out by hand (plan-125-B Phase 3).

**The single correct behavior a fix produces:** a package's registered constants
are listed on its `mfb man <pkg>` page (name, value, and description where
present), and the no-types fallback does not tell the reader to list functions
the package does not have.

Found by plan-125-B Phase 3's Codex review of `errorCode`
(`planning/plan-125-findings/B-phase3/man-pkg-errorCode.md`, findings 1–2).
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes").

## Reproduction

The three commands above, at `worktree-P-125` HEAD.

## Root cause

- `src/cli/man.rs` renders functions, records, unions, enums and resources, but
  has no section for `RegistryConstant` (`grep -n -i constant src/cli/man.rs`
  finds no rendering site).
- The no-types fallback at `src/cli/man.rs` (the `"has no public types.\n\nRun
  `mfb man {}` to list its functions."` string) is unconditional.

## Non-goals

- Changing constant values or names.
- Hand-maintaining a constants table in overview prose, which would drift from the
  registry.

## Blast-radius audit

- Every package that calls `add_constant`: `errorCode` (52), `vector` (42), and any
  other (`grep -rn add_constant src/codegen/builtins`) gains a section.
- Tests pinning the rendered `errorCode`/`vector` pages will shift.

## Fix

Phase 1 — renderer tests: `mfb man errorCode` lists `errorCode::ErrPathNotFound`
and its value; the types fallback for a package with no functions does not say
"list its functions" (RED). Commit:

Phase 2 — render a Constants section from the registry; word the fallback by what
the package has (GREEN); update proven-wrong pins. Commit:
