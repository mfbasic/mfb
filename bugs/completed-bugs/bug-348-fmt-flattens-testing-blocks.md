# bug-348: `mfb fmt` flattens every `TESTING` block to column 0 — `classify` has no arm for `TESTING`/`TGROUP`/`TCASE`

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (formatter destroys authored structure)

Status: Fixed
Regression Test: tests/syntax/testing/testing-run-valid (or a new `tests/` fmt fixture) — a `TESTING` block round-trips through `mfb fmt` unchanged

`mfb fmt` rewrites every line inside a `TESTING` block to column 0. `TESTING`,
`TGROUP`, and `TCASE` open real, nestable blocks in the language, but the
formatter's block classifier has no arm for any of them, so nothing is pushed
onto the indent stack while the matching `END TCASE` / `END TGROUP` /
`END TESTING` lines *are* classified as `Op::End` and pop it. The net effect is
that the entire body — group nesting, case nesting, and every statement inside —
prints at depth 0.

This is a formatter that silently destroys hand-authored structure in a file it
was asked to tidy. It is not merely cosmetic: 35 committed `.mfb` sources fail
`mfb fmt --check` for this reason, which means `--check` cannot be adopted as a
gate, and anyone who runs `mfb fmt` over the test tree produces a large,
meaningless diff that will bury a real change.

The single correct behavior a fix produces: `TESTING`, `TGROUP`, and `TCASE`
each open one indent level and their `END` closes it, so a correctly-indented
`TESTING` block is a fixed point of `mfb fmt`.

References:

- `src/fmt.rs:380-433` — `classify`; the `match k` has arms for `Type`, `Union`,
  `Enum`, `Match`, `Trap`, … and **none** for `Testing`.
- `src/fmt.rs:306-319` — the `Block` enum: `Func, Sub, Type, Union, Enum, If,
  For, While, Do, Match, Trap, Case`. No `Testing`/`Tgroup`/`Tcase`.
- `src/fmt.rs:123-132` — `enum Sig`; `TGROUP`/`TCASE` are contextual, so they do
  not scan as `Sig::Kw` and reach `classify` at all.
- `src/lexer.rs:99` — `Keyword::Testing` exists; `src/lexer.rs:1202, 1251`.
- `src/docs/spec/tooling/05_fmt.md:147-148` — the spec's `Block` variant list,
  which shares the hole verbatim.
- [[plan-18 test framework]] — introduced `TESTING`/`TGROUP`/`TCASE`.
- Found during the cleanup-focused source review (worktree `cleanup-review`).

## Failing Reproduction

Minimal, self-contained:

```sh
cat > /tmp/f2.mfb <<'EOF'
IMPORT io

TESTING
  TGROUP "g"
    TCASE "c"
      expectInteger(1, 1)
    END TCASE
  END TGROUP
END TESTING

FUNC helper() AS Integer
  IF 1 = 1 THEN
    RETURN 1
  END IF
  RETURN 0
END FUNC
EOF
cp /tmp/f2.mfb /tmp/f2.orig.mfb
target/debug/mfb fmt /tmp/f2.mfb
diff /tmp/f2.orig.mfb /tmp/f2.mfb
```

- Observed:

```
4,8c4,8
<   TGROUP "g"
<     TCASE "c"
<       expectInteger(1, 1)
<     END TCASE
<   END TGROUP
---
> TGROUP "g"
> TCASE "c"
> expectInteger(1, 1)
> END TCASE
> END TGROUP
```

- Expected: no diff. The input is already correctly indented.

On a real committed source, `tests/acceptance/src/math.mfb` (hand-indented
2/4/6/8), the same run flattens 20+ lines:

```
<   TGROUP "Builtin: math"
<     TGROUP "abs"
<       TCASE "scalar"
<         LET fixedNeg AS Fixed = 0 - 4.75F
<         expectInteger(math::abs(-7), 7)
---
> TGROUP "Builtin: math"
> TGROUP "abs"
> TCASE "scalar"
> LET fixedNeg AS Fixed = 0 - 4.75F
> expectInteger(math::abs(-7), 7)
```

