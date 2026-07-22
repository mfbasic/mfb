# bug-357: `WHILE` is the only block closed by a bespoke keyword (`WEND`) instead of `END WHILE`

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: `tests/syntax/control-flow/while-end-while-valid/`, `tests/syntax/control-flow/while-wend-removed-invalid/`

Every multi-line block construct in MFBASIC closes with `END <kind>` — `END IF`,
`END FUNC`, `END SUB`, `END MATCH`, `END TYPE`, `END TESTING` — with the sole
exceptions of the two counted-loop forms (`NEXT`, `LOOP`) and `WHILE`, which
closes with the inherited-from-BASIC `WEND`. `NEXT`/`LOOP` at least pair with a
non-`END` opener idiom; `WHILE … WEND` is a plain condition-guarded block whose
terminator names nothing and reads as an unrelated token. A reader who has
learned `END IF`/`END FUNC`/`END MATCH` will reasonably write `END WHILE` and get
a confusing `MFB_PARSE_EXPECTED_EXPRESSION` pointing at `END`, because `END` is
parsed as the start of a statement and `WHILE` is then read as an expression.

**The single correct behavior a fix produces:** a `WHILE` block closes with
`END WHILE`, exactly as every other `END`-terminated block does, and the parser,
formatter, spec, man pages, editor tooling, and every in-tree `.mfb` source use
that form.

References:

- `src/docs/spec/language/10_control-flow.md` — the `WHILE … WEND` shape this bug changes
- `src/docs/spec/language/19_grammar.md` — the grammar production for the loop
- `src/docs/spec/language/02_lexical-structure.md` — the reserved-word list containing `WEND`
- `src/docs/spec/tooling/05_fmt.md` — formatter block/indent rules
- `src/docs/man/flow/while.md` — the user-facing `WHILE` page
- Found during: 2026-07-18 session, user-reported surface-consistency defect

## Failing Reproduction

```
$ mkdir -p /tmp/wendrepro/src
$ cat > /tmp/wendrepro/project.json <<'EOF'
{
  "name": "wendrepro",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main",
  "targets": ["native"]
}
EOF
$ cat > /tmp/wendrepro/src/main.mfb <<'EOF'
IMPORT io

SUB main()
  MUT x AS Integer = 0
  WHILE x < 3
    x = x + 1
  END WHILE
  io::print(toString(x))
END SUB
EOF
$ mfb build /tmp/wendrepro
```

- Observed (verbatim, `target/release/mfb`, macos-aarch64):

  ```
  Building wendrepro (executable) for macos-aarch64
     7 |   END WHILE
       |   ^^^
  /tmp/wendrepro/src/main.mfb:7 error[1-102-0001 MFB_PARSE_EXPECTED_EXPRESSION]: parser expected an expression
                 Expected an expression.
     7 |   END WHILE
       |   ^^^
  /tmp/wendrepro/src/main.mfb:7 error[1-102-0004 MFB_PARSE_UNEXPECTED_STATEMENT]: parser found an unexpected statement
  ```

- Expected: builds, and running it prints `3`.

Contrast cases that work correctly today (these bound the bug and become
regression guards — none may change):

- Replacing `END WHILE` with `WEND` in the same program builds and prints `3`.
  Verified on macos-aarch64.
- `END IF`, `END FUNC`, `END SUB`, `END MATCH`, `END TYPE` all parse, proving
  the two-token `END <kind>` machinery (`src/ast/lexical.rs:is_end_block` /
  `consume_end_block`) is already general — `WHILE` is simply not wired into it.
- `EXIT WHILE` and `CONTINUE WHILE` parse and target the innermost `WHILE`
  loop; they are a different production and must be unaffected.
- The single-line form `WHILE c : … : WEND` parses (see
  `tests/rt-behavior/control-flow/control-flow-behavior/src/main.mfb:105`).

Not platform-dependent: this is a parser-table fact, identical on every target.

## Root Cause

Not a latent defect — a deliberate BASIC-heritage choice that was never
revisited, hard-coded in three places:

- `src/lexer.rs:119` defines `Keyword::Wend`; `src/lexer.rs:1205` maps the source
  text `WEND` to it; `src/lexer.rs:1271` renders it back as `wend`;
  `src/lexer.rs:1446` lists it among the reserved words.
