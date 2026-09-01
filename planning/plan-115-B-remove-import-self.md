# plan-115-B: Remove `IMPORT self`

Last updated: 2026-08-31
Effort: medium (1h–2h)
Depends on: plan-115-A

With letter A landed, a bare `ISOLATED FUNC` name is a valid `thread::start`
entry, which leaves `IMPORT self` with nothing to do. Its sole documented purpose
is intra-package thread fan-out (`src/docs/spec/language/13_modules-and-packages.md:113`),
and it is pure syntactic sugar — `self::worker` canonicalizes to bare `worker`
before lowering (`src/ir/lower.rs:3047`), so it carries no semantic weight
whatever.

After this letter: `self` is an ordinary identifier again. `IMPORT self` is no
longer recognized as a reserved specifier and resolves through the normal
package-resolution order — i.e. it becomes `IMPORT_PACKAGE_NOT_DECLARED` unless a
package literally named `self` is declared. The `IMPORT_SELF_IN_EXECUTABLE` rule
is deleted.

This is a **breaking source change** for any project spelling `self::`. It is
taken as a hard removal rather than a deprecation because `self::` and the bare
name would otherwise be two spellings of one thing, which is worse than either.

References:

- `planning/plan-115-A-unified-thread-entry.md` — **the Prerequisites table for
  the whole plan-115 feature lives there.** Re-run both commands before starting
  this letter.
- `planning/completed/plan-81-import-self.md` — the plan being reverted. Its §4.1
  (reserved specifier), §4.3 (`IMPORT_SELF_IN_EXECUTABLE`) and §4.4
  (canonicalization) are the three seams to unwind.
- `scripts/sync-package-mfp.sh` — the only sanctioned way to regenerate a
  committed `.mfp` and every copy of it.
- `AGENTS.md` — "Never edit a test/golden to pass". Three fixtures here exist
  *only* to pin `IMPORT self` behavior; deleting them is correct because the
  feature is gone, and that reasoning belongs in the commit message.

## Prerequisites

See `plan-115-A-unified-thread-entry.md` § Prerequisites — bug-480 and bug-482,
both of which gate the whole plan-115 feature. **Re-run both commands now**; a
status recorded in A is a snapshot, not the truth.

Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-115-A is complete and archived | `ls planning/plan-115-A-*.md` → no matches (moved to `planning/completed/`) | **NOT MET** (measured 2026-09-01: → `planning/plan-115-A-unified-thread-entry.md`; A has not started — its own bug-482 prerequisite is NOT MET) |

If A is not complete, this letter cannot start, full stop — removing `self::`
without bare-entry support breaks the 8 call sites measured below with no
replacement spelling.

## 1. Goal

`grep -rn "SELF_IMPORT" src/` returns 0 hits, `IMPORT self` is not special-cased
anywhere, and every fixture and package source that used `self::` has been
converted to the bare spelling and still passes — including the runtime proof
that the isolation semantics are unchanged by the conversion.

### Non-goals (explicit constraints)

- **No semantic change whatsoever.** `self::worker` and `worker` already lower
  identically (`src/ir/lower.rs:3047`); this letter deletes the former spelling
  and nothing else. Any behavioral diff is a bug in this letter.
- **`self` does not become a reserved word.** It was "special only in the
  import-root position" (§13); after this letter it is special nowhere.
  `LET self = 1` must keep working — verify, don't assume.
- **No change to the entry rules** — those landed in A.
- **No `.mfp` binary-format change.** The one `.mfp` regenerated here changes
  because its *source* changed, not because the format did.

## 2. Current State

### The six `src/` seams

`grep -rn "SELF_IMPORT" src/` → 10 occurrences across 6 files:

| File | What it does |
|---|---|
| `src/ast/types.rs:23` | `pub const SELF_IMPORT: &str = "self";` — the constant itself |
| `src/resolver/packages.rs:12-19` | Short-circuits resolution: returns early for `self`, emitting `IMPORT_SELF_IN_EXECUTABLE` when not a package |
| `src/resolver/resolution.rs:521` | The `IMPORT other AS self` guard (`SYMBOL_DUPLICATE_IMPORT`) |
| `src/resolver/mod.rs:292` | The `is_package` field doc, which exists to gate `IMPORT self` |
| `src/ir/lower.rs:3047` | `canonical_import_name` — rewrites `self.x` → `x` |
| `src/ir/shape.rs:740, 799, 2100, 2120` | Two "skip probing for a `self.mfp`" guards, plus the entry-name canonicalization and the (A-simplified) entry arm |