Tree-wide (`mfb fmt --check` over each file individually — note that
`mfb fmt --check tests` fails with `PROJECT_JSON_MISSING` because the path
argument must be a file or a project directory, so a per-file loop is required):

```sh
for f in $(find tests examples -name '*.mfb' | sort); do
  ./target/debug/mfb fmt --check "$f" >/dev/null 2>&1 || echo "FAIL: $f"
done
```

- Observed: **51 files fail**, of which **35 contain a `TESTING` block** and 16
  do not. The 35:

```
tests/acceptance/src/{arithmetic,bits,collections,csv,encoding,errorCodes,
  filters,general,json,math,money,primitives,regex,strings,vector}.mfb
tests/rt-behavior/codegen/inplace-grow-free-bug47/src/main.mfb
tests/rt-behavior/general/bug137_bool_xor_call/src/main.mfb
tests/rt-behavior/http/{func_http_constructors_valid,func_http_read_crlf_invalid,
  func_http_respondPath_valid,func_http_route_valid,http_server_loopback}/src/main.mfb
tests/rt-behavior/lexical/lexical-literals/src/main.mfb
tests/rt-behavior/native/{native-struct-cstring-rt,native-struct-scalar-rt}/src/main.mfb
tests/rt-behavior/net/{func_net_parseQuery_valid,func_net_percentDecode_valid}/src/main.mfb
tests/rt-behavior/project/binding-global-list-literal/src/main.mfb
tests/rt-behavior/testing/testing-trap-parity/src/main.mfb
tests/rt-behavior/trap/{func-bare-trap-loop-leak-rt,func-bare-trap-rt,
  inline-trap-infallible-builtin-valid}/src/main.mfb
tests/rt-behavior/vector/{vector-inline-ops,vector-native-carrier,vector-promotion}/src/main.mfb
tests/rt-error/{parser_statement_block_depth,parser_tgroup_depth}/src/main.mfb
tests/syntax/lexical/bug89_parser_eof_open_delim/src/main.mfb
tests/syntax/math/func_math_log_fixedarray_invalid/src/main.mfb
tests/syntax/testing/{testing-assert-invalid,testing-coverage-valid,
  testing-nested-valid,testing-parse-invalid,testing-run-valid}/src/main.mfb
```

The reviewer's estimate of "5+" understated it by an order of magnitude; the
reviewer's specific prediction about `tests/acceptance/src/math.mfb:10-20` is
confirmed exactly.

**The other 16 failures are a different, unrelated defect** and are out of scope
here. They are hand-aligned column padding inside `CSTRUCT` bodies, e.g.
`tests/syntax/native/native-cstruct-valid/src/lib.mfb:23-25`, where the source
uses 4 spaces and `fmt` emits 2:

```
<     format     CInt32
---
>   format     CInt32
```

That is `fmt` disagreeing with a deliberate alignment convention, not a block
being lost. It should be filed separately.

Contrast (correct today): the same file's `FUNC`/`IF` block after the `TESTING`
block is indented correctly, and **code following a `TESTING` block is not
corrupted** — see the Root Cause note on `saturating_sub`.

## Root Cause

Two independent gaps that compound:

1. **`src/fmt.rs:classify` has no `K::Testing` arm.** The `match k` at
   `src/fmt.rs:401-432` enumerates every block opener the formatter knows —
   `K::Type`, `K::Union`, `K::Enum`, `K::Match`, `K::Trap` at `:425-429` — and
   falls through to `_ => None` at `:430`. `Keyword::Testing` (`src/lexer.rs:99`)
   hits that fallthrough, so `TESTING` pushes nothing.