- `src/ast/parser.rs:56` declares `BlockTerminator::Wend`, and
  `src/ast/stmt.rs:795` resolves that terminator with a *single*-token
  `check_keyword(Keyword::Wend)` — unlike `BlockTerminator::EndIf`/`EndMatch`
  (`src/ast/stmt.rs:792-793`), which use the two-token `is_end_block(…)`.
- `src/ast/stmt.rs:677-678` parses the `WHILE` body up to that terminator and
  then requires it: `consume_keyword(Keyword::Wend, "WHILE block must end with WEND.")`.

Why the observed diagnostic is so unhelpful: with `WEND` the only accepted
terminator, `END` is never a `WHILE` block terminator, so
`parse_statement_block` treats the `END` line as another *statement* in the loop
body. `END` alone starts no statement, so the parser falls through to expression
parsing and reports `MFB_PARSE_EXPECTED_EXPRESSION` at `END` — never mentioning
`WHILE` or `WEND` at all. The contrast cases are immune because their
terminators already route through `is_end_block`, which peeks
`self.current + 1` for the block-kind keyword.

## Goal

- `WHILE <cond>` … `END WHILE` parses, type-checks, lowers, and executes
  identically to today's `WHILE … WEND`, in both multi-line and single-line
  (`WHILE c : … : END WHILE`) forms.
- `WEND` is removed from the language: it is no longer a reserved word, and a
  source file using it fails with a diagnostic that names `END WHILE` as the
  replacement rather than a bare `MFB_PARSE_EXPECTED_EXPRESSION`.
- `mfb fmt` indents and closes a `WHILE` block on `END WHILE` exactly as it does
  for `END IF`, with no stray dedent or block-nesting drift.
- Every in-tree `.mfb` source (stdlib packages, bindings, benchmarks, tools,
  test fixtures) uses `END WHILE`; no occurrence of `WEND` remains outside this
  document and the historical bug/plan archive.
- Spec, man pages, and editor tooling describe `END WHILE`.

### Non-goals (must NOT change)

- **`NEXT` and `LOOP` stay as they are.** This bug is scoped to `WHILE` only.
  Converting `FOR … NEXT` to `END FOR` or `DO … LOOP` to `END DO` is a separate,
  much larger surface change and is explicitly out of scope.
- **`EXIT WHILE` / `CONTINUE WHILE` semantics and syntax are untouched.** They
  name a loop kind; they must not start colliding with the new `END WHILE`
  terminator, and `WHILE` after `EXIT`/`CONTINUE` must still not open a block.
- **`DO WHILE <cond> … LOOP` is untouched.** `WHILE` after `DO` is a condition,
  not a block opener (`src/fmt.rs:430`), and stays that way.
- Loop semantics, IR lowering, codegen output, and `.mfp` encoding must not
  shift. The parse of `WHILE … END WHILE` must produce a byte-identical AST to
  today's `WHILE … WEND`, so no golden may change except where the fixture's own
  *source text* changed.
- **Tempting wrong fix, forbidden:** accepting `END WHILE` while quietly leaving
  `WEND` in place as a permanent second spelling. That leaves the language with
  two terminators for one block and does not achieve the stated goal — the point
  is uniformity, not additive tolerance. `WEND` may exist only transiently
  between Phase 2 and Phase 4 of this document.
- **Also forbidden:** re-baselining `tests/syntax/control-flow/control-flow-invalid/golden/build.log`
  or `.../control-flow-condition-types-invalid/golden/build.log` to whatever the
  new parser emits without reading the diff. Those goldens assert specific
  diagnostics; a changed line there must be justified as the intended new text,
  not accepted because the harness offered it.

## Blast Radius

Counts from a tree-wide search on 2026-07-18 (`grep -ri wend`, excluding
`target/` and `.git/`): **295 occurrences of `WEND` across 61 `.mfb` files**,
plus compiler, docs, and tooling sites below.

**Phase 1 re-audit (2026-07-22): the tree drifted.** Fresh word-boundary search
(`grep -rciw wend --include='*.mfb'`): **331 occurrences across 73 `.mfb`
files**. Sites present now but missing from the lists below:

