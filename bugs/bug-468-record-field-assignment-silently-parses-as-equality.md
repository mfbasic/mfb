# bug-468: assigning to a record field silently parses as an equality comparison, discarding the write with no diagnostic

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (silent wrong behavior)

Status: Open
Regression Test: `tests/syntax/records/field-assignment-rejected/` (new)

`p.x = 77`, where `p` is a `MUT` binding of a record type, **compiles clean and
does nothing.** No error, no warning, no effect. The developer's write is
silently discarded and the program runs on with the old value.

The cause is a parser gap, not a codegen fault. MFBASIC spells both assignment
and equality `=`. The statement parser recognises exactly three assignment
shapes (`src/ast/stmt.rs:228-320`):

| Shape | Parsed as |
|---|---|
| `ident = expr` | `Statement::Assign` (`stmt.rs:298-320`) |
| `ident.state = expr` | `Statement::StateAssign` (`stmt.rs:237-241`) |
| `ident.state.field = expr` | `Statement::StateAssign`, desugared to a one-field `WITH` (`stmt.rs:243-255`) |

There is **no `ident.field = expr` case**. When `field` is anything other than
the literal identifier `state`, none of the three guards match, control falls
through to the generic `self.parse_expression()` at `stmt.rs:322`, and `=` is
consumed as the **equality operator**. The result is a `Boolean` expression
statement whose value is computed and thrown away.

**The single correct behavior a fix produces:** `p.x = 77` on a plain record is
**refused at compile time** with a located, coded diagnostic that names the
field and points at `WITH` as the way to update a record. It must never again
compile to a discarded comparison. (Whether MFBASIC should instead *support*
in-place record field assignment is a language-design question — see Open
Questions — but silently accepting and discarding it is wrong under either
answer.)

This is the sharpest possible form of the failure mode: the language is
`WITH`-only for records by design, and the one construct a developer coming
from any other language will reach for first is accepted and ignored.

## Failing Reproduction

macos-aarch64, `target/release/mfb` at `e30c538de`, standard executable project
from `mfb init`.

### 1. The write is silently discarded

```basic
IMPORT io

TYPE Point
  x AS Integer
  y AS Integer
END TYPE

SUB main()
  MUT p AS Point = Point[x := 1, y := 2]
  p.x = 77
  io::print("after p.x = 77, p.x is " & toString(p.x))
END SUB
```

```
$ mfb build /tmp/p108-rec
Building p108_rec (executable) for macos-aarch64
Wrote executable to /tmp/p108-rec/build/p108_rec.out
$ /tmp/p108-rec/build/p108_rec.out
after p.x = 77, p.x is 1
```

Expected: a compile error. Actual: a clean build and `1`.

### 2. The proof that it is parsed as a COMPARISON

Give the right-hand side an incompatible type. If the statement were an
assignment, the diagnostic would be about assigning `String` to an `Integer`
field. It is not — it is about *comparing* them:

```basic
  p.y = "not an integer"
```

```
/tmp/p108-rec/src/main.mfb:17 error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]:
binary operator operands have incompatible types
    Operator `=` requires compatible comparable operands, got Integer and String.
```

`requires compatible **comparable** operands` — the parser produced an equality
expression. This is the whole bug in one diagnostic.

Note what this implies: the *only* reason the developer ever learns something is
wrong is if the types happen to be incomparable. With matching types — the
normal case, and the case in Reproduction 1 — there is no diagnostic at all.

### 3. It is specific to the `state` special case

The same shape works correctly through a `RES` handle, because `state` is one of
the three recognised forms:

`s.state.field = value` updates in place (`src/docs/spec/language/15_resource-management.md:58`),
and is desugared to `WITH s.state { field := value }` at `stmt.rs:281-289`.

So `s.state.x = 1` works and `p.x = 1` silently does not — the difference being
whether the member is literally spelled `state`.

## Expected vs Actual

| | |
|---|---|
| Expected | `p.x = 77` is rejected with a located, coded diagnostic naming the field and pointing at `WITH value { field := expr }` |
| Actual | compiles clean; evaluates `p.x = 77` as a `Boolean`, discards it; `p.x` keeps its old value |

## Impact

Silent data loss in the developer's own program logic. Every other class of
mistake in this language is caught — this one produces a working binary that
quietly does the wrong thing. A developer debugging "my field never changes"
has no compiler output to work from, and the natural next step (checking the
type of `p.x`) shows nothing wrong.

The blast radius is every record in every program, and records are the
language's primary data-carrying construct.

## Suggested Fix

In `src/ast/stmt.rs`, alongside the existing `state_assign` /
`state_field_assign` guards (`stmt.rs:228-256`), detect the general
`ident . ident =` shape where the member is **not** `state`, and report a new
rule from `src/rules/table.rs` — something in the shape of
`TYPE_RECORD_FIELD_ASSIGNMENT_REQUIRES_WITH` — carrying the field name and the
`WITH` spelling in its help text.

Detect it in the parser rather than the type checker: by the time the type
checker sees it, the statement is an ordinary comparison expression and is
indistinguishable from a deliberate (if pointless) one such as
`IF p.x = 77 THEN`. The guard must be careful to fire only in **statement**
position, never on a comparison inside an expression.

Two details worth pinning in the fix:

- The existing `state` forms must keep working unchanged — they are load-bearing
  (`bug-424`, `spec §15`).
- A discarded bare comparison in statement position is arguably wrong for
  *every* expression, not just member access (`1 = 2` as a statement). Widening
  the check is optional; the record-field case is the one that silently eats a
  developer's write.

## Open Questions

- **Should `p.x = 77` be made to WORK rather than rejected?** The language is
  deliberately `WITH`-only for records
  (`src/docs/spec/language/04_types.md:124`), and `s.state.field = value` is
  documented as "the only member-target assignment in the language"
  (`15_resource-management.md:58`) — so rejection matches the current design.
  Making it work would be a language change needing its own plan. Either way,
  the silent discard is a bug now.
- Does the same silent-discard path swallow writes through a collection element
  or a nested field (`p.inner.x = 1`)? Not probed. Worth covering in the
  regression test.

## References

- `src/ast/stmt.rs:228-256` — the three recognised assignment shapes.
- `src/ast/stmt.rs:322` — the fall-through to `parse_expression`, where `=`
  becomes equality.
- `src/docs/spec/language/04_types.md:124` — `WITH value { field := expr }` is
  the record update form.
- `src/docs/spec/language/15_resource-management.md:58` — the `state` carve-out.
- `src/rules/table.rs` — where the new rule belongs.
- Found during: plan-108-A Phase 2b, writing the `mfb man variable` topic. The
  page needed to demonstrate "records are `WITH`-only"; the probe that was meant
  to show the *correct* way instead showed that the incorrect way compiles
  silently.