2. **`TGROUP`/`TCASE` are contextual keywords**, not `Keyword` variants, so
   `scan_line` (`src/fmt.rs:141`) records them as `Sig::Other` and they never
   reach `classify` in the first place. Even adding a `K::Testing` arm leaves
   these two unhandled.

Meanwhile `K::End => Some(Op::End(next_keyword(sig, idx)))` (`src/fmt.rs:406`)
is **unconditional** — every `END` emits a close op regardless of whether a
matching open was ever recorded. So `END TCASE` / `END TGROUP` / `END TESTING`
each pop a stack that the corresponding opener never pushed to.

Why the damage is *contained* rather than catastrophic: `apply_ops`
(`src/fmt.rs:447`) pops with saturating arithmetic, so the three spurious pops
clamp the depth at 0 instead of underflowing into the enclosing frames. This is
confirmed by the synthetic reproduction above — the `FUNC helper()` following
`END TESTING` is indented correctly. The blast is therefore bounded to the
interior of `TESTING` blocks, which is what caps this at MEDIUM rather than HIGH.
It is *luck*, not design: nothing in `classify` or `apply_ops` documents that
unmatched `END`s are expected.

`src/docs/spec/tooling/05_fmt.md:147-148` lists the `Block` variants and reads
`Func, Sub, Type, Union, Enum, If, For, While, Do, Match, Trap, Case` — the same
twelve as the code, with `TESTING`/`TGROUP`/`TCASE` absent. A `grep -n
'TESTING\|TGROUP\|TCASE' src/docs/spec/tooling/05_fmt.md` returns nothing at all.
So spec and implementation are *consistently* wrong: the spec cannot be used to
adjudicate the bug and must be fixed alongside the code.

## Goal

- `TESTING`, `TGROUP`, and `TCASE` each open exactly one indent level; the
  matching `END` closes exactly that level.
- A correctly-indented `TESTING` block is a fixed point: `mfb fmt` over it
  produces no diff.
- All 35 `TESTING`-bearing files above pass `mfb fmt --check`.

### Non-goals (must NOT change)

- Indentation of any construct that formats correctly today (`FUNC`, `IF`,
  `MATCH`/`CASE`, `TRAP`, `FOR`, …). No golden or fixture indentation outside
  `TESTING` blocks may move.
- The 16 `CSTRUCT` column-alignment failures — a separate defect; do not
  opportunistically change `fmt`'s handling of aligned field bodies here.
- Do **not** close this by reformatting the 35 committed sources to match the
  broken output. That would flatten the test suite's readability to make a bug
  look fixed, and is explicitly forbidden.
- Do not make `END` conditional in a way that silently swallows a genuinely
  unmatched `END` in malformed input; `fmt` operates on possibly-invalid source
  and must remain total.

## Blast Radius

Found by `grep -n` over `src/fmt.rs` and a tree-wide `fmt --check` sweep.

- `src/fmt.rs:classify` (`:401-432`) — missing `K::Testing` arm; fixed here.
- `src/fmt.rs:Block` (`:306-319`) — needs `Testing`, `Tgroup`, `Tcase` variants;
  fixed here.
- `src/fmt.rs:scan_line` / `enum Sig` (`:123-132`, `:141`) — must recognize the
  contextual `TGROUP`/`TCASE` identifiers; fixed here. This is the part with real
  risk: they are contextual, so a bare identifier `tgroup` in expression position
  must **not** be treated as a block opener.
- `src/fmt.rs:next_keyword` (`:441`) — `END TGROUP`/`END TCASE` name contextual
  identifiers, not `Keyword`s, so `next_keyword` returns `None` for them. Only
  `Op::End(Some(Keyword::Match))` is special-cased today (`:459`), so `None` is
  currently harmless — but confirm this when `Tcase`/`Tgroup` blocks exist.
- `src/docs/spec/tooling/05_fmt.md:147-148` — the `Block` list; fixed here.
- The 35 `TESTING`-bearing sources listed above — become `fmt --check`-clean
  without editing them.
