# bug-356: `mfb fmt` flattens `CSTRUCT` and `BIND IN` blocks inside `LINK`, de-indenting their bodies by one level

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (formatter output; silently destroys authored structure)

Status: Open
Regression Test: tests/syntax/native/native-cstruct-valid (add an `mfb fmt --check` assertion), plus a `src/fmt.rs` unit test over a LINK block containing `CSTRUCT`, `BIND IN`, and `BIND STATE`

`mfb fmt` does not recognize `CSTRUCT … END CSTRUCT` or `BIND IN … END BIND` as
block constructs. Both are emitted at their enclosing depth instead of one level
deeper, so running `mfb fmt` on any source containing them silently de-indents
the block body by one indent level and collapses the nesting a reader relies on.
`mfb fmt --check` correspondingly reports these files as unformatted, so the
committed sources and the formatter disagree: **every one of the 8 `.mfb` files
in the repo that contains a real `CSTRUCT` block currently fails `mfb fmt
--check`**, including the shipped binding `bindings/libsnd/src/lib.mfb`.

The single correct behavior a fix produces: a `CSTRUCT` body and a `BIND IN`
body are each indented one level deeper than their opening keyword, and
`END CSTRUCT` / `END BIND` return to the opener's level — matching how `FUNC`,
`SUB`, and `FREE` are already handled inside a `LINK` block. Formatting an
already-correctly-indented file is then a no-op, and `mfb fmt --check` passes on
the committed tree.

This is silent: `fmt` reports success and rewrites the file. Nothing about the
output is a parse error, and no program behavior changes — the damage is to
authored structure, which is the one thing a formatter is trusted not to lose.

References:

- `src/docs/spec/tooling/05_fmt.md` — the formatter contract (§`Block` list; note it
  does not enumerate `CSTRUCT`/`BIND` either, so spec and code share the hole).
- `src/docs/spec/language/17_native-libraries.md` — `CSTRUCT` and `BIND IN` syntax.
- bug-348 (`mfb fmt` flattens `TESTING` blocks) — **same class, different code
  path**. bug-348 is a missing `Block` enum variant in `classify`
  (`src/fmt.rs:380`); this bug is a missing entry in `format_link_block`'s own
  separate opener/closer lists. The two fixes are independent and neither
  subsumes the other. They should land together.
- Found during the goal cleanup review of `src/fmt.rs`, while verifying the
  blast radius of bug-348 (`mfb fmt --check` fails on 51 files: 35 are bug-348,
  16 are this bug).

## Failing Reproduction

```
cargo build
cd tests/syntax/native/native-cstruct-valid
../../../../target/debug/mfb fmt --check
```

- Observed:
  ```
  Not formatted: …/tests/syntax/native/native-cstruct-valid/src/lib.mfb
  error[2-200-0101 FMT_CHECK_FAILED]: one or more source files are not formatted (mfb fmt --check)
                 1 file(s) are not formatted; run `mfb fmt` to fix.
  ```
- Expected: no output, exit 0 — the file is correctly indented as committed.

Running `mfb fmt` on a copy shows what it would write (`·` = space):

```
 LINK·"demo"·AS·demoLink
   CSTRUCT·SfFormatInfo·AS·AudioFormat
-····format·····CInt32
-····name·······CString
-····extension··CString
+··format·····CInt32
+··name·······CString
+··extension··CString
   END·CSTRUCT
```

The fields land at the same column as the `CSTRUCT` keyword that opens them.
Note the *intra-line* column alignment (`format·····CInt32`) is preserved — what
is lost is the block nesting level.

The same run on the shipped binding `bindings/libsnd/src/lib.mfb` shows the
second construct:

```
     CONST·datasize·=·SIZEOF·SfFormatInfo
     BIND·IN·info
-······format·=·index
+····format·=·index
     END·BIND
```

Contrast cases that work correctly today, which bound the bug: inside the same
`LINK` block, `FUNC` / `END FUNC`, `SUB` / `END SUB`, and `FREE` / `END FREE`
all indent their bodies correctly, because they are the three openers
`format_link_block` knows about. `LINK` / `END LINK` itself is also correct.

## Root Cause

`src/fmt.rs:585` `format_link_block` does not use the `classify` / `Block`
machinery that formats ordinary code (`src/fmt.rs:380`). It carries its own
two-line opener/closer test:

- `src/fmt.rs:605-607` — `is_opener` is `FUNC | SUB | FREE`.
- `src/fmt.rs:608-612` — `is_closer` is `END FUNC | END SUB | END FREE`.

`CSTRUCT` and `BIND` appear in neither list. Tracing a `CSTRUCT` block through
the loop at `src/fmt.rs:600-628`:

