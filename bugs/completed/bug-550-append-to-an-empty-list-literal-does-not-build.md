# bug-550: `collections::append([], x)` type-checks and then fails to build

Last updated: 2026-09-12 (FIXED)
Effort: unknown — small if the declared binding type simply needs to reach the
literal; medium if the empty literal's element type is inferred somewhere that
does not know it yet
Severity: MEDIUM
Class: Correctness (valid program does not build) / Diagnostics

Status: **FIXED** by `2203554bb`. Reproduced with the release compiler,
attributed, then closed — see "The fix" below.

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

## What was measured before touching anything

The report's attribution held up exactly as written.

* The repro builds the stated error with the release compiler, verbatim:
  `error: native collection list item must be Unknown, got Integer while
  lowering bind xs AS List OF Integer`.
* It is not a branch artifact. A detached worktree at `main` (`git worktree add
  --detach`, `cargo build --release`, 1m25s) produces the identical error on the
  fixture written for this bug.
* The report's own contrast is real: `LET xs AS List OF Integer = []` builds and
  runs, `collections::append([1], 2)` builds and runs, and
  `collections::append(e, 1)` over a *named* empty `List OF Integer` builds and
  runs. Only the inline `[]` at a generic parameter fails.
* `prepend` fails identically, as the report predicts from the shared helper. So
  does the un-annotated `LET xs = collections::append([], 1)`, which reports
  `... while lowering bind xs AS List OF Unknown` — the same defect one step
  further along.

The named root cause is also right, and the IR shows it directly
(`mfb build --ir`):

```json
"value": { "kind": "call", "type": "List OF Unknown", "target": "collections.append",
           "args": [{ "kind": "list", "type": "List OF Unknown", "values": [] },
                    { "kind": "const", "type": "Integer", "value": "1" }] }
```

The empty literal reaches codegen typed `List OF Unknown`, so
`typed_list_element_type` hands `collection_argument_as_list_slot` an `Unknown`
element type and the `Integer` item cannot match it.

## Where the element type was lost

`src/ir/lower.rs::call_argument_expected_type` is the single place a call
argument gets an expected type, and it had three answers, none of which applies
to a generic parameter:

* `registry::argument_types_typed` returns `None` for a member with more than
  one overload **or** any parameter mentioning a `Var` — `append` is both.
* `registry::agreed_argument_type` returns `None` the moment an overload's
  parameter at that index `contains_var`.
* the user-function tables do not know the name.

So the argument lowered with `expected = None`, and
`HirExpression::ListLiteral`'s fallback chain (`expected_element` →
`literal_expression_type(first)` → `expression_type(first)`) has no first element
to consult and lands on `Unknown`.

The information was never missing — overload selection already computes it.
`registry::unify` binds `T := Unknown` from the empty literal (its
"`Unknown` concrete" arm) and then REFINES it to `Integer` from the item (its
"`Some(ParameterType::Unknown) =>` insert" arm). The binding was simply thrown
away after substituting the *return* type; nobody substituted it back into the
*parameter* types.

## The fix

A fourth answer for `call_argument_expected_type`, for generic positions only:
`registry::resolved_parameter_type(qualified, index, arg_types, expected_return)`
re-runs the same overload selection over the call's actual argument types and
substitutes the resulting bindings into the parameter at `index`. `List OF T`
with `T := Integer` is `List OF Integer`, which is what the empty literal then
lowers as.

Two properties make it safe to consult:

* **It answers only generic positions.** A parameter that is not
  `contains_var` returns `None`, so `argument_types_typed` /
  `agreed_argument_type` keep owning every monomorphic position — including the
  ones `agreed_argument_type` deliberately declines (`json::stringify`'s
  `indent`, whose overloads disagree), which still go through the existing path.
* **It never guesses.** Both `append` overloads unify with `([], List OF
  String)` — the element form binding `T := List OF String`, the concatenating
  form `T := String` — and they disagree about parameter 0. Where the surviving
  overloads disagree the answer is `None` and the call lowers exactly as it did
  before (that spelling still reports the old error, rather than silently
  building a `List OF List OF String`).

The tie-breaker for that ambiguous case is the call's own expected type, which is
threaded in and SEEDS the unification: `append` returns `Arg(0)`, so an expected
return of `List OF String` is parameter 0's pattern and binds `T := String`
before any argument is looked at, leaving only the concatenating overload. This
is what makes `LET xs AS List OF String = collections::append([], ["a", "b"])`
build AND concatenate.

Touched: `src/codegen/registry/mod.rs` (`resolved_parameter_type`),
`src/codegen/builtins/mod.rs` (`resolved_argument_type`, the facade),
`src/ir/lower.rs` (`resolved_generic_argument_type` + the `expected_return`
thread through `call_argument_expected_type`). Landed as `2203554bb`.

