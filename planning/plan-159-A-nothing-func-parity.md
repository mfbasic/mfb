# plan-159-A: `FUNC … AS Nothing` parity — the entry point and the rewriter

Last updated: 2026-09-26
Overall Effort: x-large (1d–3d) — the whole plan-159 "remove `SUB`" feature (guess, derived from the measured populations below)
Effort: medium (1h–2h) (guess)
Depends on: nothing

plan-159 removes the `SUB` keyword from MFBASIC. Every `SUB` becomes `FUNC … AS Nothing`,
and the language, the spec, `mfb man`, the compiler, the package format, the editor
grammar and every `.mfb` source in the tree stop knowing `SUB` exists. A `SUB` is not
worth its weight. It is already typed as `FUNC(...) AS Nothing` (`src/ir/lower.rs:function_return_type`),
and codegen does not tell the two apart. Measured: a `SUB` that uses `EXIT SUB`, and
the same function written as `FUNC … AS Nothing` with a bare `RETURN`, produce
byte-identical `.ncode` (see §2 Verified properties). What `SUB` does contribute are
a few rules that apply only to it. This letter removes the one place where
`FUNC … AS Nothing` is weaker than `SUB` (the entry point). After that, `SUB` has no
remaining benefit, and B–D can remove it without losing anything.

**`Nothing` is a first-class type (owner decision, 2026-09-26).** Its one value,
`NOTHING`, binds, passes, stores, compares and collects like any other value
(`mfb spec language types` §4.6, §4.10 defaults, §4.11 comparable; `mfb man types`).
So the value of a call whose callee returns `Nothing` is simply `NOTHING`:
`LET v = f()`, `g(f())`, `LET a = f(a)` and `LET v = io::print("x")` are all legal and
bind or pass `NOTHING`. Only a `SUB` call is value-less, and that rule leaves with
`SUB` in D. See Corrections: an earlier draft of this plan made such calls a compile
error; that was wrong and is withdrawn.

**The single outcome for A.** `FUNC main() AS Nothing` is a valid executable entry.
(The `LET v = f()` compiler panic is fixed by bug-744, not by this plan; see §2.)

The letters, in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | `FUNC main AS Nothing` entry; the `tools/sub-to-func/` rewriter (the old "value-less rule" phase is withdrawn — Corrections) | medium |
| **B** | every `.mfb` in the tree migrated (tests, examples, packages + their `.mfp`, benchmark, bugs, spikes, tools); goldens regenerated | large |
| **C** | every MFBASIC source embedded in Rust migrated: 821 `mfb man` examples, 34 injected built-in bodies, `mfb init`, test-harness generators, inline Rust test sources | large |
| **D** | `SUB` deleted from the compiler, package format, `wire`, fmt, `pkg info`, doc HTML, VS Code grammar; spec + man narrative rewritten; rules retired; final gate | large |

References:

- `mfb spec language subs` (`src/docs/spec/language/07_subs.md`): the SUB contract being removed
- `mfb spec language functions` (`06_functions.md`), `mfb spec language error-model` (`08_error-model.md`)
- `mfb spec tooling project-manifest` (`src/docs/spec/tooling/01_project-manifest.md:346`): the entry-point table
- `.ai/compiler.md` (checking topology: `ir::shape` vs `ir::verify`), `.ai/specifications.md`,
  `.ai/spec-content.md`, `.ai/man-content.md`, `.ai/testing-gates.md`

## Prerequisites

These conditions gate the whole of plan-159. B, C and D point here.

| Must be true | Command | Status (2026-09-27, re-measured at start of execution) |
|---|---|---|
| No other plan-159 exists | `find . /private/tmp -name 'plan-159*' -not -path './target/*'` → only these four files | MET (2026-09-27: the only other hits are byte-identical copies of these four files in other checkouts — `.claude/worktrees/735`, `/private/tmp/b740-base`, `/private/tmp/l754-base`; `cmp` → same for all 12) |
| `main` builds | `cargo build --release` → exit 0 | MET (2026-09-27: `cargo build --release` in `.claude/worktrees/P-159` → `Finished release`, exit 0) |

Everything below assumes both conditions hold.

> **NOTE: the Status column is a snapshot. The Command column is the truth.** Re-run
> each command before you start, and again before you decide to stop. If you stop,
> report the status of every prerequisite.

## 1. Goal

