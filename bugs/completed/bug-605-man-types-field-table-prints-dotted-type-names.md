# bug-605: `mfb man <pkg> types` field tables print internal dotted type names (`color.Color`)

Last updated: 2026-09-14
Effort: small (<1h)
Severity: LOW
Class: Documentation (renderer)

Status: FIXED (09fb11415)
Regression Test: `cli::man::tests::types_page_field_tables_use_public_package_qualification`,
`cli::man::tests::no_rendered_page_spells_a_dotted_package_type`,
`types::tests::display_spells_package_qualifiers_as_source_does`

## STATUS: FIXED (09fb11415)

`ParameterType::display()` (`src/types.rs`) is the one user-facing spelling. Every
`mfb man` render site and `RegistryRecord::render`'s injected companion source go
through it; `public_type_name` and `source_spelling` are deleted.

Deviations from the plan below:

- The Phase 2 equivalence check was run over more than record fields: every registry
  record field, function parameter and return type —
  `source_spelling(ty) == ty.display()` for 2745 types, 0 differ. So `display()` is a
  plain `.` → `::` replace, and the UNVERIFIED note in the blast-radius audit is
  settled. The one-off assertion was removed with `source_spelling`.
- Phases 1 and 2 landed as one commit (the tests were RED-verified first on the
  un-fixed tree: the guard reported exactly the six cells).
- Line numbers in the audit had drifted by landing time (field row at `man.rs:493`,
  `public_type_name` at `:597`); the sites were the same.
- Also reattached the orphaned "Construct the registry by registering every migrated
  package" doc comment, which had sat above `source_spelling`, to `fn build()`.

A record's field table on a types page spells a package type the way the compiler
stores it, with a dot, instead of the way source writes it:

```
│ color  │ color.Color │ The colour at that offset. A color::Color — the …
│ font   │ RES canvas.Font │ The font to draw it in. …
│ from   │ net.Address │ The address the datagram was sent from. …
```

MFBASIC source writes `color::Color`; `color.Color` is not valid syntax, and the
description in the same row says `color::Color`. The Parameters table on every
function page already converts the spelling.

**The single correct behavior a fix produces:** every type a user reads is spelled
with `::` for a package qualifier, produced by one method,
`ParameterType::display()`. No renderer converts `.` to `::` on its own. The six
cells below read `color::Color`, `RES canvas::Font`, `RES canvas::Image` and
`net::Address`.

Found by plan-125-B while sweeping the rendered man surface for dotted names.
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes").

## Reproduction

```
mfb man canvas types | grep 'color\.Color\|canvas\.Font\|canvas\.Image'
mfb man udp types | grep 'net\.Address'
```

Observed at `worktree-P-125` HEAD, and again on `main` at `a2519ecbb` (2026-09-13,
fresh release build): six cells in total —
`canvas::GradientStop.color`, `canvas::Paint.fill`, `canvas::Paint.stroke`,
`canvas::Text.font`, `canvas::Picture.image`, `udp::Datagram.from`.
(`python3 /tmp/p125-ex/dotted.py` walks `mfb man --all` for every table cell of
the form `pkg.Type`; those six are all it finds.)

Expected: no output from either command.

## Root cause — why this keeps coming back

`ParameterType` has no user-facing spelling:

- `ParameterType::name()` (`src/types.rs`) is the internal canonical spelling
  (`color.Color`). `ParameterType::parse` reads that spelling back, so it cannot
  change.
- `impl fmt::Display for ParameterType` (`src/types.rs`) is `write!(f, "{}", self.name())`,
  so `format!("{ty}")` is dotted too.

Every renderer therefore has to convert `.` to `::` itself, through a helper it may
not be able to reach:

- `src/cli/man.rs:public_type_name` — `ty.name().replace('.', "::")`, private to `man.rs`.
- `src/codegen/registry/mod.rs:source_spelling` — a per-package `pkg.` → `pkg::`
  replace loop, private to the registry.

Past fixes routed one render site at a time through `public_type_name`
(`372a205d1` "fix man type qualification", 2026-08-27; `8361ec0c6`, bug-591). The
field table in `render_types_markdown` has called `prop.ty.name()` directly since
the registry-driven renderer was written (`31276d8c4`, 2026-08-17), and no test
reads the rendered pages for dotted names. The only qualification test,
`function_types_use_public_package_qualification`, checks one function page.
Any site that forgets the conversion prints dots until someone notices.

## Non-goals

- Changing `ParameterType::name()` or `impl Display for ParameterType`. They are the
  internal spelling that `ParameterType::parse` round-trips, and that the `.ast`/`.ir`
  goldens carry.
- Changing type IDs (`COLOR_TYPE_ID = "color.Color"` and friends). They are the
  compiler's internal keys and correct as they are.
- Editing descriptor prose to work around the cell.
- `src/ir/shape.rs` line 1792 (`callee.replace('.', "::")`): it converts a *callee
  name* for a diagnostic, not a `ParameterType`, so `display()` does not apply to it.

## Blast-radius audit

Measured 2026-09-13 on `main` at `a2519ecbb`:

| Site | What it does today | Change |
|---|---|---|
| `src/cli/man.rs:public_type_name` (line 515) | `ty.name().replace('.', "::")` | delete; callers use `ty.display()` |
| `public_type_name` callers, production (`man.rs` lines 496, 510, 604, 762, 800) | parameter decls, return types, overload equality, Parameters table | `…​.display()` |
| `public_type_name` callers, tests (`man.rs` lines 1380, 1391; `mod tests` starts line 936) | test assertions | `…​.display()` |
| `src/cli/man.rs:render_types_markdown` field row (line 417) | `prop.ty.name()` — **this bug** | `prop.ty.display()` |
| `src/codegen/registry/mod.rs:source_spelling` (line 2033), caller line 674 | per-package `pkg.` → `pkg::` loop; renders record fields into injected companion SOURCE | `ty.display()` — see Phase 2's equivalence check |

Commands:

- `git grep -nE "name\(\)\.replace\(('\.'|\"\.\"), \"::\"\)" -- src` → `src/cli/man.rs:515` only
- `git grep -nE "replace\(('\.'|\"\.\"), \"::\"\)" -- src` → the above plus `src/ir/shape.rs:1792` (callee name, non-goal)
- `git grep -nE "public_type_name\(" -- src | grep -v "fn public_type_name"` → 7 call sites
- `git grep -nE "source_spelling\(" -- src` → the definition and one caller, `registry/mod.rs:674` (the third hit, `src/manifest/package.rs:1473`, is an unrelated test name)
- `grep -nE "prop\.ty\.name\(\)" src/cli/man.rs` → line 417

Union-member and resource sections of `render_types_markdown` have no raw `.name()` call.

~~UNVERIFIED~~ VERIFIED (2026-09-14): `.` never occurs in a registry `ParameterType`
name except as a package qualifier — the equivalence check over 2745 registry types
found 0 differences, so `display()` is a plain replace.

## Fix

Phase 1 — tests first (RED):

- [x] `src/cli/man.rs` tests: render the `canvas` and `udp` types pages. Assert they contain
  `color::Color`, `RES canvas::Font`, `RES canvas::Image` and `net::Address`, and no
  `color.Color`, `canvas.Font`, `canvas.Image` or `net.Address`. RED on today's tree.
- [x] `src/cli/man.rs` tests: a guard that renders every package's overview, types page and
  every function page. It fails on any `<pkg>.<UpperCaseLeaf>` for any registry package
  name, so a new render site that skips `display()` fails CI instead of resurfacing.
  RED on today's tree (the six cells).
- [x] `src/types.rs` tests: `display()` spells a qualified leaf, a `List OF`, a `Map OF … TO`,
  a `RES`, and a nested qualified type with `::`. It spells an unqualified builtin
  (`Integer`) unchanged.

Commit: 09fb11415 (with Phase 2)

Phase 2 — the method and the sites (GREEN):

- [x] `src/types.rs`: `pub(crate) fn display(&self) -> String`, the user-facing source
  spelling (`.` → `::` on package qualifiers). Doc comment: `name()` is internal;
  anything a user reads uses `display()`.
- [x] `src/cli/man.rs`: replace every `public_type_name(&x)` with `x.display()`, and delete
  `public_type_name`. Change the field row at line 417 to `prop.ty.display()`.
- [x] `src/codegen/registry/mod.rs`: `source_spelling(&prop.ty)` → `prop.ty.display()`;
  delete `source_spelling`. Equivalence check before deleting: for every registry
  record field, `source_spelling(&ty) == ty.display()`, as a one-off assertion over
  `registry().packages()`. If any differ, `display()` walks the type's leaves instead
  of doing a string replace, and the check re-runs.

Commit: 09fb11415

Phase 3 — verify:

- [x] The two Phase 1 man tests and the `display()` unit test pass:
  `cargo test --bin mfb -- cli::man:: types:: codegen::registry::` → 136 passed, 0 failed.
- [x] `mfb man canvas types` and `mfb man udp types` show the six cells with `::`. The
  Reproduction commands print nothing (fresh release build of 09fb11415; grep exit 1).
- [x] Registry companion source unchanged. The proof that covers every package is Phase 2's
  equivalence assertion (`source_spelling(&ty) == ty.display()` for every registry record
  field). Record registrations live in 14 packages, including canvas (19), which no
  byte-identity fixture builds (`git grep -nE "RegistryRecord \{|add_record\(" -- src/codegen/builtins`
  → astrings, audio, canvas, color, crypto, csv, datetime, http, json, net, regex, term,
  udp, vector; `ls tests/byte-identity` has no `canvas`).
  As an execution-free confirmation on two packages whose fixtures carry qualified record
  types: `scripts/artifact-gate.sh target/release/mfb net` and
  `scripts/artifact-gate.sh target/release/mfb udp` → `0 diff(s)` each.
  Measured: net 7 goldens, 0 diff(s); udp 7 goldens, 0 diff(s).
- [x] Full suite: `cargo test --no-fail-fast` on 09fb11415 → `EXIT=0`, 185 `test result: ok`
  lines, 5697 passed, 0 failed (includes the `artifact_gate_all` cross-target sweep).

Commit: none (verification only)
