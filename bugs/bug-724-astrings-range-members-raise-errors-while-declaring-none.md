# bug-724: `astrings` range and index members declare no errors, yet raise `ErrInvalidArgument` and `ErrIndexOutOfRange`

Last updated: 2026-09-16
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the implementation)

Status: FIXED (2026-09-27)
Regression Test: `codegen::engine::tests::builtins_codegen::declared_errors_all::
every_astrings_member_declares_exactly_the_errors_its_lowering_raises` (part of
bug-698's generalized declared-errors sweep)

Every `astrings` member is registered with `errors: vec![]`
(`grep -n 'errors: vec' src/codegen/builtins/astrings/func_*.rs`), so no `astrings`
man page renders an Errors table. Four members raise errors of their own, and their
pages say so only in prose:

| Member | Raises | Evidence |
|---|---|---|
| `addAttribute`, `removeAttribute`, `clearAttributes` (three-argument form) | `ErrInvalidArgument` (`77050002`, "invalid attribute range") when `start` is negative or greater than `endIndex` | `helper_validate_range.rs:__astrings_validateRange`; probe `/tmp/p125-ex/astrtie`: `addAttribute(a, -1, 2, …)` and `addAttribute(a, 3, 2, …)` → `77050002` |
| the same three | `ErrIndexOutOfRange` (`77050001`, "attribute range out of bounds") when `endIndex` is past the visible text, including `0, 0` on empty text | same helper; `astrtie`: `addAttribute(a, 0, 5, …)` on `"hello"` and `addAttribute("", 0, 0, …)` → `77050001` |
| `getAttributes` | `ErrIndexOutOfRange` (`77050001`, "attribute index out of bounds") | `func_get_attributes.rs:49` |

This is the `astrings` instance of the class already filed for `encoding` (bug-606),
`udp` (bug-608), `math` per-overload (bug-617), `collections` (bug-641) and `io`
(bug-722).

**The single correct behavior a fix produces:** each `astrings` member declares
exactly the errors it can raise, so the pages for `addAttribute`, `removeAttribute`,
`clearAttributes` and `getAttributes` render an Errors table. Members that cannot
raise (the attribute constructors, for example) keep `errors: vec![]`. For
`clearAttributes`, only the three-argument overload declares the range errors.

Found by plan-125-D Phase 2 while triaging the `astrings::addAttribute` and
`astrings::clearAttributes` reviews.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT astrings
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET x AS AttributedString = astrings::addAttribute(a, 0, 5, astrings::bold()) TRAP(err)
    io::print("raised " & toString(err.code))
    EXIT SUB
  END TRAP
  io::print("ok")
END SUB
```

Observed: prints `raised 77050001`, while `./target/release/mfb man astrings
addAttribute` renders no Errors section.

## Root cause

Each descriptor's `errors` list is written by hand. The range check lives in the
shared helper `__astrings_validateRange`, and its two `FAIL` sites were never
reflected back into the declarations of the members that call it.

## Non-goals

- Changing what any `astrings` member raises or the error codes it uses.
- Declaring `ErrOutOfMemory` rows beyond whatever the project-wide rule for
  memory-needing members settles on (see bug-641).

## Blast-radius audit

- The `strings::` `AttributedString` overloads (`left`, `mid`, `replace`, the padding
  functions, …) run `astrings` helpers; check whether their declared errors match
  the `String` overloads' and whether any helper adds a raise.
- `readSpans`/`writeSpans` (internal) and `toMarkdown`: confirm they cannot raise.

## Fix (landed via bug-698)

`addAttribute`, `removeAttribute`, `clearAttributes` and `getAttributes` are
`Body::Mfb` — bug-698's native-lowering sweep does not check a `Body::Mfb`
member's declared errors (its separate `Body::Mfb` guard is not built; see
that bug's report). Fixed directly from this doc's own hand-verified evidence
(`helper_validate_range.rs`'s two `FAIL` sites, `func_get_attributes.rs`'s
own): `addAttribute`/`removeAttribute`/`clearAttributes`'s three-argument
(ranged) overload now declare `errors: vec!["ErrIndexOutOfRange",
"ErrInvalidArgument"]`; `clearAttributes`'s two-argument (whole-string)
overload correctly keeps `errors: vec![]`; `getAttributes` declares
`errors: vec!["ErrIndexOutOfRange"]`. Every code was already documented in
prose.

`fromString`/`writeSpans` construct an `AttributedString` record, which this
sweep's bare `TypeModel::empty()` harness has no layout for — skipped with a
documented reason and a repro, the same fixture gap as `canvas`'s
`KeyPair`/`Paint`.

`readSpans` (internal_only) declared no errors despite its lowering raising
`ErrOutOfMemory`; declared it.

Commit: (this worktree, bug-698 branch)