- `FUNC main() AS Nothing` and `FUNC main(args AS List OF String) AS Nothing` are
  valid executable entries. On success they behave exactly like `SUB main` (exit 0).
- A call whose callee returns `Nothing`, used in value position, yields `NOTHING`
  (user `FUNC`s and built-ins alike). **A adds no diagnostic for it.** The codegen that
  makes it work is bug-744's; A only relies on it. A `SUB` call in value position keeps
  `TYPE_SUB_HAS_NO_VALUE` (`2-203-0074`) until D deletes `SUB`, and D retires that
  rule with it.
- `tools/sub-to-func/` exists and mechanically rewrites a `.mfb` file from `SUB` to
  `FUNC … AS Nothing`. It is proven on the fixtures listed in Phase 3.

### Non-goals (explicit constraints)

- `SUB` still parses and compiles unchanged after A. Nothing is removed until D.
- No codegen change in A. A touches only the manifest entry check and a tool, so the
  `.ncode`/`.ncodesum` goldens must stay byte-identical.
- `2-203-0074` (`TYPE_SUB_HAS_NO_VALUE`) is not renamed or widened. It stays SUB-only.
- Never make a `Nothing` value an error anywhere: not a `FUNC … AS Nothing` call's
  result, not `NOTHING` itself, not `List`/`Map`/`Set OF Nothing` (bug-745).

## 2. Current State

**The value-less rule is SUB-only.** In `src/ir/verify/values.rs:check_value_depth`, the
`IrValue::Call`/`CallResult` arm emits `TYPE_SUB_HAS_NO_VALUE` only when
`self.functions.get(target).is_some_and(|sig| sig.kind == "sub")`. Statement position is
signalled through the one-shot `allow_sub_call` cell, which is set at
`src/ir/verify/ops.rs:114` and `:433` and declared at `src/ir/verify/mod.rs:785`. A
`FUNC … AS Nothing` or a built-in `Nothing` callee in value position passes the verifier
and then panics codegen:

```
$ cat /tmp/p159/val/src/main.mfb         # FUNC logIt() AS Nothing … LET v = logIt()
$ mfb build -q -ncode .
thread 'mfb' panicked at src/target/shared/abi.rs:243:14:
`void` is not a register operand (bug-721)
```

The `io::print` equivalent (`LET v = io::print("x")`) panics identically. This crash is
now **tracked as bug-744** (the fix makes each such call yield `NOTHING`; 12 value-position
shapes are covered by `tests/runtime/rt_nothing_func_value_position.rs`). Collections of
`Nothing` (`[NOTHING, NOTHING]`, `List OF Nothing`) fail the native build with an internal
error: **bug-745**, fixed so they work. Both are owner decisions of 2026-09-26 that
`Nothing` is a valid, first-class type. The panic text names bug-721 because bug-721's
typed-register change turned an older `unknown AArch64 register 'void'` into this panic.

**The entry check forbids `FUNC … AS Nothing`.** `src/manifest/entry.rs` (the
`FunctionKind` match near l.44 and the guard after it) rejects any `FUNC` entry whose
return type is not `Integer`:

```
./src/main.mfb:8 error[2-200-0011 PROJECT_ENTRY_INVALID]: project entry point is invalid
               Executable FUNC entry `main` must return Integer.
```

The spec mirrors this at `src/docs/spec/tooling/01_project-manifest.md:346`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `SUB` declarations in `.mfb` | 464 in 257 files | `rg -c '^\s*(PUBLIC\s+\|PRIVATE\s+)?SUB\b' -g '*.mfb' .` summed → 464; `-l \| wc -l` → 257 |
| `END SUB` / `EXIT SUB` in `.mfb` | 481 / 78 | `rg -c '\bEND SUB\b' -g '*.mfb' .` summed; same for `EXIT SUB` |
| `FUNC … AS Nothing` decls already in `.mfb` | 191 | `rg -c '^\s*(PUBLIC\s+\|PRIVATE\s+)?FUNC\b.*\bAS\s+Nothing\b' -g '*.mfb' .` summed |
| SUB headers whose parameter list spans lines | 0 | `rg -n '<decl regex>' -g '*.mfb' . \| rg '\(' \| rg -v '\)\s*(\x27.*)?$' \| wc -l` → 0 |
| SUB headers with a trailing comment | 0 | `rg -n "^\s*(PUBLIC\s+\|PRIVATE\s+\|EXPORT\s+)?SUB\b.*\x27" -g '*.mfb' .` → none |