1. `CSTRUCT SfFormatInfo AS AudioFormat` — not `END LINK`, not `is_closer`, not
   `is_opener`, so it falls to the final `else` at `:624-625`, is emitted at the
   current `depth`, and **does not increment `depth`**.
2. Each field line — same `else` branch, emitted at the same unchanged `depth`.
3. `END CSTRUCT` — `first == "END"` but `second == "CSTRUCT"`, so it is neither
   the `END LINK` case at `:612-615` nor `is_closer`; it too falls to `else` and
   is emitted at the same `depth`.

So the entire construct is emitted flat at one level. `BIND IN … END BIND`
follows the identical path.

The contrast cases are immune because they are exactly the three names in the
two lists: `FUNC` hits `is_opener` at `:621-623` (emit, then `depth += 1`) and
`END FUNC` hits `is_closer` at `:618-620` (`depth -= 1`, then emit).

## Goal

- `mfb fmt` indents a `CSTRUCT` body and a `BIND IN` body one level deeper than
  their opening keyword, and returns `END CSTRUCT` / `END BIND` to the opener's
  level.
- `mfb fmt --check` passes on every committed `.mfb` file that contains a
  `CSTRUCT` or `BIND IN` block (currently 8 and 7 respectively).
- `mfb fmt` on an already-correct file is a byte-level no-op (idempotence).

### Non-goals (must NOT change)

- The intra-line column alignment of `CSTRUCT` field lines
  (`format·····CInt32`). `fmt` preserves interior whitespace today and must
  continue to; this bug is only about the leading indent.
- `FUNC` / `SUB` / `FREE` / `LINK` handling, which is already correct.
- The `classify` / `Block` machinery — that is bug-348's territory. Do not
  attempt to route `LINK` bodies through `classify` as part of this fix; that is
  a larger restructure (see bug-343, which proposes consolidating the formatter's
  duplicated recognizers) and would put this small fix at risk.
- **Do NOT "fix" this by re-indenting the committed `.mfb` sources to match the
  formatter.** The sources are correct; the formatter is wrong. Rewriting
  `bindings/libsnd/src/lib.mfb` to satisfy a broken `--check` would entrench the
  defect and destroy the authored structure permanently.

## Blast Radius

Found by `grep -rl CSTRUCT --include="*.mfb"` and an `mfb fmt --check` run over
each containing project:

- `bindings/libsnd/src/lib.mfb` — **fixed by this bug**. The only non-fixture
  site; a shipped binding, and the only file affected by *both* constructs
  (2 `CSTRUCT` blocks and 1 `BIND IN`).
- `tests/rt-behavior/native/native-struct-cstring-rt/src/main.mfb` — fixed by this bug.
- `tests/rt-behavior/native/native-struct-scalar-rt/src/main.mfb` — fixed by this bug.
- `tests/syntax/native/native-bind-state-valid/src/lib.mfb` — fixed by this bug.
- `tests/syntax/native/native-cstruct-escape-invalid/src/lib.mfb` — fixed by this bug.
- `tests/syntax/native/native-cstruct-invalid/src/lib.mfb` — fixed by this bug.
- `tests/syntax/native/native-cstruct-valid/src/lib.mfb` — fixed by this bug.
- `tests/syntax/native/native-struct-slot-invalid/src/lib.mfb` — fixed by this bug.
- `tests/syntax/native/native-bind-state-invalid/src/lib.mfb` — **unaffected**:
  it mentions `CSTRUCT` only in a comment and contains no `CSTRUCT` block. It is
  the sole file matching the grep that passes `--check`, and is therefore a
  useful negative control, not an exception to the pattern.

Repo-wide counts of the affected constructs (`grep -rhoiE` over `*.mfb`):
`END CSTRUCT` 15, `END BIND` 7, against `END FUNC` 2484 / `END SUB` 125 /
`END FREE` 4 / `END LINK` 42 for the already-correct openers.

Not in scope, same file, different mechanism: the 35 files failing `--check` for
bug-348 (`TESTING` blocks).

## Fix Design

Add both constructs to the two lists at `src/fmt.rs:605-612`.

**The `BIND` case has a trap that must not be missed.** `BIND` has two forms and
only one is a block:

- `BIND IN <slot>` … `END BIND` — a block. 7 occurrences, matched 1:1 by 7
  `END BIND`.
- `BIND STATE <res> = <slot>` — a **single line** with no `END BIND`. 3
  occurrences (e.g. `tests/syntax/native/native-bind-state-valid/src/lib.mfb:34`).

So the opener test must be `first == "BIND" && second == "IN"`, not a bare
`first == "BIND"`. A bare test would increment `depth` on each `BIND STATE` line
and never decrement it, progressively over-indenting the remainder of the
enclosing `LINK` block — a worse and more confusing corruption than the bug it
was meant to fix. The closer `END BIND` is unambiguous and needs no such guard.