- The 16 `CSTRUCT`-alignment failures — **latent, different hazard, out of
  scope**: `fmt` disagrees with hand-aligned field columns, which is a policy
  question about aligned bodies, not a lost block. File separately.
- `scripts/`, `.github/workflows/` — **unaffected**: `grep -rn 'fmt --check'
  scripts/ .github/ .ai/` returns nothing, so no gate currently runs `fmt
  --check`. That is precisely why 35 files could drift; adopting such a gate
  should follow this fix.

## Fix Design

Add three `Block` variants (`Testing`, `Tgroup`, `Tcase`) and make the scanner
recognize the two contextual openers.

- `TESTING` is a real `Keyword`, so it needs only a `K::Testing =>
  Some(Op::Open(Block::Testing))` arm in `classify`.
- `TGROUP`/`TCASE` need a narrow recognition rule in `scan_line`: treat the
  identifier as a block opener **only when it is the first significant token on
  the line**, mirroring the `is_first` guard already used for `K::If` (`:402`)
  and `Op::Else` (`:403`). This is what keeps a variable or call named `tcase`
  from opening a phantom block.
- `END TGROUP` / `END TCASE`: the existing unconditional `Op::End` already does
  the right thing once the opener pushes. The `prev_kw == Some(K::End)` early
  return at `src/fmt.rs:390-392` suppresses a *keyword* following `END`; verify
  the analogous suppression exists for the contextual identifiers so
  `END TGROUP` does not both close and re-open.

Rejected: promoting `TGROUP`/`TCASE` to real lexer `Keyword`s. It would make
`classify` uniform, but they are contextual by design (plan-18) and reserving
them would be a language change breaking any program using those identifiers —
far out of proportion to a formatter fix.

Rejected: special-casing "inside a `TESTING` block, indent everything by
`stack.len()`". It fixes the symptom without modeling the nesting, so
`TGROUP` inside `TGROUP` (which `tests/acceptance/src/math.mfb:10-11` uses) would
still collapse.

**Expected output shift:** none in any committed file, because the 35 affected
sources are already correctly indented by hand — the fix makes `fmt` agree with
them rather than the reverse. If any file *does* shift, that file was
mis-indented at commit time and the shift must be inspected individually.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a fixture asserting a `TESTING` block (with nested `TGROUP`) is a fixed
      point of `mfb fmt`; confirm it fails today with the flattening above.
- [ ] Record the full `fmt --check` sweep and the 35/16 split (done — see Failing
      Reproduction). File the 16 `CSTRUCT`-alignment failures as a separate bug.

Acceptance: the new fixture fails with lines at column 0; the 35/16 split is
written into this file.
Commit: —

### Phase 2 — the fix

- [ ] Add `Testing`, `Tgroup`, `Tcase` to `src/fmt.rs:Block`.
- [ ] Add the `K::Testing` arm to `src/fmt.rs:classify`.
- [ ] Recognize first-token `TGROUP`/`TCASE` in `src/fmt.rs:scan_line`, guarded
      so a contextual identifier elsewhere on the line opens nothing.
- [ ] Confirm `END TGROUP`/`END TCASE` close exactly one level and do not
      re-open.

Acceptance: the Phase 1 fixture passes; the synthetic reproduction produces no
diff; a `tgroup` identifier used as a variable still formats correctly.
Commit: —

### Phase 3 — sweep + spec sync + validation

- [ ] Re-run the tree-wide `fmt --check` loop. Expect exactly 16 failures
      remaining, all `CSTRUCT` alignment; confirm zero `TESTING`-bearing files
      remain.
- [ ] Update `src/docs/spec/tooling/05_fmt.md:147-148` to list the three new
      `Block` variants and describe the contextual-opener rule.
- [ ] Run the full acceptance suite:
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Confirm **no** committed `.mfb` file was modified by the fix.

Acceptance: 35 → 0 `TESTING` failures; 16 `CSTRUCT` failures unchanged; full
suite green; spec matches code; zero source churn.
Commit: —