### The diagnostic

The report's second defect — that the message is an internal one with no rule
code — is **not** fixed, and no longer applies to the program the report shows:
there is nothing left to refuse. It still applies to the ambiguous
`collections::append([], <list>)` written with no annotation, and to the
neighbouring `native collection …` refusals generally. That is the same
reporting problem bug-549 left open, and it belongs with those, not here.

### What is still `List OF Unknown`

`LET xs = collections::append([], 1)` now builds and produces a one-element
list, but the BINDING's static type stays `List OF Unknown`: an unannotated
`LET` freezes its type from `expression_type` before the value is lowered, and
`append`'s `Arg(0)` return echoes the raw argument type. That is not this bug —
it reproduces with no `append` in sight (`LET e = []` followed by
`collections::get(e, 0)` reports the same `argument type(s) (Unknown)`), and it
is the documented "bare list-literal synthesis" rule in
`mfb spec architecture type-inference`.

## Documentation

`src/docs/spec/architecture/22_type-inference.md` carried a section headed
"Call arguments — expected is NOT pushed into the argument". **The spec was
wrong, and was wrong before this change.** `call_argument_expected_type` has
threaded the parameter type into argument lowering since `2087edf3d`
(2026-06-13); the spec section was written on `dd9b96f83` (2026-06-26), two
weeks later. The registry's own `agreed_argument_type` doc comment records the
consequence in the other direction — a union-typed parameter that stops being
wrapped lowers a bare record where a tagged union is expected. The section is
rewritten to state the real lookup order, and the expected-type-position table
gains a `Call argument` row.

## Coverage

`tests/rt-behavior/collections/empty-list-literal-argument-rt` — in the
in-process corpus, so every backend lowers it on every `cargo test`. Every
assertion is a VALUE, because the risk the fix carries is overload selection and
a wrongly selected overload changes a length and an element, not an exit code:

```
append=1 1        prepend=1 a       insert=1 9
concat=2 b        nested=1 2        inferred=1
literal=3 3       both=4 4          named=0 1 7
loop=3 67         find=1            mid=2
```

The first six rows are the RED half (none of them built before). `concat=2 b` is
the ambiguity tie-break — a `1` there would mean the element overload won. The
rest are the POSITIVE pins the report's own contrast names: a non-empty literal,
the concatenating form over non-empty lists, an explicitly typed empty binding
used by name (`named=0 1 7` also pins that `append` did not mutate its
argument), the `append`-in-a-loop shape from the man page, and two other generic
positions (`find`, `mid`) that the new expected-type path now also answers.

`src/codegen/registry/mod.rs::resolved_parameter_type_infers_a_generic_position_from_the_call`
pins the resolver itself: the refinement, the ambiguity decline, the
expected-return seed, and the monomorphic decline.

## Gates

* **Artifact gate (full):** 1428 tests, 1594 build(s), 2003 golden(s) checked,
  **0 diff(s)**. The pre-fixture run of the same gate with the fix already
  applied reported 1427 / 1593 / 2001 / 0, so the delta is exactly +1 test,
  +1 build, +2 goldens — this bug's fixture and nothing else. **Zero `.run`
  goldens moved.**
* **Acceptance harness:** `acceptance tests passed (1451 test(s) ran)`, with
  **0** `unexpected actual` — which is the check that matters for a new
  fixture, because a missing golden reports only that way and prints no
  `mismatch:` line.
* **`cargo test --no-fail-fast`:** 161 `test result: ok` targets, 0 `FAILED`.

### A stale binary faked a three-golden blast radius

Worth recording because the wrong instrument said the right thing. A gate run
mid-way through this work reported exactly three DIFFs —
`http_codegen_cover_rt`, `resource_xfer_slots_cover_rt` and
`tls_codegen_cover_rt`, all `macos-aarch64`. Those are precisely the fixtures a
peer's `d77c3ced0` (bug-564, a `tls::close` fix) moved; all three `IMPORT tls`,
and every public member of an imported builtin package is emitted whether it is
called or not.

`git merge-base --is-ancestor d77c3ced0 HEAD` passed, which looked like proof
the baseline was current. It is not that proof: **ancestry in `HEAD` says
nothing about what a binary was compiled from.** `cargo build --release`
reported `Finished in 2m 02s` with the binary written at 07:44:17, so it started
~07:42:15 — and `d77c3ced0` landed at 07:43:44, 89 seconds INTO that build.
Cargo had already read the tls sources. The fix was to `touch` the two tls files,
rebuild, and confirm with a SCOPED gate (`artifact-gate.sh <mfb> tls` → 7
goldens, 0 diffs) before re-running the full sweep. Regenerating those goldens
would have written a stale compiler's hashes over a landed fix's record, matching
neither compiler.