Concretely:

```rust
let is_opener = first.eq_ignore_ascii_case("FUNC")
    || first.eq_ignore_ascii_case("SUB")
    || first.eq_ignore_ascii_case("FREE")
    || first.eq_ignore_ascii_case("CSTRUCT")
    || (first.eq_ignore_ascii_case("BIND") && second.eq_ignore_ascii_case("IN"));
let is_closer = first.eq_ignore_ascii_case("END")
    && (second.eq_ignore_ascii_case("FUNC")
        || second.eq_ignore_ascii_case("SUB")
        || second.eq_ignore_ascii_case("FREE")
        || second.eq_ignore_ascii_case("CSTRUCT")
        || second.eq_ignore_ascii_case("BIND"));
```

Rejected alternative: routing `LINK` bodies through `classify` / `Block` so all
block constructs are described in one place. That is the right long-term shape
and is proposed in bug-343, but it is a restructure of two independent indenters
and would make this one-line-per-list fix a multi-day change. Keep them separate;
this fix is a strict subset of that future work and does not conflict with it.

Note that `format_link_block`'s opener list being separate from `classify` is
itself the root cause pattern shared with bug-348 — two independently-maintained
lists of "what opens a block". Worth recording in bug-343 as motivating evidence.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a `src/fmt.rs` unit test formatting a `LINK` block that contains a
      `CSTRUCT`, a `BIND IN`, and a `BIND STATE`, asserting the expected indent
      for each. Confirm it fails for the documented reason.
- [ ] Add an `mfb fmt --check` assertion to the `native-cstruct-valid` fixture
      (or a dedicated fmt test) so the committed-source/formatter disagreement is
      caught in CI.
- [ ] Confirm the blast-radius list above by re-running the per-project
      `--check` sweep; record the verdict per file.

Acceptance: the new test(s) fail; the 8-file list is confirmed with
`native-bind-state-invalid` verified as the negative control.
Commit: `—`

### Phase 2 — the fix

- [ ] Extend `is_opener` and `is_closer` at `src/fmt.rs:605-612` per the design
      above, including the `BIND IN` guard.
- [ ] Verify idempotence: `mfb fmt` twice over each affected file produces the
      same bytes as once.

Acceptance: Phase 1 tests pass; `mfb fmt --check` is clean on all 8 files;
`FUNC`/`SUB`/`FREE`/`LINK` behavior is unchanged; `BIND STATE` lines do not shift.
Commit: `—`

### Phase 3 — validation

- [ ] Run the full acceptance suite
      (`scripts/test-accept.sh target/debug/mfb target/accept-actual`).
      No golden should shift: this changes only formatter output, and no fixture
      golden captures `mfb fmt` output for these files. If a golden does move,
      stop and establish why before regenerating.
- [ ] Re-run the repo-wide `mfb fmt --check` sweep and confirm the failure count
      drops by exactly the 8 files in the blast radius (the remaining failures
      are bug-348's 35 and should be unchanged).
- [ ] Confirm `bindings/libsnd` still builds and its runtime test still passes —
      the file is a real binding, not a fixture.

Acceptance: full suite green; `--check` failures reduced by exactly 8; libsnd
builds and runs.
Commit: `—`

## Validation Plan

- Regression test: the `src/fmt.rs` unit test over a `LINK` block containing all
  three of `CSTRUCT`, `BIND IN`, and `BIND STATE` — the third is the guard
  against the over-indent trap in Fix Design.
- Runtime proof: `mfb fmt --check` exits 0 across the 8 affected files, and
  `mfb fmt` on each is a byte-level no-op.
- Doc sync: `src/docs/spec/tooling/05_fmt.md`'s block list omits `CSTRUCT` and
  `BIND` as well — add both, so spec and code stop sharing the hole. (bug-338
  tracks the wider `05_fmt.md` drift; cross-reference rather than duplicating.)
- Full suite: `scripts/test-accept.sh`.

## Open Decisions

- Whether to land this together with bug-348 as one "formatter block-opener
  coverage" change. Recommended: **yes, same PR, separate commits** — they are
  one class of defect, they are both one-list edits, and landing them together
  lets a single `mfb fmt --check` sweep serve as the acceptance criterion for
  both. They touch different functions so the commits stay reviewable.

## Summary

Two block constructs are missing from `format_link_block`'s hand-maintained
opener/closer lists, so `mfb fmt` flattens them. The fix is four lines across two
boolean expressions; the engineering risk is concentrated entirely in the `BIND`
guard, where treating the single-line `BIND STATE` as a block opener would
over-indent the remainder of every enclosing `LINK` block. The existing
`FUNC`/`SUB`/`FREE` handling is the working reference, and the committed sources
are the correct expected output — they must not be rewritten to match the broken
formatter.
