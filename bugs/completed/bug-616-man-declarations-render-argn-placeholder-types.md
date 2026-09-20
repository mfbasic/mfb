# bug-616: `mfb man` declarations show `AS Arg0` instead of the real return type (78 lines, `math` and `collections`)

Last updated: 2026-09-13
Effort: small
Severity: LOW
Class: Documentation (renderer)

Status: **FIXED (`d68ec886e`)** — landed on main 2026-09-19.
Regression Test: `cli::man::tests::a_declaration_resolves_an_arg_placeholder_to_the_parameters_type`
(the concrete `math::abs` overloads, the generic `collections::reduceRight`, and a
container `collections::append`) and the TOTAL sweep
`cli::man::tests::no_rendered_page_spells_an_arg_placeholder`.

## STATUS: FIXED (`d68ec886e`)

`scripts/man-manual.sh` renders **0** `ArgN` occurrences, down from 93. Every line the
manual diff removes contained an `ArgN`; none were collateral. The document's four quoted
examples now render as it specified — `math::abs(value AS Float) AS Float`,
`math::atan2(y AS Fixed, x AS Fixed) AS Fixed`, `collections::reduceRight(…) AS U`, and
`Returns \`List OF T\`.`

**Validation.** `cargo test --no-fail-fast`: **exit 0**, 199 result blocks, 0 failures
(4272 tests in the main binary). Run on the tree *after* merging main, which had advanced
by ~25 commits (another session's plan-139 landed the `zip` and `tar` packages and two new
`fs` builtins) — the sweep test covers their new man pages automatically and passes.

**Two corrections to this document, both recorded above:** its root-cause symbol
(`public_type_name`) no longer exists and is now `ParameterType::display`, and its census
of 78 undercounts the true surface of 93 (the extra 15 are the `Returns ArgN.` sentences
and the continuation lines of declarations that word-wrap mid-type). The stated Goal
already covered them.

**Blast Radius item closed as a non-issue:** `mfb doc` / `mfb pkg doc` do not share the
rendering — both render *user* documentation, not registry descriptors.

A descriptor can say "this overload returns the same type as argument n" with
`ParameterType::Arg(n)`. The renderer prints that internal placeholder literally,
so the reader sees a type that does not exist:

```
1. `math::abs(value AS List OF Integer) AS Arg0`
5. `math::abs(value AS Float) AS Arg0`
`math::atan2(y AS Fixed, x AS Fixed) AS Arg0`
`collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS Arg1`
```

A developer cannot write `Arg0`, and the page never says what it means.

**The single correct behavior a fix produces:** every declaration, Overloads line
and "Returns" sentence shows the concrete return type. For an overload with
concrete parameters that is `Arg(n)`'s parameter type (`math::abs(value AS Float)
AS Float`). For a generic parameter it is the type variable (`reduceRight(…) AS U`).
No rendered page contains `ArgN`.

Found by plan-125-C Phase 1 while reading `mfb man math atan2`. **Filed, not
fixed**, by user instruction during a documentation-only plan ("file all bugs,
make no fixes").

## Reproduction

```
./scripts/man-manual.sh > /tmp/manual.txt
grep -c -E '\b(AS|OF|TO) Arg[0-9]+\b' /tmp/manual.txt
# 78 at worktree-P-125 f011d27b9
grep -E '\b(AS|OF|TO) Arg[0-9]+\b' /tmp/manual.txt | grep -oE '`[a-zA-Z]+::' | sort | uniq -c
#  66 `math::    11 `collections::   (+1 wrapped reduceRight line)
```

Expected: `0`.

## Root cause

`src/types.rs:ParameterType::name` renders `ParameterType::Arg(n)` as
`format!("Arg{n}")`. ~~`src/cli/man.rs:public_type_name`~~ — **stale name,
corrected 2026-09-19**: that helper no longer exists; it is now
`src/types.rs:ParameterType::display`, the same `name().replace('.', "::")`
passthrough, and the bug is unchanged. It passes the placeholder through for
every declaration and return line (the `render_*` callers of
`implementation.return_type.display()`). Nothing substitutes the n-th
parameter's type before rendering.

## Non-goals

- Changing `ParameterType::name`'s output for diagnostics or IR, which other
  consumers may depend on (`src/types.rs` notes rendering stability for
  diagnostics). The fix belongs at the man-rendering layer.
- Changing any descriptor's `return_type`.

## Blast-radius audit

**Verdicts, 2026-09-19 (release compiler at `be60f76eb`):**

- [x] Every `ParameterType::Arg(` in `src/codegen/builtins/**` — **23 sites**
      (`grep -rn "ParameterType::Arg(" src/codegen/builtins/ | grep -v tests`):
      `math/mod.rs` (the `preserving_unary` shared shape) and `math/func_clamp.rs`,
      plus 15 `collections` members. All render through
      `ParameterType::display()`, all fixed by the one substitution.
- [x] Nested `Arg` inside `List OF Arg0` or `FUNC(...) AS Arg0` — **no descriptor
      builds one today**: every one of the 23 sites is a bare top-level return.
      The resolver is written recursively anyway so a future nested use resolves
      rather than silently shipping a placeholder.
- [x] HTML docs (`mfb doc`, `mfb pkg doc`) — **verdict: NOT shared, no change
      needed.** Both render *user* documentation, not the builtin registry:
      `mfb doc` renders source doc-blocks via `build_source_doc_page`, and
      `mfb pkg doc` reads the doc section out of a compiled `.mfp`
      (`binary_repr::read_package_docs`). No registry `Implementation::return_type`
      reaches either, and `grep -rn "render_declaration\|render_function_markdown"
      src/` has no hit outside `man.rs`.

**Census correction.** The document's target was the 78 lines matching
`(AS|OF|TO) ArgN`. The true surface is **93** occurrences: the other 15 are the
`Returns ArgN.` sentences and the continuation lines of declarations that word-wrap
mid-type. The stated Goal ("No rendered page contains `ArgN`") already covers
them, so the fix and its sweep test target 93 → 0, not 78 → 0.

## Fix

Phase 1 — [x] renderer unit tests: `a_declaration_resolves_an_arg_placeholder_to_the_parameters_type`
(the concrete `math::abs` overloads, the generic `collections::reduceRight`, and a
container `collections::append`) plus a TOTAL sweep,
`no_rendered_page_spells_an_arg_placeholder`, modelled on the existing
`no_rendered_page_spells_a_dotted_package_type` (bug-605) so a new render site
cannot reintroduce the leak. Both confirmed RED, listing all 93 occurrences.
Commit: `89b36ba68`

Phase 2 — [x] `src/cli/man.rs:resolved_return_type` substitutes `Arg(n)` with the
n-th parameter's type, recursively, at the two sites that render a return type
(`render_declaration` and `render_parameters`' "Returns" line). Kept at the
rendering layer per the Non-goals; `ParameterType::name` is untouched. No pin was
proven wrong — no existing test asserted an `ArgN` spelling.
Commit: `89b36ba68`