- Embedded MFB sources in Rust unit tests: `src/audit/collect/source.rs`,
  `src/resolver/resolution.rs`, `src/scope_privates.rs`,
  `src/syntaxcheck/{checking,helpers}.rs`, `src/testing/desugar.rs`,
  `src/monomorph/lower.rs`, `src/ast/tests.rs`, `src/ir/tests.rs`, and
  `tests/native_resource_scope_drop.rs` (in addition to the listed
  `tests/native_loop_runtime.rs`).
- `scripts/gen_vector_package.py` (emits `WEND` into generated vector sources).
- A **third** golden containing the token:
  `tests/syntax/resources/use-after-move-still-fires-invalid/golden/build.log`
  (quotes a source line `WEND`); same inspect-line-by-line rule applies.
- Additional man pages: `builtins/bits/ctz`, `builtins/fs/{eof,readLine}`,
  `builtins/io/setBuffered`, `builtins/net/read`, `builtins/term/sync`,
  `builtins/thread/{cancel,isCancelled,isRunning}`, and `tour/package`.
- `src/docs/man/tour/package.md` (alongside the six listed tour pages).

**Formatter finding:** the predicted `src/fmt.rs:430` hazard is already covered
at HEAD — `classify` has a generic `prev_kw == Some(K::End)` guard (added for
`END FUNC` et al.) that stops the `WHILE` in `END WHILE` from opening a block,
and `K::End => Op::End(next_keyword(...))` already pops it. The new fmt unit
test is therefore expected to pass at HEAD and serves as a regression guard; the
formatter change this bug still owns is deleting the `K::Wend => Op::Pop` arm in
Phase 4.

Compiler — fixed by this bug:

- `src/lexer.rs:119,1205,1271,1446` (`Keyword::Wend`, text mapping, rendering,
  reserved-word list) — fixed by this bug.
- `src/ast/parser.rs:56` (`BlockTerminator::Wend`) — fixed by this bug.
- `src/ast/stmt.rs:677-678,795` (`WHILE` parse + terminator check) — fixed by
  this bug.
- `src/fmt.rs:361,430,437` (`Op::Pop` on `K::Wend`; the `while_is_condition`
  guard) — fixed by this bug. **This is where the correctness risk
  concentrates** — see Fix Design.
- `src/fmt.rs:848-849` (formatter unit test asserting
  `IF TRUE THEN RETURN 3 ELSE WHILE FALSE : WEND`) — fixed by this bug; the
  source text under test changes, so the expectation changes with it. This is a
  fixture-text update, not a weakened assertion: the assertion (single-line
  `WHILE` in an `ELSE` tail does not open a block) is preserved verbatim.

Stdlib / bindings `.mfb` sources — fixed by this bug (mechanical text change):

- `src/builtins/{collections,crypto,csv,datetime,audio,encoding,http,json,net,regex,vector}_package.mfb`
- `bindings/sqlite3/src/lib.mfb`

Test fixtures and goldens — fixed by this bug:

- ~48 fixtures under `tests/rt-behavior/**`, `tests/rt-error/**`,
  `tests/syntax/**`, `tests/acceptance/src/collections.mfb`, and
  `tests/native_loop_runtime.rs`.
- 2 goldens containing the token:
  `tests/syntax/control-flow/control-flow-invalid/golden/build.log` and
  `tests/syntax/control-flow/control-flow-condition-types-invalid/golden/build.log`.
  Each must be inspected line-by-line, not blind-regenerated (see Non-goals).

Docs — fixed by this bug:

- `src/docs/spec/language/{02_lexical-structure,10_control-flow,16_threads,19_grammar,20_worked-example}.md`
- `src/docs/spec/tooling/05_fmt.md`
- `src/docs/man/flow/{while,package}.md`
- `src/docs/man/builtins/tls/accept.md`
- `src/docs/man/tour/{package,01_c,02_java,03_go,04_typescript,05_python}.md`
  — the tour pages compare MFBASIC to other languages side-by-side; the MFBASIC
  column must be updated or the comparison teaches the removed syntax.

Editor / generator tooling — fixed by this bug:

- `tools/editors/vscode/syntaxes/mfbasic.tmLanguage.json` (keyword pattern)
- `tools/editors/vscode/language-configuration.json` (block-pairing rules)
- `tools/editors/vscode/mfbasic-parse.js`, `tools/editors/vscode/README.md`
- `tools/mfbgen/mfbgen.py` (emits `WEND` in generated sources)
- `tools/thread-package-sources/{os_env_race_workers,allocator_churn_worker}/src/lib.mfb`
- `benchmark/mfb/src/{main,list,iobench}.mfb`, `benchmark/mfb/workers/src/lib.mfb`
  — benchmark sources are compiled by the benchmark harness; missing one breaks
  the run, not the build.

