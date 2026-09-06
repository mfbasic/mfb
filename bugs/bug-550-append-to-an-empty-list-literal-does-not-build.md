# bug-550: `collections::append([], x)` type-checks and then fails to build

Last updated: 2026-09-06
Effort: unknown — small if the declared binding type simply needs to reach the
literal; medium if the empty literal's element type is inferred somewhere that
does not know it yet
Severity: MEDIUM
Class: Correctness (valid program does not build) / Diagnostics

Status: **OPEN.** Reproduced with the release compiler, and attributed: it
reproduces identically on `main` (built from `git archive main` into a clean
`/tmp` tree, so it is not this branch's).

## Reproduction

```basic
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET xs AS List OF Integer = collections::append([], 1)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
```

```
$ mfb build .
Building probe_emptyappend (executable) for macos-aarch64
error: native collection list item must be Unknown, got Integer while lowering bind xs AS List OF Integer
```

The front end accepts it. The failure is at lowering, from
`src/codegen/builtins/collections/gen_mutate.rs:110`
(`collection_argument_as_list_slot`):

```rust
if item.type_ != *element_type {
    return Err(format!(
        "native collection list item must be {}, got {}",
        element_type, item.type_
    ));
}
```

The empty literal `[]` reaches codegen with element type `Unknown`, the item is
`Integer`, and the two do not match. The declared binding type
(`List OF Integer`) never propagated into the literal.

## Why this is a bug and not a limitation

`[]` is the ordinary way to write the empty list, and every other position
accepts it against a declared type — `LET xs AS List OF Integer = []` builds,
and so does `Nest[Leaf[0], [], ...]` for a `List OF Shape` field. It is only
`append`'s (and, by the shared helper, `prepend`/`insert`/`set`'s) *item* check
that refuses, because it compares against an element type nothing filled in.

The diagnostic is also an internal one: no rule code, no source position beyond
the enclosing bind, and it names the impossible expectation ("must be Unknown")
rather than what the author did. Compare the rule-coded refusals the same area
produces. Either the element type should be inferred from context, or the
program should be refused with a rule code that names the restriction — one of
those, not neither.

The same reasoning, and the same shape of fix, as bug-549 (a `List OF <enum>`
type-checking and then failing on the payload classifier); this one is about
where the element type comes from rather than which element types are allowed.

## How it was found

`planning/tests.md` (the per-file coverage gate task), writing
`tests/rt-behavior/types/default-values-rt` — the fixture reaching codegen's
inline-TRAP default arms. The line

    MUT one AS List OF Integer = collections::append([], failing(1)) TRAP(e)

produced the error above, and it reproduces outside a `TRAP` just as well. The
fixture uses `[0]` as its source list instead, with a comment pointing here, so
it tests its own subject rather than this.