`src/resolver/mod.rs:292`'s `is_package` field may have other consumers — check
before deleting it (`grep -n "is_package" src/resolver/`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `IMPORT self` declarations in `.mfb` | 7, in 7 files | `grep -rnE "^[[:space:]]*IMPORT self([[:space:]]+AS[[:space:]]+[A-Za-z_]+)?[[:space:]]*$" --include="*.mfb" tests/ examples/ tools/ \| wc -l` → 7 |
| `thread::start(self::` call sites | 8, in 4 files | `grep -rn "thread::start(self::" --include="*.mfb" tests/ examples/ tools/ \| wc -l` → 8 |
| `SELF_IMPORT` occurrences in `src/` | 10, in 6 files | `grep -rn "SELF_IMPORT" src/ \| wc -l` → 10 |
| Files mentioning `IMPORT_SELF_IN_EXECUTABLE` | 7 | `grep -rln "IMPORT_SELF_IN_EXECUTABLE" src/ tests/` |
| Committed `self_fanout_workers.mfp` copies | 2 | `find . -path ./target -prune -o -name "self_fanout_workers.mfp" -print` |

**Beware a grep false positive here:** plain `grep -rn "IMPORT self"` returns 11
hits across 9 files, because it also matches `IMPORT self_fanout_workers`
(a prefix) and two prose comments. The anchored regex above is the correct
census; use it.

### The 7 declaration sites, by disposition

| File | Project kind | Disposition |
|---|---|---|
| `tools/thread-package-sources/self_fanout_workers/src/lib.mfb` | package | Convert `self::bump` → `bump`; **regenerate its `.mfp` (2 copies)** |
| `tests/syntax/threads/func_thread_start_self_valid/src/lib.mfb` | package | Convert to bare; rename fixture (see Phase 3) |
| `tests/syntax/threads/func_thread_start_self_invalid/src/lib.mfb` | package | Convert; its 2 call sites pin *rejections* — re-derive what each still rejects under A's rules |
| `tests/syntax/threads/func_thread_start_self_http_fanout/src/lib.mfb` | package | Convert 3 call sites to bare |
| `tests/syntax/project/import-self-package-valid/` | package | **Delete** — pins that `IMPORT self` resolves; feature gone |
| `tests/syntax/project/import-self-alias/` | package | **Delete** — pins `IMPORT self AS me` |
| `tests/syntax/project/import-self-in-executable/` | executable | **Delete** — pins `IMPORT_SELF_IN_EXECUTABLE`; rule gone |

### Verified properties

| Claim | How verified |
|---|---|
| `self::` is sugar with no semantic weight | Read `src/ir/lower.rs:3040-3050`: the `SELF_IMPORT` arm returns the bare name; nothing else distinguishes it |
| `self_fanout_workers`'s source is in-tree and rebuildable | `tools/thread-package-sources/self_fanout_workers/src/lib.mfb` exists; `scripts/sync-package-mfp.sh` header names `tools/thread-package-sources/*` as a build root |
| Only one `.mfp` in the tree is affected | The census's 7 declaration sites include exactly one under `tools/thread-package-sources/` |
| `LET self = 1` works today | UNVERIFIED — Phase 1 task |

## 3. Design Overview

Strictly ordered: convert every consumer to the bare spelling **first** (which is
a no-op under A's rules, so it is independently landable and independently
verifiable), then delete the compiler support once nothing uses it. Deleting
first would red the fixtures and leave no way to tell a conversion mistake from a
deletion mistake.

**Where correctness risk concentrates.** The `.mfp` regeneration in Phase 2. A
committed `.mfp` is a compiled binary consumed by a fixture that does not rebuild
it; a stale or wrongly-built copy is silently mis-lowered rather than rejected
(the `sync-package-mfp.sh` header documents exactly this failure mode from
plan-58-C, which surfaced as a runtime SIGSEGV). It is scheduled before the
deletion so a regeneration mistake is attributable.

**Byte-identity is NOT this letter's gate** — but it is a uniquely strong
*sentinel* here, because the conversion is provably neutral by construction
(`self.x` and `x` produce the same lowered name). The `.ir` goldens for the three
converted thread fixtures **are expected to diff**, because deleting an `IMPORT`
line shifts every subsequent source line and the goldens embed line numbers.
`.ncode`/`.ncodesum` for those fixtures should be **unchanged** — a diff there is
a real bug in the conversion and must be root-caused (objdump one fixture), not
re-baselined.

### Rejected alternatives

- **Deprecate rather than remove** (accept both spellings for a release).
  Near-zero implementation cost, but it leaves two spellings for one thing
  permanently, and the census is 7 declarations in 7 files — a morning's work.
- **Keep `IMPORT self` as a no-op alias.** Same objection, plus it keeps a
  reserved word in the import-root position for no benefit.

## Compatibility / Format Impact

- **Breaking source change:** `IMPORT self` / `IMPORT self AS x` / `self::name`
  stop compiling. There is no deprecation window.
- **One rule deleted:** `IMPORT_SELF_IN_EXECUTABLE` (`2-201-0019`). Remove its
  row from `src/rules/table.rs` and `src/docs/spec/diagnostics/01_rule-codes.md`.
  **Do not reuse the code `2-201-0019`.**
- **One `.mfp` changes bytes** (`self_fanout_workers.mfp`, 2 committed copies)
  because its source changed. No format version bump.
- `.ir` goldens for the 3 converted thread fixtures shift (line numbers).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` **in the same
> commit as the work it describes**. Use `- [~]` for partial with one line on
> what remains. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`.
> Fill each `Commit:` the moment it lands. **An unticked box means NOT DONE.**

### Phase 1 — Convert every `self::` call site to the bare spelling

Provably neutral under A's rules and independently landable: the compiler still
accepts `IMPORT self`, so this phase can land and be verified on its own.

- [ ] Verify `LET self = 1` compiles today (a throwaway project). Record the
      result — it establishes whether this letter *restores* `self` as an
      identifier or it was never taken.
- [ ] `tests/syntax/threads/func_thread_start_self_valid/src/lib.mfb` — delete the
      `IMPORT self` line, rewrite `thread::start(self::echoText, …)` as
      `thread::start(echoText, …)`.
- [ ] `tests/syntax/threads/func_thread_start_self_http_fanout/src/lib.mfb` — same
      for the 3 `self::fetchStatus` sites; update the two REM lines that explain
      the `self::` mechanism.
- [ ] `tests/syntax/threads/func_thread_start_self_invalid/src/lib.mfb` — convert
      both sites. **Re-derive what this fixture still rejects.** It pins
      `self::hiddenWorker` and `self::plainWorker`; under A, a non-`EXPORT`
      isolated function is now *valid*, so at least one of its two cases has
      stopped being an error. Keep only the cases that are still rejections
      (e.g. non-`ISOLATED` entry) and regenerate `golden/build.log`.
- [ ] `tools/thread-package-sources/self_fanout_workers/src/lib.mfb` — delete
      `IMPORT self`, rewrite the two `thread::start(self::bump, …)` sites, and
      update the leading REM block (it explains the `self` specifier).
- [ ] Regenerate goldens for the three converted syntax fixtures via
      `scripts/sync-goldens.sh` (never hand-edit a `build.log`).

Acceptance: `grep -rn "thread::start(self::" --include="*.mfb" tests/ examples/ tools/`
→ 0 hits. All 34 syntax thread fixtures pass. `.ncodesum` for the converted
fixtures is unchanged (the conversion is name-identical after lowering); `.ir`
diffs are line-number-only and reviewed as such.
Commit: —

### Phase 2 — Regenerate `self_fanout_workers.mfp` and prove the rt fixture (largest blast radius)

The one binary artifact in this letter. Lands separately from the deletion so a
mis-built `.mfp` is attributable.

- [ ] Run `scripts/sync-package-mfp.sh` and confirm it rebuilt
      `self_fanout_workers` and updated **both** committed copies
      (`tools/thread-package-sources/self_fanout_workers/self_fanout_workers.mfp`
      and `tests/rt-behavior/threads/thread-self-fanout-rt/packages/self_fanout_workers.mfp`).
      Check the blast radius of the script before running it — it rebuilds every
      buildable package fixture in the tree, not only this one, and must not be
      run unchecked.
- [ ] `git status` — confirm exactly the two `.mfp` copies changed. If other
      `.mfp` files moved, stop: either they were already stale (a pre-existing
      condition to report, not to absorb) or the script did more than intended.
- [ ] Run `tests/rt-behavior/threads/thread-self-fanout-rt/` natively and confirm
      it still prints its `a=107 b=207 parent=7`-shaped result and
      `main_counter=7`. This is the proof the conversion preserved isolation
      semantics exactly.
- [ ] Update the fixture's REM header in `src/main.mfb` (it describes the
      `IMPORT self` mechanism) and regenerate its `.ir`/`build.log` goldens.