Unaffected (verified, no change):

- IR, monomorph, codegen, `.mfp` encode/decode — unaffected because the change
  is purely in token→AST mapping; the `While` AST node and everything downstream
  of it are untouched.
- `EXIT`/`CONTINUE` parsing — unaffected because they consume the loop-kind
  keyword themselves and never consult `BlockTerminator`.
- `bugs/completed-bugs/**`, `planning/old-plans/**`, `planning/old-moved-to-src-spec/**`
  — historical records of what the language *was*; deliberately left as-is.

## Fix Design

The two-token `END <kind>` machinery already exists and is already used by
`END IF` and `END MATCH`. The parser change is small and low-risk:

1. Add `BlockTerminator::EndWhile`, resolved by `self.is_end_block(Keyword::While)`
   in `src/ast/stmt.rs:check_block_terminator` — the same shape as `EndIf`.
2. In `src/ast/stmt.rs:677-678`, parse the body against `EndWhile` and close with
   `consume_end_block(Keyword::While, "WHILE block must end with END WHILE.")`.
3. Delete `Keyword::Wend` and `BlockTerminator::Wend` (Phase 4), after the tree
   is migrated.

**Where the risk actually is: the formatter.** `src/fmt.rs:structural_ops`
decides block open/close from a flat keyword scan, and `K::While` currently maps
to a block *opener* unless preceded by `DO`/`LOOP` (the `while_is_condition`
guard at `src/fmt.rs:430`). Once `END WHILE` exists, a `WHILE` preceded by `END`
must also not open a block — otherwise `END WHILE` emits `Op::End(Some(While))`
*and* `Op::Open(Block::While)` on the same line, and every subsequent line in the
file is indented one level too deep. The guard must be widened to include
`K::End`, and `K::Wend`'s `Op::Pop` arm replaced by the `K::End` arm's existing
`Op::End(next_keyword(...))` path, which already handles a named block kind.

Second formatter hazard: the single-line form. `WHILE c : x = 1 : END WHILE` must
round-trip on one line, matching how `IF … THEN … ELSE …` single-line forms are
preserved today. The existing `src/fmt.rs:848` test covers exactly this shape for
`WHILE`/`WEND` and is the guard for it after the change.

Rejected alternatives:

- *Keep `WEND` as a permanent alias alongside `END WHILE`.* Rejected: it makes
  the inconsistency permanent and doubles the surface every reader, formatter
  rule, and syntax highlighter must know. The whole point is one spelling.
- *Change `NEXT`/`LOOP` to `END FOR`/`END DO` in the same change.* Rejected:
  strictly larger blast radius, independent value, and it would make the diff
  impossible to review. Worth its own bug if wanted.
- *Emit a deprecation warning on `WEND` and keep accepting it indefinitely.*
  Rejected for the same reason as the alias, plus it needs a warning
  infrastructure decision this bug should not make. The transitional window in
  Phases 2–4 exists only so the tree can be migrated in one reviewable commit.

