# bug-468: assigning to a record field silently parses as an equality comparison, discarding the write with no diagnostic

Last updated: 2026-08-31
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (silent wrong behavior)

Status: FIXED
Regression Test: `tests/syntax/types/types-record-field-assign-invalid/` (new) + 7 parser unit tests in `src/ast/tests.rs`

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

## Reproduction confirmed (2026-08-31)

Reproduced on macos-aarch64 against `target/release/mfb` built at `5815262c4`,
in a fresh `mfb init` project — and confirmed to fail **for the documented
mechanism**, not merely with the documented symptom:

- Reproduction 1 (`p.x = 77`): `mfb build` → exit 0, `Wrote executable`; running
  it printed `after p.x = 77, p.x is 1`. The write was discarded, no diagnostic.
- Reproduction 2 (`p.y = "not an integer"`): exit 1 with
  `error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]` … `Operator `=` requires
  compatible **comparable** operands, got Integer and String.` — the parser had
  produced an equality expression, exactly as documented.
- Reproduction 3 (`s.state.field = v`): still worked, confirming the failure is
  specific to a member not literally spelled `state`.

Answering the doc's Open Question: the nested form `o.inner.x = 77` is swallowed
by the **same** path (built clean, `nested: 1`). The collection-element form the
question also asked about does not exist — `xs[0]` is constructor syntax, not
indexing (`mfb build` reports `SYMBOL_UNKNOWN_TYPE`: `Type `xs` is not a
built-in or top-level project type`), so there is nothing to swallow there.

## Partition

One root cause, one file, one fix: the fall-through at `src/ast/stmt.rs:322`.
All three failing shapes (plain field, nested field, inline-`IF` branch) are the
same mechanism with the same touch-set, so the work was **serial** on the
integration worktree — a fan-out would have produced overlapping diffs over one
guard.

## Phases

### Phase 1 — the parser guard + its tests

- [x] New rule `MFB_PARSE_RECORD_FIELD_ASSIGNMENT` (`1-102-0013`, the next free
      code in the `1-102` parse range — `0011`/`0012` are the TESTING rules, and
      the spec forbids adding to the existing code collisions) in
      `src/rules/table.rs`, with its row in
      `src/docs/spec/diagnostics/01_rule-codes.md` (a `cargo test` guard,
      `every_rule_is_documented_in_the_spec`, enforces that pairing).
- [x] Guard in `src/ast/stmt.rs::parse_simple_statement`, placed after the
      `resource.state[.field] =` block and before the plain `ident =` block. It
      walks the leading `ident (. ident)*` token chain and fires only when the
      chain is complete, has ≥2 segments, and is followed by `=`. Reports at the
      `=` token, naming the field and the `WITH <base> { <field> := <value> }`
      spelling, then returns `None` so the existing `synchronize()` recovery
      yields exactly one diagnostic per bad statement.
- [x] Detection is by **token lookahead in statement position**, per the doc: a
      comparison in expression position (`IF p.x = 77 THEN`) is parsed by
      `parse_if_statement`, never reaches `parse_simple_statement`, and is
      unaffected. A malformed chain (`a.b. = c`) is declined by the
      `path_complete` flag so it keeps its ordinary syntax diagnostic.
- [x] `tests/syntax/types/types-record-field-assign-invalid/` — covers the plain
      field write, the nested write, and the inline-`IF` branch. RED-verified:
      before the fix `mfb build -ast -ir` on this fixture exited **0** with
      `Wrote AST`/`Wrote IR`; after, exit 1 with three located diagnostics.
- [x] 7 parser unit tests in `src/ast/tests.rs`. RED-verified by neutralizing
      the guard condition and re-running: exactly the three `parse_rejects_*`
      tests failed and the four guard-neutral ones stayed green.
- [x] Spec §4 (`src/docs/spec/language/04_types.md`) now states that `WITH` is
      the only record-field update, that `value.field = expr` in statement
      position is rejected with this rule and why, and that the `RES` `STATE`
      forms and expression-position comparisons are unaffected.

Commit: `2c644d8c1`

## Blast radius, measured

- Statement-position `ident(.ident)+ =` across `tests/`, `examples/` and `src/`
  (including the injected builtin `package.mfb` sources), excluding `.state`:
  **0 hits** other than the new fixture
  (`grep -rn --include="*.mfb" -E '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)+[[:space:]]*=[^=]' tests/ examples/ src/ | grep -v '\.state'`).
- The inline-`IF` spelling (`THEN a.b =`): **0 hits**
  (`grep -rn --include="*.mfb" 'THEN [A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]* *=' tests/ examples/ src/`).