## Validation Plan

- Regression test(s): a `TESTING`-block fixed-point fixture under
  `tests/syntax/testing/`, plus the negative case (contextual `tgroup` as an
  ordinary identifier).
- Runtime proof: the tree-wide per-file `fmt --check` loop drops from 51
  failures to 16, with every remaining failure attributable to `CSTRUCT`
  alignment.
- Doc sync: `src/docs/spec/tooling/05_fmt.md:147-148` — required; spec and code
  are currently wrong in the same way, so fixing only the code would leave the
  spec authoritative-and-false.
- Full suite: `cargo test` and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Adopt `mfb fmt --check` as a gate once this lands?** Recommend yes — no
  script or workflow runs it today (`grep -rn 'fmt --check' scripts/ .github/
  .ai/` is empty), which is the direct reason 35 files drifted unnoticed. It
  should not be adopted before the 16 `CSTRUCT` failures are also resolved.

## Severity reasoning

MEDIUM, not HIGH: `mfb fmt` is a shipped user-facing tool that silently rewrites
source and destroys authored structure across 35 committed files, and `fmt
--check` is unusable as a gate. But the damage is *bounded* — `apply_ops` clamps
at depth 0, so code outside the `TESTING` block is untouched, the output still
parses, and no program behavior changes. Not LOW, because a formatter that
mangles the file it was asked to tidy erodes trust in the tool and buries real
diffs.

## Summary

The engineering risk is concentrated in one place: recognizing the *contextual*
`TGROUP`/`TCASE` identifiers as block openers without misfiring on ordinary
identifiers that happen to share the spelling. The `K::Testing` arm and the three
`Block` variants are mechanical. The spec must be corrected in the same change,
since `05_fmt.md:147-148` currently ratifies the bug. No committed source should
move.

## Resolution

`Block` gained `Testing`, `Tgroup` and `Tcase`, and the two kinds of opener are
recognized differently because the language treats them differently:

- `TESTING` **is** a keyword, so it is classified from the keyword stream like every
  other opener: `K::Testing => Some(Op::Open(Block::Testing))`.
- `TGROUP` and `TCASE` are **contextual identifiers**. They never scan as
  `Sig::Kw`, so they cannot be classified from the keyword stream at all — a new
  `contextual_block_opener` recognizes them word-wise at the start of a line. Their
  `END TGROUP` / `END TCASE` closers already reached the ordinary `END` handling,
  which is precisely why the stack was being popped for frames nothing had pushed.

### Measured, not asserted

The report's claim that 35 committed sources fail `mfb fmt --check` was checked by
sweeping every `.mfb` under `tests/`, `bindings/` and `examples/` that mentions
`TESTING`: **36** failed before the fix, **2** after. Both survivors were then
examined, and neither is an indentation flattening:

- `tests/rt-error/parser_tgroup_depth/src/main.mfb` is a *deliberately malformed*
  fixture — deeply nested unclosed `TGROUP`s that exist to exercise the parser's
  depth cap. An unbalanced file is not a fixed point of any formatter, and it should
  not become one.
- `tests/acceptance/src/money.mfb` differs only by `true` → `TRUE` literal recasing,
  which is fmt's normal and correct behaviour on a source that happens to spell them
  lowercase. Its indentation is untouched.

So the flattening is fully closed, and `--check` is now adoptable as a gate for
everything except one intentionally-broken fixture.

Both directions are covered by tests, which matters because they fail for different
reasons: flattened input must be *restored* to the authored nesting, and correctly
indented input must be a *fixed point*. A third test pins that a `TESTING` block no
longer disturbs the indentation of the code following it — the concrete damage the
unbalanced pops caused.

`spec/tooling/05_fmt.md`'s `Block` variant list, which the report notes shared the
hole verbatim, now lists all three and explains the keyword/contextual split.

Full `cargo test` green; artifact gate 0 diffs.