The decl regex misses the `EXPORT SUB` and `ISOLATED SUB` forms. Use
`^\s*(PUBLIC\s+|PRIVATE\s+|EXPORT\s+)?(ISOLATED\s+)?SUB\b` from here on. Per directory
(declarations / files): tests 283/188, examples 76/19, packages 41/18, tools 34/18,
bugs 17/5, spikes 7/7, benchmark 6/2 (the loop `for d in …; rg -c "$R" -g '*.mfb' $d`).

### Verified properties

- **SUB and `FUNC … AS Nothing` produce identical codegen.** Two scratch projects,
  `/tmp/p159/sub` (`SUB logIt` with `EXIT SUB`) and `/tmp/p159/func` (`FUNC logIt … AS
  Nothing` with bare `RETURN`), were each built with `mfb build -q -ncode .`, then
  `cmp sub/p159.ncode func/p159.ncode` → **IDENTICAL**. That is one pair on the host
  target only. B re-proves it corpus-wide with the `.ncodesum` gate.
- **Bare `RETURN` works inside a function-level `TRAP` of a `FUNC … AS Nothing`**
  (`/tmp/p159/trap` builds). Fall-through into the `TRAP` is still caught as
  `TYPE_TRAP_FALLTHROUGH` ("body paths before TRAP must end with RETURN or FAIL"), so
  the `SUB` rule (error-model §8 rule 9) already has an exact `FUNC` twin.
- **A `FUNC … AS Nothing` passed to `collections::transform` is rejected**
  (`TYPE_UNKNOWN_VALUE`), which matches the spec's "a `SUB` is not valid for `transform`".
- **Codegen does not branch on the `"sub"` kind.** The `"sub"` string's remaining uses
  are validation-only (`src/target/shared/validate/names.rs:81,112`) and internal
  constructors (`src/target/shared/nir/lower.rs:219` for global initializers, test
  builders in `plan/mod.rs:556` and `validate/mod.rs:253`). The sites were read.
- **The native entry wrapper keys off the return type, not the kind** (task 2.2).
  `validate_entry_point` produces `ir::EntryPoint { returns: Nothing }` for a `SUB` entry
  and for a `FUNC … AS Nothing` entry alike; codegen's `lower_program_entry`
  (`src/codegen/engine/function/entry.rs`) reads only `language_entry_returns`
  (`rg -n language_entry_returns src`), and the package writer's entry flag
  (`binary_repr/writer.rs`, `entry.returns == Integer`) likewise. So the two entries
  lower identically.
- **UNVERIFIED, task 1.2:** whether `self.functions` in `ir::verify` holds built-in
  signatures. The `io::print` panic shows the check does not currently cover them, and
  the fix has to.

## 3. Design Overview

Two independent pieces, each landable alone. Neither changes codegen, so the
**correctness gate is byte-identity of `.ncode`/`.ncodesum`** plus the new fixtures.
A `.ncodesum` diff here is a bug to root-cause (`objdump` one fixture). It never means
the design is dead.

1. ~~Generalize the value-less rule to every `Nothing` callee.~~ **Withdrawn**
   (Corrections): `Nothing` is first-class, so a `Nothing` call's value is `NOTHING`.
   bug-744 fixes the panic by lowering it that way.
2. **Entry point:** `src/manifest/entry.rs` accepts `FunctionKind::Func` with
   `returns == Nothing` as well as `Integer`. The runtime exit-code contract for a
   `Nothing` entry is the one `SUB` uses today (`mfb spec language error-model`, entry
   contract; `package/03_metadata-encoding.md:73`).
3. **`tools/sub-to-func/`**, a line-based rewriter (the grammar allows it: 0 multi-line
   headers and 0 trailing header comments, measured above):
   - `<vis>? ISOLATED? SUB <rest>` → `<vis>? ISOLATED? FUNC <rest> AS Nothing`
   - `END SUB` → `END FUNC`
   - `EXIT SUB` → `RETURN`
   - It leaves comments and string literals alone, except that a `'`-comment line is
     never rewritten. `SUB` inside prose comments is left for a human, and the tool
     prints those lines as a report.
   - It is idempotent and has `--check` (exit 1 if anything would change).

