# bug-463: a `RES`-marked collection on a thread's message plane does not parse — the whole type becomes an opaque `Named`

Last updated: 2026-08-30
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `src/types.rs::res_collection_on_thread_plane_parses_structurally` (new)

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

### Non-goals (must NOT change)

- **The set of accepted programs.** A resource on the thread data plane stays
  rejected; only the diagnostic that rejects it changes, from an accidental
  resolver error to the rule that actually governs it.
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

- [ ] Add `res_collection_on_thread_plane_parses_structurally` to `src/types.rs`'s
      test module, asserting `matches!(ParameterType::parse(s), ParameterType::ThreadHandle { .. })`
      for all six "fails ✗" spellings **and** for the `Thread OF List OF Integer TO Integer`
      / `Thread OF Integer RES fs.File TO Integer` contrast cases. Confirm it fails
      on exactly the six.
- [ ] Extend `thread_handle_round_trips_byte_exact` (`:1240`) and
      `res_collection_round_trips_byte_exact` (`:1898`) with the crossing spellings,
      so byte-exactness is pinned once it holds structurally. Note in a comment that
      round-trip alone does not detect this class.
- [ ] Audit the sibling `split_map_body`/`type_prefix_len` copies flagged in
      `planning/Compiler Pipeline.md:25` and `src/types.rs:~925`; write each
      verdict into the Blast Radius section above.

Acceptance: the new structural test fails on exactly the six spellings and passes
on the two contrast cases; the audit list has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Add `element_prefix_len` and route the `List`/`Result` element (`:1096`) and
      the `Map`/`MapEntry` key and value (`:1100`, `:1103`) through it.
- [ ] Apply the same fix to any live sibling copy the Phase 1 audit found.
- [ ] Add `tests/syntax/threads/thread-res-collection-plane-invalid/` — the source
      fixture from the reproduction, with a `golden/build.log` showing
      `TYPE_THREAD_NOT_SENDABLE`. (plan-114-A Phase 3 later re-goldens this to
      `2-203-0137`; that is that plan's task, not this one's.)

Acceptance: the Phase 1 test passes for all eight spellings; the source repro
reports `2-203-0063 TYPE_THREAD_NOT_SENDABLE`; every "works ✓" row still works.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `cargo test --no-fail-fast` (redirect to a file and check cargo's own exit
      status — a piped `| tail` reports tail's status, not cargo's).
- [ ] `cargo check --all-targets`.
- [ ] `scripts/test-accept.sh target/release/mfb /tmp/bug463-scratch` — full. This
      is a diagnostics change and `artifact-gate.sh` is blind to diagnostic prose.
      Never pass a real directory as the second argument; it is `rm -rf`'d.
- [ ] `scripts/artifact-gate.sh target/release/mfb all` → expect `diffs=0`: no
      currently-valid program's type parses differently, so codegen must not move.
      A diff is a mis-measurement introduced by the fix — objdump one fixture to
      localize it, then fix it.
- [ ] Re-run the full contrast matrix above and confirm every row.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`

Acceptance: full suite green; `diffs=0`; every matrix row matches its expected
column.
Commit: —

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

## Open Decisions

- **Does the `Map` *key* position need the marker too?** `Map OF RES fs.File TO Integer`
  is already in the `res_collection_round_trips_byte_exact` corpus (`src/types.rs:1908`),
  so the spelling is legal and the key position should take the marker for symmetry.
  Recommendation: route both key and value through `element_prefix_len`; a Phase 1
  test row (`Thread OF Map OF RES fs.File TO Integer TO Integer`) settles whether it
  is currently broken as well.
- **Should `Result OF RES …` keep sharing the `List` arm?** The AST parser
  deliberately allows the marker there and lets type-checking reject it
  (`src/ast/expr.rs:657-661`). Recommendation: keep the arms shared so the string
  grammar and the AST parser agree; splitting them would create exactly the
  two-grammar drift this codebase has already paid for once.

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