- [ ] Rename the fixture directory `thread-self-fanout-rt` →
      `thread-package-fanout-rt` and update `project.json` `name`, or record why
      the name is kept.

Acceptance: the rt fixture passes with a real native run producing the same
numbers as before the conversion. `git status` shows exactly the expected `.mfp`
and golden changes and nothing else.
Commit: —

### Phase 3 — Delete the compiler support

Only after nothing in the tree uses the feature.

- [ ] `src/resolver/packages.rs:12-19` — delete the `SELF_IMPORT` short-circuit
      so `self` falls through to normal package resolution.
- [ ] `src/resolver/resolution.rs:521` — delete the `IMPORT other AS self` guard.
      Confirm `SYMBOL_DUPLICATE_IMPORT` retains its other callers
      (`grep -n "SYMBOL_DUPLICATE_IMPORT" src/`); if this was its only one, decide
      explicitly whether the rule survives.
- [ ] `src/ir/lower.rs:3040-3050` — delete the `SELF_IMPORT` arm of
      `canonical_import_name` and its comment block.
- [ ] `src/ir/shape.rs` — delete the two `SELF_IMPORT` skip-guards (lines 740,
      799) and the entry-name arms (2100, 2120) left over from A.
- [ ] `src/ast/types.rs:23` — delete the `SELF_IMPORT` constant.
- [ ] `src/resolver/mod.rs:292` — the `is_package` field exists to gate this
      feature. `grep -n "is_package" src/resolver/` for other consumers; delete
      the field if it has none, otherwise just correct its doc comment.