**Rejected alternatives.**
- *Keep `SUB` as sugar for `FUNC … AS Nothing`.* The user's premise is that it adds
  nothing, and sugar keeps every surface in D alive.
- *Make using a `Nothing` call's value a compile error.* Rejected by the owner
  (2026-09-26): `Nothing` is a valid, first-class type, so its value must bind, pass
  and collect like any other. It would also have rejected ordinary code such as
  `LAMBDA(x) -> io::print(x)` and `RETURN f()` inside a `FUNC … AS Nothing`.
- *Rewrite with the compiler's AST serializer.* It would reformat whole files and
  churn every golden. The line rewrite touches only the SUB lines.

## Phases

> Tick `- [x]` in the same commit as the work. `- [~]` = partial (say what remains).
> Moot = `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — ~~The value-less rule covers every `Nothing` callee~~ — withdrawn

- [x] ~~1.1–1.6: generalize `2-203-0074` to every `Nothing` callee, rename it
      `TYPE_NOTHING_CALL_HAS_NO_VALUE`, add `nothing-call-value-invalid`.~~ — moot: owner
      decision 2026-09-26, `Nothing` is a first-class type, so a `Nothing` call's value is
      `NOTHING` and is legal (Corrections). **Do not implement this phase.** The panic it
      targeted is bug-744; collections of `Nothing` are bug-745.

Acceptance: n/a (withdrawn). A's remaining work is Phases 2 and 3.
Commit: —

### Phase 2 — `FUNC main … AS Nothing` is a valid entry

- [x] 2.1 RED: add `tests/rt-behavior/functions/entry-func-nothing-valid/` with
      `FUNC main() AS Nothing` printing one line, and golden `run.log`/exit 0. Add
      `tests/rt-behavior/functions/entry-func-nothing-fail-rt/`, where `main` does
      `FAIL`, whose golden must match the exit code/stderr `SUB main` gives for the same
      `FAIL`. Take that expectation from a sibling `SUB main` fixture, not from the new
      code. — Done: no tracked `SUB main` fixture FAILs out of `main` (every rt fixture
      with a non-zero `[exit` uses `FUNC main AS Integer`), so the expectation was
      measured on the unchanged compiler with a scratch `SUB main` (`/tmp/p159/subfail`:
      `SUB main` → `io::print("before")` → `FAIL error(700, …)`), run → `before` /
      `Error: 700` / message / `[exit 255]`. The two `build.log` goldens were hand-written
      from that before any code change; RED run: `test-accept.sh … 'entry-func-nothing*'`
      → `2 mismatch(es)`, both `PROJECT_ENTRY_INVALID … must return Integer.`
- [x] 2.2 `src/manifest/entry.rs`: accept `Func` whose `returns` is `Integer` **or**
      `Nothing`. Update the message to `must return Integer or Nothing`. Confirm that
      the runtime entry wrapper keys off the return type, not the kind, by reading the
      entry lowering the `SUB` case uses. Record it in Verified properties. — Done;
      recorded in §2 Verified properties (the wrapper is `lower_program_entry`).
- [x] 2.3 Unit test in `src/manifest/entry.rs` tests (next to the `SUB main` case at
      l.198): `FUNC main AS Nothing` accepted, `FUNC main AS String` rejected. — Added
      `func_entry_returning_nothing_is_accepted`; the `AS String` rejection is the
      existing `func_entry_not_returning_integer_is_error`. `cargo test --bin mfb
      manifest::entry` → `11 passed; 0 failed`.
- [x] 2.4 Spec: `src/docs/spec/tooling/01_project-manifest.md:346` and the entry
      contract in `language/08_error-model.md`. — Done, plus `architecture/02_frontend.md`
      (a third copy of the Integer-only rule, `rg -n 'must return .?Integer' src/docs`)
      and the stale uncaught-error banner (Corrections). `scripts/spec-census.sh
      --citations` → `MISS-PATH 0 / MISS-LINE 0 / MISS-SYMBOL 0`.

Acceptance: both new fixtures pass with the expected exit codes.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc 'entry-func-nothing*'` → 2 PASS (est. 1 min); `cargo test --bin mfb manifest::entry` → pass (est. 2 min).
  Result (2026-09-27): `acceptance tests passed (2 test(s) ran)`; `11 passed; 0 failed`.
Commit: —

### Phase 3 — `tools/sub-to-func/` rewriter

- [ ] 3.1 `tools/sub-to-func/sub-to-func.py`, plus `README.md` (usage, the three
      rewrites, `--check`, the prose-report). Per AGENTS.md this is a tool, not a
      `scripts/` gate, so no `scripts/README.md` entry is needed.
- [ ] 3.2 Prove it on copies: run it on scratch copies of
      `tests/syntax/functions/sub-template-valid` (template header `SUB consume OF T(value AS T)`),
      `tests/syntax/packages/package-simple` (`EXPORT SUB`, overloads) and
      `tests/rt-behavior/trap/func-bare-trap-rt` (`EXIT SUB` in a `TRAP`). Build each
      copy, then `cmp` its `.ncode` against the unmodified fixture's `.ncode`.
- [ ] 3.3 Idempotence: running it twice leaves the second run with 0 changes
      (`--check` exit 0).

Acceptance: all three rewritten copies compile, and their `.ncode` is byte-identical to the originals.
  Check: the three `cmp` commands from 3.2 → identical (est. 3 min).
Commit: —

## Validation Plan

- Tests: the two `entry-func-nothing` fixtures and the `manifest::entry` unit test.
  (`nothing-call-value-invalid` is withdrawn with Phase 1; `Nothing`-value coverage is
  bug-744's `tests/runtime/rt_nothing_func_value_position.rs` and bug-745's tests.)
- Coverage: the new fixtures own `golden/` dirs, so `test-accept.sh` compares them.
- Runtime proof: `mfb run` on `entry-func-nothing-valid` prints its line and exits 0.
- Doc sync: `01_project-manifest.md`, `08_error-model.md`, then
  `scripts/spec-census.sh --citations`. (`04_types.md` §4.6's statement that `Nothing`
  values bind, pass and collect lands with bug-744/745.)
- Final gate: plan-159 runs the full suite once, at the end of D. A runs the scoped
  checks above and `cargo test --bin mfb`.

## Open Decisions

- ~~Rule name for 0074~~ — moot: 0074 stays `TYPE_SUB_HAS_NO_VALUE`, SUB-only, and D
  retires it with `SUB`.

## Corrections

- **2026-09-27 — rt fixtures have no `run.log` golden (task 2.1).** A rt-behavior
  fixture's program output and exit code live in `golden/build.log`; an empty
  `golden/<pkg>.run` is only the marker that makes `test-accept.sh` execute the binary
  (`scripts/test-accept.sh`, the `.run` comment block). The two fixtures follow that
  convention. Their `.ast`/`.ir` goldens were adopted from the first green run (they are
  dumps, not the behaviour under test).
- **2026-09-27 — the spec's uncaught-entry-error banner was stale (found in 2.4, fixed).**
  `08_error-model.md` (rule 10 and the §8.7 table) and `mfb man errors` said stderr gets
  `Code: <err.code> Message: <err.message>`. The runtime writes `Error: <code>`, a
  newline, then the message (`ENTRY_ERROR_PREFIX` in
  `src/codegen/error/constants/error_constants.rs`; measured: `/tmp/p159/subfail` →
  `Error: 700` / `intentional failure out of main`; every rt golden agrees, e.g.
  `process/spawn-fail-trap`). Spec and man corrected to the measured shape.

- **2026-09-26 — the value-less rule for `Nothing` calls is withdrawn (owner).** The
  first draft made using the value of any `Nothing`-returning call a compile error
  (`TYPE_NOTHING_CALL_HAS_NO_VALUE`, §1 Goal, §3 item 1, Phase 1). That contradicts the
  language: `Nothing` is a valid, first-class unit type whose value `NOTHING` binds,
  passes, compares and collects (`mfb spec language types` §4.6/§4.10/§4.11,
  `mfb man types`). The owner's ruling: `LET a = NOTHING`, `LET y = f(1)` with `f … AS
  Nothing`, and `LET a = f(a)` must all work, as must collections of `Nothing`. The
  panic that motivated Phase 1 is bug-744 (fixed by lowering the call to `NOTHING`);
  collections of `Nothing` are bug-745. Phase 1 is withdrawn, the 0074 rule stays
  SUB-only until D, and the Summary's risk moves off the verifier.

## Summary

A is additive: the entry check widens and a tool is added. The `Nothing`-value
behaviour it relies on is bug-744's and bug-745's, not A's. After A, `FUNC … AS Nothing`
can do everything `SUB` can, which is the premise B–D rest on.