- Expression-position member comparisons (`IF err.code = …`) are widespread and
  all still parse — verified by the full acceptance sweep and by a direct probe
  that also re-confirmed `WITH`, `f.state.pos = 10` and `f.state = Cursor[…]`.

## Open Questions — resolved

- **Should `p.x = 77` be made to WORK?** No. Left as rejection, matching the
  current design (`04_types.md`, `15_resource-management.md:58`). Making records
  field-assignable is a language change and would need its own plan; this fix
  only stops the silent discard, which is wrong under either answer.
- **Nested / collection-element writes?** Nested (`p.inner.x = 1`) was swallowed
  by the same path and is covered by the fix and the fixture. Collection-element
  writes do not exist as syntax (see Reproduction confirmed, above).

## Verification

All four gates run in the integration worktree, in this order (the release
binary must be rebuilt *after* `cargo test` — whose `rt_*`/`codegen_*` tests
exec `target/release/mfb` — and *before* the two harnesses, which take it as an
argument):

| Gate | Result |
|---|---|
| `cargo test --no-fail-fast -- --skip artifact_gate_all` | `EXIT=0` — **4225 passed, 0 failed** across 79 targets, 0 `failures:` blocks |
| `scripts/test-accept.sh` | `exit=0` — **1316 tests ran**, all passed; the new fixture ran as `[1284] syntax/types/types-record-field-assign-invalid` |
| `scripts/artifact-gate.sh <exe> all` | `exit=0` — 1300 tests, 1457 builds, **1786 goldens checked, 0 diff(s)** |
| Original reproduction | now `exit 1` with the located `MFB_PARSE_RECORD_FIELD_ASSIGNMENT` diagnostic, on the doc's only environment (macos-aarch64) |

**No golden shifted.** That is the expected outcome and is itself evidence: a
parse-time *rejection* cannot change the code generated for any program that
still compiles, and the tree-wide census found no program that used the rejected
shape. The `1316 ran` count is +1 over the pre-change sweep — the new fixture,
not a silently skipped one.

Load-bearing forms re-confirmed by direct probe against the fixed compiler:
`WITH p { x := 77 }`, `IF p.x = 77 THEN`, `LET b AS Boolean = p.y = 2`,
`f.state.pos = 10` and `f.state = Cursor[pos := 20]` all build and print the
expected values.

`main` advanced during the fix (bug-464, TLS/thread codegen plus new fixtures);
it was merged into `worktree-B-468` — cleanly, no conflicts — and the full chain
above was re-run on the merged tree.

## STATUS: FIXED (`2c644d8c1`, `abf57a382`)

| Commit | What |
|---|---|
| `2c644d8c1` | The parser guard, the `MFB_PARSE_RECORD_FIELD_ASSIGNMENT` rule, the syntax fixture, 7 parser unit tests, and the §4 / rule-codes spec entries |
| `abf57a382` | Follow-up: moved the §4 provenance citation to the end of the paragraph so `mfb spec language types` stops rendering `ASSIGNMENT .` |

`p.x = 77` is now refused at compile time with a located, coded diagnostic that
names the field and gives the `WITH` spelling. It can no longer compile to a
discarded comparison.

**Deviations from the Suggested Fix, and why:**

- **Rule code `1-102-0013`, not a `TYPE_*` code.** The doc sketched
  `TYPE_RECORD_FIELD_ASSIGNMENT_REQUIRES_WITH`. The check lives in the parser
  (as the doc itself argued it must), so it belongs in the `1-102` parse family,
  not the `2-203` type family. `0011`/`0012` were already taken by the TESTING
  rules and `01_rule-codes.md` forbids adding to the existing code collisions,
  so `0013` is the next free code in that range.
- **Detection walks the whole `ident (. ident)*` chain, not just `ident . ident`.**
  The doc's Open Question asked whether nested writes were also swallowed. They
  were (`o.inner.x = 77` built clean and printed the old value), so the guard
  handles arbitrary depth rather than a single member.
- **The bare-comparison widening (`1 = 2` as a statement) was left alone**, as
  the doc marked optional. No write is lost there, and widening it would refuse
  programs that are merely pointless rather than wrong.

**Verification (merged tree, after taking main's bug-464):** `cargo test` exit 0
— 4226 passed, 0 failed across 80 targets; `test-accept.sh` exit 0 — 1320 tests
ran, all passed, with the new fixture at `[1288]`; `artifact-gate.sh all` exit 0
— 1304 tests, 1466 builds, 1799 goldens checked, **0 diffs**. Zero golden churn
is the expected result and is itself evidence: a parse-time rejection cannot
change codegen for any program that still compiles, and the tree-wide census
found no program using the rejected shape.
