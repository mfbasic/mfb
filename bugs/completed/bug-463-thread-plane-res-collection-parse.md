# bug-463: a `RES`-marked collection on a thread's message plane does not parse — the whole type becomes an opaque `Named`

Last updated: 2026-08-30
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: FIXED (86da1dacc)
Regression Tests: `src/types.rs::res_collection_on_thread_plane_parses_structurally`,
`::type_prefix_len_measures_every_parse_constructor`,
`::thread_message_plane_measures_every_parse_constructor` (all new)

`ParameterType::parse("Thread OF List OF RES fs.File TO Integer")` returns
`Named("Thread OF List OF RES fs.File TO Integer")` — the entire spelling as one
opaque nominal — instead of a `ThreadHandle`. The type then reaches the resolver
as a single name containing a `.`, which splits it at the first dot and reports a
package that never existed:

```
error[2-201-0014 SYMBOL_UNKNOWN_IMPORT]: package-qualified symbol uses an unknown import
               Package `Thread OF List OF RES fs` is used but not imported in this file.
```

**The single correct behavior a fix produces:** the type parses structurally into
`ThreadHandle { msg: ListOf(Res(fs.File)), .. }`, and the program is then rejected
by the *thread-sendability* check — `ParameterType::Res(_) => false`
(`src/ir/verify/resources.rs:439`, "Sharing a resource collection across threads
is out of scope (§15.6)") — with `TYPE_THREAD_NOT_SENDABLE`. That is exactly what
already happens when the same collection is on the **output** plane (see the
contrast table below), so the fix makes the message plane agree with the output
plane.

What makes this more than a bad message: the type survives as an opaque `Named`
through the whole front end. The diagnostic that *should* fire never does. Today
that is masked because the resolver rejects the junk name for an unrelated reason
— but the program is being refused by an accident of spelling, not by the rule
that governs it, and the accident depends on the resource being
package-qualified.

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 — "Sharing a resource
  collection across threads remains out of scope."
- `src/docs/spec/language/16_threads.md` — the data plane / `RES` plane split.
- `planning/plan-114-A-thread-plane-resource-error.md` — Phase 1 depends on this
  fix; that letter adds `2-203-0137 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, and
  without this bug fixed the new rule is unreachable from MFBASIC source for this
  spelling. Found while writing that plan.
- Memory: `byte-identity-cannot-see-backward-seams` — the reason the existing
  round-trip corpus does not catch this (see "Why the tests missed it").

## Failing Reproduction

Unit level — decisive, and the shape the regression test takes:

```rust
// added to src/types.rs's #[cfg(test)] mod tests, then `cargo test --bin mfb`
let parsed = ParameterType::parse("Thread OF List OF RES fs.File TO Integer");
assert!(matches!(parsed, ParameterType::ThreadHandle { .. }));
```

- Observed (run 2026-08-30, `cargo test --bin mfb tmp_bug463`):
  ``` 
  `Thread OF List OF RES fs.File TO Integer` did not parse into ThreadHandle;
  got Named("Thread OF List OF RES fs.File TO Integer")
  ```
- Expected: a `ThreadHandle` whose `msg` is `ListOf(Res(Named("fs.File")))`.

Source level — the user-visible symptom:

```basic
IMPORT fs
IMPORT io
FUNC f(x AS Thread OF List OF RES fs::File TO Integer) AS Integer
  RETURN 0
END FUNC
FUNC main AS Integer
  io::print("ok")
  RETURN 0
END FUNC
```

- Observed: `2-201-0014 SYMBOL_UNKNOWN_IMPORT` — ``Package `Thread OF List OF RES fs`
  is used but not imported in this file.``
- Expected: `2-203-0063 TYPE_THREAD_NOT_SENDABLE` (and, once plan-114-A lands,
  `2-203-0137 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`).

### Contrast matrix

Measured with `target/release/mfb build` on a scratch project, one `FUNC f(x AS <T>)`
parameter per row (host `macos-aarch64`; the bug is in the shared type grammar and
is not platform-dependent):

| Type spelling | Result |
| --- | --- |
| `List OF RES fs::File` | works ✓ |
| `Map OF String TO RES fs::File` | works ✓ |
| `Map OF String TO List OF RES fs::File` | works ✓ |
| `List OF List OF RES fs::File` | works ✓ |
| `Thread OF Integer TO Integer` | works ✓ |
| `Thread OF List OF Integer TO Integer` | works ✓ |
| `Thread OF Integer RES fs::File TO Integer` (resource plane) | works ✓ |
| `Thread OF Integer TO List OF RES fs::File` (**output** plane) | works ✓ — correctly reaches `TYPE_THREAD_NOT_SENDABLE` |
| `Thread OF List OF RES fs::File TO Integer` | fails ✗ |
| `Thread OF Map OF String TO RES fs::File TO Integer` | fails ✗ |
| `ThreadWorker OF List OF RES fs::File TO Integer` | fails ✗ |
| `Thread OF List OF RES fs::File STATE Cursor TO Integer` | fails ✗ |
| `Thread OF List OF RES fs::File RES fs::File TO Integer` | fails ✗ |
| `Map OF Thread OF List OF RES fs::File TO Integer TO Integer` | fails ✗ |

The two informative rows are the last-position ones: the **output** plane works and
the **message** plane does not. The output plane is not *measured* — it is
`tail.strip_prefix(" TO ")?.trim()`, i.e. "everything left". The message plane must
be measured to find where it ends. That is the whole bug.

### The matrix was incomplete — four arms were missing, not one

Re-measured while reproducing (2026-08-31, same method, `/tmp/b463-matrix.sh`). The
`RES` marker is one of **four** constructors `type_prefix_len` did not recognise, and
the other three fail the same way for the same reason. Two of them are worse than the
reported bug: they reject programs that are perfectly **valid**.

| Type spelling | Before | After |
| --- | --- | --- |
| `Thread OF Set OF Integer TO Integer` | ✗ `SYMBOL_UNKNOWN_TYPE` | **accepted** |
| `Thread OF List OF Set OF Integer TO Integer` | ✗ `SYMBOL_UNKNOWN_TYPE` | **accepted** |
| `Thread OF Map OF Set OF Integer TO Integer TO Integer` | ✗ `SYMBOL_UNKNOWN_TYPE` | **accepted** |
| `Thread OF Pair OF Integer, String TO Integer` (user generic) | ✗ `SYMBOL_UNKNOWN_TYPE` | **accepted** |
| `Thread OF FUNC(Integer) AS String TO Integer` | ✗ `SYMBOL_UNKNOWN_TYPE` | `TYPE_THREAD_NOT_SENDABLE` |
| `Map OF Thread OF List OF RES fs::File TO Integer TO Integer` | ✗ `SYMBOL_UNKNOWN_IMPORT` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` |
| `Thread OF Map OF RES fs::File TO Integer TO Integer` (`Map` **key**) | `MFB_PARSE_INVALID_IDENTIFIER` | unchanged — the AST parser is the gate |
| `Thread OF MapEntry OF String TO Integer TO Integer` | ✓ | unchanged |
| `Thread OF Result OF Integer TO Integer` | `TYPE_RESULT_NOT_USER_VISIBLE` | unchanged |

`Set` was absent from the descent entirely; `FUNC(...)`/`ISOLATED FUNC(...)` died on
the base-name scan, which stops at the `(`; a user generic fell through to its bare
head. Each one makes the *enclosing* type fail to split, so the symptom is identical
in every case — the whole spelling becomes one opaque `Named`.

The `Map` **key** row settles the Open Decision below: the key position was broken in
the string grammar too, but it is not reachable from MFBASIC source, because
`ast/expr.rs`'s map arm offers the `RES` marker only in the value position. The
string grammar accepts the spelling (`"Map OF RES fs.File TO Integer"` is in the
`res_collection_round_trips_byte_exact` corpus), so the measurement must accept it —
**measurement is not acceptance**, and the AST parser remains the gate.

## Root Cause

`type_prefix_len` (`src/types.rs:1058`) measures how many bytes a single type
occupies at the start of a string. Its `List`/`Result` arm is:

```rust
if matches!(base, "List" | "Result") {
    return type_prefix_len(after_of).map(|len| base_end + 4 + len);   // :1095-1097
}
```

For `List OF RES fs.File TO Integer`, `after_of` is `"RES fs.File TO Integer"`.
The recursive call scans for the first character that is not alphanumeric/`_`/`:`/`.`
(`:1077-1086`), which is the space after `RES`, so it measures `base = "RES"`,
finds no `" OF "` after it, and returns `3`. The `List` arm therefore reports that
the type ends after `"List OF RES"` — 11 bytes.

`split_thread_types` (`:882`) then takes `message = "List OF RES"` and
`tail = " fs.File TO Integer"`, which matches neither `" RES "` nor `" TO "`, so it
returns `None`. `thread_parts_full` returns `None`, `ParameterType::parse` falls
through to its nominal tail, and the whole spelling becomes one `Named`.

The `Map`/`MapEntry` arm (`:1099-1105`) has the identical defect in its **value**
position, which is why `Thread OF Map OF String TO RES fs::File TO Integer` fails
the same way.

**The helper that gets this right already exists.** `resource_element_len`
(`src/types.rs:1042`) measures exactly a `RES` element plus its optional
` STATE T` clause, and is already used for the thread **resource** plane
(`split_thread_types:875`, `thread_body_len:1118`) — which is precisely why
`Thread OF Integer RES fs::File TO Integer` is in the works column. `type_prefix_len`
simply never calls it.

**Stated generally, which is what the fix acts on:** `type_prefix_len` must recognise
every constructor `ParameterType::parse` can decompose, and it recognised six of ten.
A missing arm does not measure *short* — it makes the enclosing type fail to split at
all, and the whole spelling survives as one nominal through the entire front end with
nothing downstream reporting it. Reading the two functions side by side is the check
that finds all four gaps at once; chasing the `RES` symptom alone finds one.

Why the resolver's report is the shape it is: with the type reduced to a `Named`,
`resolve_type` (`src/resolver/resolution.rs:1330-1370`) finds no structural arm and
falls to `resolve_leaf` (`:1443`), whose tail does `if name.contains('.')` →
`resolve_package_qualified_name`, which takes `name.split('.').next()` (`:1443`) as
the package root — `"Thread OF List OF RES fs"`.

### Why the tests missed it

`src/types.rs::thread_handle_round_trips_byte_exact` (`:1240`) and
`res_collection_round_trips_byte_exact` (`:1898`) both exist and both pass. Neither
covers the **crossing** case (a `RES` collection *inside* a thread plane) — and,
more importantly, **a round-trip assertion cannot detect this class of bug at all.**
Verified: adding `"Thread OF List OF RES fs.File TO Integer"` to
`thread_handle_round_trips_byte_exact` **passes**, because `Named(s).name() == s`
echoes the junk back verbatim. Only asserting the parsed *variant* fails. The
regression test must therefore assert `matches!(.., ThreadHandle { .. })`, not
`parse(s).name() == s`.

The `ir/verify` thread tests (`src/ir/verify/tests.rs:7898-8054`) missed it for a
different reason: they build `ParameterType`s directly from the dotted
`"List OF RES fs.File"` spelling and never go through the parser.

## Goal

- `ParameterType::parse` produces a `ThreadHandle` for every row in the "fails ✗"
  column above, with the message plane structurally intact
  (`ListOf(Res(..))` / `MapOf(.., Res(..))`, `STATE` clause preserved).
- The source repro reports `TYPE_THREAD_NOT_SENDABLE`, not `SYMBOL_UNKNOWN_IMPORT`.
- Every row in the "works ✓" column behaves exactly as it does today.
- **(widened)** `type_prefix_len` recognises every constructor `parse` decomposes,
  so a `Set`, a `FUNC(...)` and a user generic measure on the message plane too —
  and the valid programs among them compile.

### Non-goals (must NOT change)

- ~~**The set of accepted programs.**~~ **DEVIATION — this non-goal could not be
  kept, because the diagnosis behind it was incomplete.** It holds for everything
  the report measured: a resource on the thread data plane stays rejected, and only
  the diagnostic changes, from an accidental resolver error to the rule that governs
  it. But the same measurement gap was *also* refusing valid programs —
  `Thread OF Set OF Integer TO Integer` is thread-sendable
  (`ir::verify::resources.rs:431`, `SetOf(element) => is_thread_sendable(element)`)
  and was rejected outright with `SYMBOL_UNKNOWN_TYPE`. Four such spellings now
  compile (see the widened matrix). Refusing a correct program is the more severe
  half of this bug, not a separate one, so it is fixed here rather than deferred.
- **`parse` ↔ `name` byte-exactness.** `ParameterType::parse(s).name() == s` is
  load-bearing at every parse-in boundary. It currently holds for these spellings
  *by accident* (a `Named` echoing itself); after the fix it must hold *structurally*.
  Both corpora must still pass.
- **No second type grammar.** `ParameterType::parse` is the only type grammar;
  fix `type_prefix_len` in place and reuse `resource_element_len`. Do not add a
  `RES` special case to `resolve_package_qualified_name` — that would mask the
  symptom and leave the type an opaque `Named` through the rest of the front end.
- **Tempting wrong fix, forbidden:** "fixing" this by adding the spelling to
  `thread_handle_round_trips_byte_exact` and declaring it covered. That test
  passes today with the bug present (proven above); it is not a guard for this.
- No `.mfp`/`.ir`/`.ncode` format or ABI change.

## Blast Radius

Found by `grep -rn "type_prefix_len" src/ --include='*.rs'` → 12 hits, of which the
call sites are:

- `src/types.rs:1096` (`List`/`Result` element) — **fixed by this bug**. The
  primary defect.
- `src/types.rs:1100`/`:1103` (`Map`/`MapEntry` key and value) — **fixed by this
  bug**. Same defect, proven by the `Thread OF Map OF String TO RES fs::File TO Integer`
  row.
- `src/types.rs:882` (`split_thread_types`, message plane) — the caller that
  *exhibits* the bug; fixed transitively, no edit needed.
- `src/types.rs:1043`/`:1049` (`resource_element_len`) — **unaffected**: it already
  strips `RES ` before calling, and handles ` STATE `. It is the correct reference
  implementation this fix borrows.
- `src/types.rs:1121`/`:1126`/`:1132`/`:1138` (`thread_body_len`, nested thread
  types) — fixed transitively; `thread_body_len` already routes its `RES` planes
  through `resource_element_len`.

Sibling grammar copies flagged as a lockstep hazard by
`planning/Compiler Pipeline.md:25` and the doc comment at `src/types.rs:~925`
(`split_map_body` had copies in `monomorph::helpers` and the former source
checker's `types::split_map_body`): **audit task in Phase 1** — confirm whether any
surviving copy has the same missing-`RES` arm, and record the verdict here. If one
does and is live, it is in scope; if it is dead, say so.

**Audit verdict (2026-08-31): no live sibling copy survives. `src/types.rs` holds
the only type-grammar cascade.** Measured with

```
grep -rn 'strip_prefix("List OF \|strip_prefix("Map OF \|strip_prefix("Set OF \|strip_prefix("RES \|strip_prefix("Result OF \|strip_prefix("MapEntry OF \|strip_prefix("Thread' src/ --include='*.rs'
grep -rn "type_prefix_len" src/ --include='*.rs'
```

Per site:

- `src/types.rs` — the live grammar (`parse`, `type_prefix_len`, `split_thread_types`,
  `thread_body_len`, `resource_element_len`, `split_top_level_to`). **In scope; fixed.**
- `monomorph/helpers.rs`, `monomorph/lower.rs`, `resolver/resolution.rs`,
  `ir/lower.rs`, `ir/verify/{mod,compat,resources}.rs` — **dead**. Every remaining hit
  is a doc comment recording what plan-106-A/B/D *replaced* (e.g.
  `src/ir/lower.rs:2268` "the `strip_prefix("List OF ")` / `strip_prefix("Set OF ")` /
  `parse_map_type`…"). No executable copy remains; `type_prefix_len` has no caller
  outside `src/types.rs`.
- `src/ast/expr.rs:595-763` — a **token**-level parser over the lexer, not a
  length-measuring string cascade. It cannot have this defect (there is nothing to
  mis-measure), and it is the real acceptance gate: `Set OF RES …` and a `Map` *key*
  `RES` marker are rejected there. **Not in scope, correctly.**
- `src/resolver/resolution.rs:1489-1495` — an **orphaned doc comment**: the
  `split_map_body` it documents was deleted, and the `///` block silently reattached
  to the `#[cfg(test)] mod tests` that follows it. Harmless to behavior, actively
  misleading to a reader. **Fixed in passing** (comment deleted).

`src/resolver/resolution.rs:1443` (`resolve_package_qualified_name`'s
`name.split('.').next()`) — **latent, out of scope.** It is only ever reached with a
junk composite `Named` because some *other* grammar arm failed; hardening it would
turn a loud wrong-package error into a quieter one and hide the next instance of
this class. Left alone deliberately.

## Fix Design

In `type_prefix_len`, the `List`/`Result` and `Map`/`MapEntry` element positions
consume an optional `RES ` marker by delegating to the existing
`resource_element_len`, which already handles the trailing ` STATE T` clause:

```rust
/// Length of one element/value type, honouring the `RES ` ownership marker and
/// its optional ` STATE T` clause (§15.6). `resource_element_len` is the same
/// measurement the thread resource plane already uses.
fn element_prefix_len(input: &str) -> Option<usize> {
    match input.strip_prefix("RES ") {
        Some(after_res) => resource_element_len(after_res).map(|len| 4 + len),
        None => type_prefix_len(input),
    }
}
```

and the two arms call `element_prefix_len` for their element / value (and, for
`Map`, its key — `Map OF RES fs.File TO Integer` is in the existing corpus at
`:1908`, so the key position takes the marker too).

**DEVIATION — what was actually built.** A per-arm `element_prefix_len` fixes the
`RES` marker in the three positions the report enumerated and leaves every *other*
measured position — a `Set` element, a generic argument, a `FUNC` return, the next
one added — still broken, which is how three of the four gaps went unnoticed in the
first place. `parse` already solves this by handling `RES ` **once, at the top**,
before any container arm. `type_prefix_len` now mirrors that, arm for arm and in the
same order:

```rust
if let Some(after_res) = input.strip_prefix("RES ") {
    return resource_element_len(after_res).map(|len| "RES ".len() + len);
}
```

It still delegates to `resource_element_len` — the report is right that this is the
reference implementation, and it is what carries the ` STATE T` clause — but from one
site that covers every position rather than three sites that cover three. The other
three gaps are then one-liners in the same shape: `Set` joins `List`/`Result`; a
`FUNC(`/`ISOLATED FUNC(` arm sits beside the `RES` one (new `func_body_len`); and the
`Some(base_end)` fallthrough becomes the user-generic argument loop. Reading
`type_prefix_len` against `parse` is what makes a future gap visible, which is why the
function's doc comment now states that requirement.

`func_body_len` mirrors `codegen::builtins::split_func_params_and_return` on the
depth-0 paren scan but deliberately **measures** the return type where that helper
takes "everything after `) AS `". That is the same measured-vs-trailing asymmetry as
the thread output plane: `parse` receives a string that is nothing but the type, while
a measurement has to stop where the type stops — otherwise
`Thread OF FUNC(Integer) AS String TO Integer` swallows the thread's own ` TO `.

Correctness risk is low and concentrated in the arithmetic: the `+ 4` for `"RES "`
and the `base_end + 4` for `" OF "` are easy to get off by one, and an off-by-one
here silently mis-measures a *different* type rather than failing loudly. The
round-trip corpora plus the new structural assertions are what catch it.

Rejected alternatives:

- *Make the `base_end` scan in `type_prefix_len` treat `RES` as part of the base
  token.* Rejected: `RES` is an ownership marker, not part of a nominal; folding it
  into the name scan would let `RES` through in positions the grammar forbids
  (`Set OF RES …` is a deliberate parse error, `src/ast/expr.rs:678-687`).
- *Special-case the thread message plane in `split_thread_types`.* Rejected: it
  would fix `Thread OF List OF RES …` and leave `Map OF Thread OF List OF RES … TO …`
  and every future measured position broken. The defect is in the measurement, so
  the fix belongs there.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `res_collection_on_thread_plane_parses_structurally` to `src/types.rs`'s
      test module, asserting `matches!(ParameterType::parse(s), ParameterType::ThreadHandle { .. })`
      for all six "fails ✗" spellings **and** for the `Thread OF List OF Integer TO Integer`
      / `Thread OF Integer RES fs.File TO Integer` contrast cases. Confirm it fails
      on exactly the six.
- [x] Extend `thread_handle_round_trips_byte_exact` (`:1240`) and
      `res_collection_round_trips_byte_exact` (`:1898`) with the crossing spellings,
      so byte-exactness is pinned once it holds structurally. Note in a comment that
      round-trip alone does not detect this class.
- [x] Audit the sibling `split_map_body`/`type_prefix_len` copies flagged in
      `planning/Compiler Pipeline.md:25` and `src/types.rs:~925`; write each
      verdict into the Blast Radius section above.

Acceptance: **met, and widened.** The structural test failed on exactly the six
spellings and passed on the contrast cases. Reproducing also turned up three further
missing arms, so two more tests were added at the same time:
`type_prefix_len_measures_every_parse_constructor` (a complete type measures to its
own length — the property that generalises past this one bug) and
`thread_message_plane_measures_every_parse_constructor`. Both were RED on the real
mechanism.

Both round-trip corpora were extended with the crossing spellings and **passed with
the bug still present**, exactly as the report predicted — that is the evidence for
asserting the variant instead. The audit found no live sibling copy; verdicts are in
the Blast Radius above.
Commit: `3087285e1`

### Phase 2 — the fix

- [x] Consume the `RES ` marker in `type_prefix_len` — **once at the top**, mirroring
      `parse`'s own arm order, rather than per-arm via `element_prefix_len` (see the
      DEVIATION in Fix Design). Still delegates to `resource_element_len`.
- [x] Close the other three arms the reproduction found: `Set` joins `List`/`Result`;
      a `FUNC(`/`ISOLATED FUNC(` arm plus the new `func_body_len`; the fallthrough
      becomes the user-generic argument loop.
- [x] No live sibling copy to fix (Phase 1 audit). Fixed in passing: an orphaned
      `split_map_body` doc comment in `resolver/resolution.rs` that had silently
      reattached to the following `#[cfg(test)] mod tests`.
- [x] Add `tests/syntax/threads/thread-res-collection-plane-invalid/` — three
      spellings (`List` / `Map` message plane, and `ThreadWorker`), `golden/build.log`
      showing `TYPE_THREAD_NOT_SENDABLE` for each. (plan-114-A Phase 3 later re-goldens
      this to `2-203-0137`; that is that plan's task, not this one's.)
- [x] Add `tests/syntax/threads/thread-message-plane-constructors-valid/` — the
      acceptance side, which the report did not anticipate needing: the four
      previously-refused **valid** programs, `[exit 0]`.

Acceptance: **met.** All three structural tests pass; the source repro reports
`2-203-0063 TYPE_THREAD_NOT_SENDABLE`; every "works ✓" row still works, verified by
re-running the full matrix.
Commit: `66c028ba6`

### Phase 3 — regenerate expected outputs + full validation

- [x] `cargo test --no-fail-fast` → cargo exit **0**; `test result: ok. 3600 passed;
      0 failed` on the unit target and `0 failed` on every integration target.
- [x] `cargo check --all-targets` → exit 0, **0 warnings**.
- [x] `scripts/test-accept.sh target/release/mfb /tmp/b463-accept` — full:
      **`acceptance tests passed (1311 test(s) ran)`**.
- [x] `scripts/artifact-gate.sh target/release/mfb all` →
      **`1295 tests, 1452 build(s), 1786 golden(s) checked, 0 diff(s)`**, as predicted:
      the four newly-accepted spellings had no existing fixture, so no codegen moved.
- [x] Re-ran the full contrast matrix; every row matches its expected column, and the
      widened rows are recorded above.
- [x] `rustup run 1.96.0 cargo fmt --all` + the second pass in `repository/`. Churn was
      confined to the Phase 1 test tables.
- [x] `cargo clippy --all-targets`: the changed region is clean. Three deny-level
      lints fire, all **pre-existing** — `monomorph/lower.rs:2192`,
      `os/link_encode.rs:308`, `resolver/resolution.rs` `peel_res`; each line is
      byte-identical on `main` (`git show main:<file> | sed -n '<line>p'`) and in a
      file this change does not touch.

Acceptance: **met.** Full suite green; `diffs=0`; every matrix row as expected.
Commit: `86da1dacc` (fmt)

## Validation Plan

- Regression test: `src/types.rs::res_collection_on_thread_plane_parses_structurally`
  — asserts the parsed **variant**, not the round-trip, because the round-trip
  passes with the bug present (proven in "Why the tests missed it").
- Runtime proof: none applicable — this is a front-end rejection path; no program
  reaches codegen either before or after. The proof is the fixture `build.log`
  changing from `SYMBOL_UNKNOWN_IMPORT` to `TYPE_THREAD_NOT_SENDABLE`.
- Doc sync: none expected. §15.6 already states the rule ("Sharing a resource
  collection across threads remains out of scope") and `2-203-0063` already exists;
  this bug makes the code enforce what the spec says, and adds no new surface.
- Full suite: `cargo test --no-fail-fast`; `cargo check --all-targets`;
  `scripts/test-accept.sh`; `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions — both resolved

- **Does the `Map` *key* position need the marker too?** **Yes, and it was broken.**
  The Phase 1 row settled it: `type_prefix_len("Thread OF Map OF RES fs.File TO
  Integer TO Integer")` returned `None`. Handling `RES ` once at the top of
  `type_prefix_len` covers the key, the value, and every other position at the same
  time, so the question stops being per-position. Source acceptance is unchanged —
  `ast/expr.rs`'s map arm offers the marker only in the value position, so the key
  spelling stays `MFB_PARSE_INVALID_IDENTIFIER` from MFBASIC. That asymmetry is
  correct: **the measurement's job is to agree with `parse`, and the AST parser's job
  is to decide what a program may say.**
- **Should `Result OF RES …` keep sharing the `List` arm?** **Yes — and `Set` now
  shares it too.** All three have a single element child in `parse`, so one arm is the
  honest model. Same reasoning as above for the acceptance side: `ast/expr.rs:678-687`
  rejects `Set OF RES …` outright and offers the marker only on `List`, while the
  string grammar accepts `Set OF RES fs.File` (it is in the
  `res_collection_round_trips_byte_exact` corpus). Splitting the measurement arms to
  chase the parser's rules would create exactly the two-grammar drift this codebase has
  already paid for once.

## Summary

The engineering risk is entirely in the measurement arithmetic — an off-by-one in
`element_prefix_len` mis-measures a *different* type quietly rather than failing
loudly, which is why the fix rides two existing round-trip corpora plus new
structural assertions. The fix itself is small and reuses `resource_element_len`,
the helper that already gets this right for the thread resource plane. The most
transferable finding is a testing one: the existing round-trip corpus **passes**
with this bug present, because `Named(s).name() == s` echoes the junk back — a
byte-exact round trip cannot see a type that failed to decompose. Left deliberately
untouched: `resolve_package_qualified_name`'s naive dot-split, which is only ever
reached because some grammar arm already failed and whose loudness is useful.

### What the fix actually found (2026-08-31)

Everything above held. Two things were bigger than written:

1. **The `RES` marker was one of four missing arms, not the bug.** `Set`,
   `FUNC(...)`/`ISOLATED FUNC(...)` and user generics were missing from
   `type_prefix_len` too, each failing in exactly the same way. The report reached the
   right mechanism by chasing one symptom; reading `type_prefix_len` **against `parse`,
   arm for arm** is what finds the class, and that check is now the doc comment on the
   function and the property in
   `type_prefix_len_measures_every_parse_constructor` (a complete type measures to its
   own length).

2. **The non-goal "the set of accepted programs" could not be kept.** The same gap was
   refusing *valid* programs: `Thread OF Set OF Integer TO Integer` is thread-sendable
   and was rejected outright with `SYMBOL_UNKNOWN_TYPE`. That is the more severe half
   of this bug — a wrong diagnostic on an invalid program is a usability defect, a
   refusal of a correct program is a correctness defect — and it was invisible until
   the matrix was widened past the `RES` rows the report measured.

The generalisable lesson, beyond the round-trip one: **in a stringly-typed grammar, a
recogniser and its measurer are two lists that must be compared directly.** A gap in
the measurer is silent by construction — it does not measure short, it makes the
enclosing type fail to split, and the spelling survives as an opaque nominal with
nothing downstream reporting it. Enumerating the pairs is cheap; discovering the gap
from a symptom is what cost this bug four arms' worth of latency.

## STATUS: FIXED (86da1dacc)

Landed on `main` 2026-08-31, three commits:

| Commit | What |
| --- | --- |
| `3087285e1` | Phase 1 — RED structural tests + sibling-grammar audit |
| `66c028ba6` | Phase 2 — the fix in `type_prefix_len` + two fixtures |
| `86da1dacc` | rustfmt |

**Deviations from this document, both recorded in place above:**

1. **Scope.** The `RES` marker was one of **four** missing arms in `type_prefix_len`
   (`Set`, `FUNC(...)`/`ISOLATED FUNC(...)` and user generics were also absent), all
   with the identical failure mode. Fixed together; the contrast matrix is widened
   with before/after for each.
2. **The "set of accepted programs" non-goal.** Could not be kept — the same gap was
   refusing four *valid*, thread-sendable spellings (`Thread OF Set OF Integer TO
   Integer` and friends) with `SYMBOL_UNKNOWN_TYPE`. Those now compile, which is a
   correctness fix, not a scope creep.
3. **Fix shape.** `RES ` is consumed **once at the top of `type_prefix_len`**,
   mirroring `parse`'s own arm order, rather than by a per-arm `element_prefix_len` as
   designed here. Same delegation to `resource_element_len`; one site instead of
   three, and it covers positions the per-arm design would have left broken.

**Fixed in passing:** `resolver/resolution.rs` carried an orphaned `split_map_body`
doc comment — the function was deleted, and the `///` block had silently reattached
itself to the following `#[cfg(test)] mod tests`.

**Left untouched, as designed:** `resolve_package_qualified_name`'s naive dot-split
(`resolution.rs:1443`). It is only ever reached because a grammar arm already failed,
and its loudness is what surfaced this bug.

**Verification** (host `macos-aarch64`; the fix is in the shared type grammar and is
not platform-dependent):

| Gate | Result |
| --- | --- |
| `cargo test --no-fail-fast` | exit 0 — `3600 passed; 0 failed`, and `0 failed` on every integration target |
| `cargo check --all-targets` | exit 0, 0 warnings |
| `scripts/test-accept.sh target/release/mfb /tmp/b463-accept` | `acceptance tests passed (1311 test(s) ran)` |
| `scripts/artifact-gate.sh target/release/mfb all` | `1295 tests, 1452 build(s), 1786 golden(s) checked, 0 diff(s)` |
| contrast matrix, re-run in full | every row as expected |
| `cargo clippy --all-targets` | changed region clean; 3 deny-level lints pre-existing on `main` in untouched files |

**Downstream:** `planning/plan-114-A-thread-plane-resource-error.md` Phase 1 was
blocked on this and is now unblocked; its Phase 3 re-goldens
`tests/syntax/threads/thread-res-collection-plane-invalid/golden/build.log` from
`2-203-0063` to the new `2-203-0137 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`.