- [ ] `src/rules/table.rs:297` — delete the `IMPORT_SELF_IN_EXECUTABLE` row.
- [ ] Delete the three fixtures: `tests/syntax/project/import-self-package-valid/`,
      `tests/syntax/project/import-self-alias/`,
      `tests/syntax/project/import-self-in-executable/`. Justify in the commit
      message: each pins behavior of a removed feature, so the four-question gate
      resolves as "the feature is gone", not "the test is wrong".
- [ ] `src/docs/spec/diagnostics/01_rule-codes.md:310` — delete the
      `2-201-0019` row. Do not reuse the code.
- [ ] `src/docs/spec/language/13_modules-and-packages.md:105-120` — delete the
      reserved-`self` paragraph.
- [ ] `src/docs/spec/language/16_threads.md:28` — remove the `IMPORT self`
      sentence (A already rewrote the surrounding rule).
- [ ] Tests: add `tests/syntax/project/self-is-an-ordinary-identifier/` — a
      project using `self` as a variable and as a function name, building clean.
      This is the guardrail that the removal restored the identifier rather than
      leaving a half-reserved word.

Acceptance: `grep -rn "SELF_IMPORT" src/` → 0 hits;
`grep -rn "IMPORT_SELF_IN_EXECUTABLE" src/ tests/` → 0 hits;
`cargo test --no-fail-fast` green; full `scripts/test-accept.sh` green with the
fixture count (`N ran`) down by exactly 3 and up by 1.
Commit: —

## Validation Plan

- **Tests:** three fixtures deleted, one added
  (`self-is-an-ordinary-identifier`), three converted, one rt fixture re-proven.
- **Coverage check:** watch the `N ran` count from `test-accept.sh` across the
  letter — it must move by exactly the expected delta. A silently skipped fixture
  is the known failure mode here.
- **Runtime proof:** `thread-self-fanout-rt` (renamed) executed natively, same
  numbers as before conversion. This is the only evidence that removing the
  spelling did not change behavior.
- **Doc sync:** `13_modules-and-packages.md`, `16_threads.md`,
  `01_rule-codes.md`. `src/docs/spec/threading/01_source-model.md` also mentions
  the specifier — letter C owns that file's rewrite; if this letter leaves a
  dangling reference there, note it in C rather than half-editing it.
- **Acceptance:** `cargo test --no-fail-fast`; `scripts/test-accept.sh`;
  `scripts/artifact-gate.sh all` (expect 0 `.ncodesum` diffs — the conversion is
  name-identical after lowering; any diff is a bug in Phase 1);
  `cargo check --all-targets`; `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Rename `thread-self-fanout-rt`?** Recommend **yes** →
  `thread-package-fanout-rt`; the fixture still proves package-hosted fan-out,
  just not via `self::`. Costs a golden path update. (§Phase 2)
- **Does `SYMBOL_DUPLICATE_IMPORT` survive?** Depends on whether the `AS self`
  guard was its only caller. Recommend **keep the rule** and delete only the
  guard, unless the grep shows zero other callers. (§Phase 3)

## Corrections

<!-- Filled in DURING execution. Record the claim, what was actually true, the
     evidence — and whether letter C derived scope from a wrong number here. -->

- (none yet)

## Summary

The engineering risk is concentrated in one artifact: the regenerated `.mfp` and
its two committed copies, where a mistake is silent rather than loud. Everything
else is deletion of code that provably does nothing (`src/ir/lower.rs:3047`
returns the bare name), verified by a conversion that landed and passed one phase
earlier.

Untouched: all runtime and codegen, the entry rules (A owns those), `EXPORT`
semantics, and `examples/network-server` — which letter C collapses.