Expected output shifts: goldens change **only** where a fixture's own source text
changed (a `.mfb` line containing `WEND` becomes `END WHILE`) or where a
diagnostic quoting the source line changed. No codegen, no binary, no `.mfp`
delta is expected — that invariance is itself an acceptance criterion.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/syntax/control-flow/while-end-while-valid/` — a fixture using
      both multi-line and single-line `END WHILE`, per the fixture conventions in
      `tests/syntax/`. Confirm it fails today with
      `MFB_PARSE_EXPECTED_EXPRESSION` at the `END` token. *(Confirmed: failed
      at HEAD with `MFB_PARSE_EXPECTED_EXPRESSION` + `MFB_PARSE_UNEXPECTED_STATEMENT`
      at the `END` token, exactly as the reproduction shows. The fixture also
      exercises nesting, `CONTINUE WHILE`, and `EXIT WHILE`.)*
- [x] Add a `src/fmt.rs` unit test asserting `END WHILE` closes the block and
      does not double-indent the following lines.
      *(Drift from this doc's prediction: the test **passes at HEAD** — the
      generic `prev_kw == Some(K::End)` guard in `classify` already stops the
      `WHILE` in `END WHILE` from opening a block. Kept as a regression guard;
      see the Blast Radius re-audit note.)*
- [x] Confirm the blast-radius counts above against a fresh tree-wide search and
      record any drift in this file. *(Done — see "Phase 1 re-audit" above:
      331/73 vs 295/61, plus previously unlisted Rust-embedded sources, a third
      golden, `scripts/gen_vector_package.py`, and extra man pages.)*

Acceptance: fixture fails for the documented reason; fmt hazard found already
covered at HEAD (test kept as guard); audit recorded.
Commit: —

### Phase 2 — accept `END WHILE` (parser + formatter)

- [x] `src/ast/parser.rs` — add `BlockTerminator::EndWhile`.
- [x] `src/ast/stmt.rs:677-678,795` — parse and consume `END WHILE` via
      `is_end_block`/`consume_end_block`.
- [x] `src/fmt.rs:430` — widen the non-opener guard so `WHILE` preceded by `END`
      does not open a block. *(No change needed — already covered by the generic
      `prev_kw == Some(K::End)` guard; proven by the Phase 1 test.)*
- [x] `src/fmt.rs:437` — route `END WHILE` through the existing `Op::End` path.
      *(No change needed — `K::End => Op::End(next_keyword(...))` already
      handles it; `K::Wend => Op::Pop` is deleted in Phase 4.)*
- [x] Leave `WEND` accepted for now, so the tree still builds mid-migration.

Acceptance: Phase 1 fixture builds and prints `multi=3 single=2 nested=4 exit=4`;
`WEND` still parses; all 3190 unit tests pass; 33-test control-flow acceptance
slice passes (covers `EXIT WHILE`, `CONTINUE WHILE`, `DO WHILE … LOOP` on the
unmigrated tree).
Commit: —

### Phase 3 — migrate the tree to `END WHILE`

- [x] Convert all occurrences (331 per the re-audit) across the 73 `.mfb`
      files: stdlib packages, `bindings/sqlite3`, benchmarks,
      `tools/thread-package-sources/**`, and every test fixture. Single-line
      `: WEND` becomes `: END WHILE`. Also the Rust-embedded MFB sources from
      the re-audit (`src/audit/collect/source.rs`, `src/resolver/resolution.rs`,
      `src/scope_privates.rs`, `src/syntaxcheck/*`, `src/testing/desugar.rs`,
      `src/monomorph/lower.rs`, `src/ast/tests.rs`, `src/ir/tests.rs`,
      `src/fmt.rs` test, `tests/native_{loop_runtime,resource_scope_drop}.rs`)
      and `scripts/gen_vector_package.py`. *(Note: `gen_vector_package.py` was
      already stale vs. the tree copy before this bug — plan-39 C2 replaced the
      isqrt algorithm in `vector_package.mfb` without updating the generator;
      only its `WEND` emission was fixed here.)*
- [x] Update `tools/mfbgen/mfbgen.py` and the three VS Code tooling files
      (tmLanguage keyword set drops `WEND`; `decreaseIndentPattern` drops
      `WEND` — `END` already matches `END WHILE` lines; `mfbasic-parse.js`
      `WHILE` close regex becomes `END\s+WHILE`; README updated).
- [x] Update spec (`02`, `10`, `16`, `19`, `20`, `tooling/05`) and man pages
      (`flow/while`, `flow/package`, `builtins/tls/accept`, all six `tour/*`,
      plus the re-audit's `builtins/bits/ctz`, `builtins/fs/{eof,readLine}`,
      `builtins/io/setBuffered`, `builtins/net/read`, `builtins/term/sync`,
      `builtins/thread/{cancel,isCancelled,isRunning}`, `tour/package`),
      per `.ai/specifications.md`. `02_lexical-structure` keeps `WEND` in the
      keyword set with a note that it is reserved but productionless (see
      Open Decisions).
- [x] Regenerate the **3** affected goldens (the 2 listed plus the re-audit's
      `use-after-move-still-fires-invalid`); diffs read line by line:
      `control-flow-condition-types-invalid` and `use-after-move-still-fires-invalid`
      change only the quoted source line (`WEND` → `END WHILE`), same errors,
      same lines. `control-flow-invalid` keeps both primary inline-IF
      assertions verbatim; its trailing *cascade* changes because recovery
      after the intentionally-invalid line 8 now consumes the bare `END`
      ("END must name the block kind it closes") and unwinds the `FUNC` early,
      yielding two top-level UNEXPECTED_STATEMENT follow-ons in place of the
      old WEND-as-expression pair. Intended text, not blind churn.

Acceptance: no `WEND` remains outside `bugs/`, `planning/old-*`, the compiler
impl (Phase 4), and the spec keyword-list note; 3190 unit tests green;
112-fixture acceptance slice over the migrated areas green with zero golden
churn beyond the 3 intended files.
Commit: —

### Phase 4 — remove `WEND` from the language

- [ ] `src/lexer.rs:119,1205,1271,1446` — delete `Keyword::Wend`, its text
      mapping, its rendering, and its reserved-word entry.
- [ ] `src/ast/parser.rs`, `src/ast/stmt.rs` — delete `BlockTerminator::Wend`
      and its arm.
- [ ] Add `tests/syntax/control-flow/while-wend-removed-invalid/` asserting that
      a source file using `WEND` is rejected, and that the diagnostic names
      `END WHILE`. Confirm the message is actionable, not a bare
      `MFB_PARSE_EXPECTED_EXPRESSION`.
- [ ] Confirm `WEND` is now a legal *identifier* (it is no longer reserved) and
      decide whether that is acceptable or whether it should stay reserved.

Acceptance: `WEND` no longer parses; its rejection diagnostic points at
`END WHILE`; full suite green.
Commit: —

### Phase 5 — full validation

- [ ] `cargo test` (full suite, **not** a filtered run — see AGENTS.md).
- [ ] `cargo fmt` (second pass in `repository/`, which is not a workspace member).
- [ ] Full acceptance/golden run; confirm zero unexplained golden churn.
- [ ] Re-run the Phase 1 reproduction end-to-end; confirm it builds and prints `3`.
- [ ] Confirm no binary/codegen delta: build a fixture before and after and
      diff the executable, per `scripts/artifact-gate.sh`.

Acceptance: full suite green; the reproduction passes; artifact gate shows no
codegen delta.
Commit: —

## Validation Plan

- Regression tests: `tests/syntax/control-flow/while-end-while-valid/` (the fix
  works, both forms), `tests/syntax/control-flow/while-wend-removed-invalid/`
  (the old form is gone and says so), and the new `src/fmt.rs` indent test.
- Runtime proof: the Phase 1 reproduction program builds and prints `3` —
  proving the loop actually executes, not merely that it parses.
- Codegen invariance proof: `scripts/artifact-gate.sh` shows no executable delta
  across the change, confirming this is a pure front-end change.
- Doc sync: spec `02`/`10`/`16`/`19`/`20`, `tooling/05`, and 9 man pages listed
  in Blast Radius — required, per `.ai/specifications.md`.
- Full suite: `cargo test` plus the acceptance/golden run (~15 min; poll with
  `pgrep -f`, never rebuild while it runs).

## Open Decisions

- **Should `WEND` remain a reserved word after removal?** Recommended: **keep it
  reserved** and reject it with a dedicated "use `END WHILE`" diagnostic
  (Phase 4). The alternative — freeing it as an ordinary identifier — is
  cheaper but means a stale source file silently reinterprets `WEND` as a
  variable name and fails somewhere far from the real cause.
- **Is the four-phase transitional window acceptable, or should Phases 2–4 land
  as one commit?** Recommended: **keep them separate**. Phase 3 is a 295-site
  mechanical diff; folding a semantic parser change into it makes both
  unreviewable. Each phase leaves the tree green, so nothing is lost.

## Summary

The parser change is genuinely small — `END WHILE` reuses the `is_end_block`
machinery that `END IF` and `END MATCH` already use, and no IR, codegen, or
`.mfp` behavior moves. The real engineering risk is in two places: the
**formatter**, where `WHILE` preceded by `END` must stop being treated as a block
opener or every following line indents one level too deep; and the **295-site
mechanical migration**, where the danger is not difficulty but omission — a
missed benchmark or `tools/` source fails only when that harness next runs, long
after the change looks done. Loop semantics, `EXIT`/`CONTINUE WHILE`,
`DO WHILE … LOOP`, and `NEXT`/`LOOP` are all untouched.
