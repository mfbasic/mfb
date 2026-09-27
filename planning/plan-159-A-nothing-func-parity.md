# plan-159-A: `FUNC … AS Nothing` parity — the value-less rule, the entry point, the rewriter

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
a few rules that apply only to it. This letter moves the one rule worth keeping onto
`FUNC … AS Nothing`, and it removes the one place where `FUNC … AS Nothing` is weaker
than `SUB`. After that, `SUB` has no remaining benefit, and B–D can remove it without
losing anything.

> **OWNER DECISION, 2026-09-26 (bug-744). Phase 1's diagnostic is DROPPED. Do not
> implement `TYPE_NOTHING_CALL_HAS_NO_VALUE`.** `Nothing` is a first-class type. A call
> whose callee returns `Nothing` is an ordinary value expression that yields `NOTHING`:
> `LET a = NOTHING`, `LET y = f(1)` and `LET b = same(a)` are all legal. The panic this
> plan describes in §2 is fixed in codegen by
> `bugs/completed/bug-744-nothing-func-call-in-value-position-panics-codegen.md`
> (`lower_value` materializes `NOTHING` for a used `Nothing` call). That fix also covers
> `LET v = io::print("x")`, a call through a `FUNC(...) AS Nothing` value, and inline
> `TRAP` in value position. The sections below are amended to match.
> Struck-through text records what was planned.

**The single outcome for A.** `FUNC main() AS Nothing` is a valid executable entry.
~~Using the value of *any* call whose callee returns `Nothing` in value position
(`LET v = f()`, `g(f())`) is a compile error, not a compiler panic.~~ Using the value of
any call whose callee returns `Nothing` yields `NOTHING` (bug-744). The existing
`SUB`-only rule `TYPE_SUB_HAS_NO_VALUE` (`2-203-0074`) stays while `SUB` exists and is
retired with it in D. After the migration no `FUNC … AS Nothing` call is rejected for
being used as a value.

The letters, in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | ~~value-less rule generalized to `Nothing`~~ (dropped, owner decision / bug-744); `FUNC main AS Nothing` entry; the `tools/sub-to-func/` rewriter | medium |
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

| Must be true | Command | Status (2026-09-26) |
|---|---|---|
| No other plan-159 exists | `find . /private/tmp -name 'plan-159*' -not -path './target/*'` → only these four files | MET |
| `main` builds | `cargo build --release` → exit 0 | MET (`target/release/mfb` built 2026-09-26 21:16) |

Everything below assumes both conditions hold.

> **NOTE: the Status column is a snapshot. The Command column is the truth.** Re-run
> each command before you start, and again before you decide to stop. If you stop,
> report the status of every prerequisite.

## 1. Goal

- `FUNC main() AS Nothing` and `FUNC main(args AS List OF String) AS Nothing` are
  valid executable entries. On success they behave exactly like `SUB main` (exit 0).
- ~~A call whose callee's declared return type is `Nothing`, used in value position, is
  rejected with the existing code `2-203-0074`. The rule is renamed and reworded for
  `Nothing`. That covers user `FUNC`s, `SUB`s and built-ins alike.~~ **Dropped (owner
  decision 2026-09-26, bug-744):** a `Nothing` call in value position yields `NOTHING`.
  Only a direct `SUB` call is rejected (`TYPE_SUB_HAS_NO_VALUE`, unchanged) until D
  removes `SUB`. Statement-position calls stay legal, and so does inline `TRAP` on them.
- `tools/sub-to-func/` exists and mechanically rewrites a `.mfb` file from `SUB` to
  `FUNC … AS Nothing`. It is proven on the fixtures listed in Phase 3.

### Non-goals (explicit constraints)

- `SUB` still parses and compiles unchanged after A. Nothing is removed until D.
- No codegen change. A touches only a verifier rule, the manifest entry check and a
  tool, so the `.ncode`/`.ncodesum` goldens must stay byte-identical.
- The error code `2-203-0074` keeps its number, name and message through A. It is
  `SUB`-only, and D retires it with `SUB`.
- A function *value* of type `FUNC(...) AS Nothing` (a callback such as `forEach(xs, show)`)
  is unaffected. The rule applies only to a *call's result*.

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

The `io::print` equivalent (`LET v = io::print("x")`) panics identically. This is an
**untracked compiler crash**: bug-721 is an unrelated, completed Windows arena bug whose
name only appears in the panic message. It exists at HEAD today, independent of this plan.

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
- **UNVERIFIED, task 1.2:** whether `self.functions` in `ir::verify` holds built-in
  signatures. The `io::print` panic shows the check does not currently cover them, and
  the fix has to.

## 3. Design Overview

Three independent pieces, each landable alone. None of them changes codegen, so the
**correctness gate is byte-identity of `.ncode`/`.ncodesum`** plus the new fixtures.
A `.ncodesum` diff here is a bug to root-cause (`objdump` one fixture). It never means
the design is dead.

1. ~~**Generalize the value-less rule**~~ **DROPPED (owner decision 2026-09-26,
   bug-744: `Nothing` is a first-class type, so its value is usable).** The text below
   is kept only as a record. ~~Generalize the value-less rule from `sig.kind == "sub"`
   to "the callee's return type is `Nothing`", for both user and built-in callees.~~ Rename
   `TYPE_SUB_HAS_NO_VALUE` → `TYPE_NOTHING_CALL_HAS_NO_VALUE`, keeping code
   `2-203-0074`. Message: `a call to a FUNC … AS Nothing produces no value and cannot
   be used in value position`. This also fixes the panic. **Correctness risk
   concentrates here.** The rule must not fire on statement-position calls, on the
   operand of an inline `TRAP` in statement position, or on a function *reference*.
   Rename the `allow_sub_call` cell to `allow_value_less_call`.
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
- ~~*Drop the value-less rule and let `LET v = f()` bind a `Nothing`.* Today that
  panics codegen. Making it legal is new codegen work for a value nobody can use.~~
  **Adopted instead, by owner decision (2026-09-26).** `Nothing` is a first-class type
  and its value is usable. bug-744 did the codegen work.
- *Rewrite with the compiler's AST serializer.* It would reformat whole files and
  churn every golden. The line rewrite touches only the SUB lines.

## Phases

> Tick `- [x]` in the same commit as the work. `- [~]` = partial (say what remains).
> Moot = `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — The value-less rule covers every `Nothing` callee (fixes the panic)

> **DROPPED by owner decision, 2026-09-26. Do not implement.** The panic is fixed by
> bug-744 in codegen: a `Nothing` call's value is `NOTHING`
> (`tests/runtime/rt_nothing_func_value_position.rs`, one test per value position).
> Two parts still apply to `SUB` removal and move to D. First, `TYPE_SUB_HAS_NO_VALUE`
> and `sub-value-less-invalid`'s value-position cases are deleted with `SUB`, because a
> migrated `FUNC … AS Nothing` call in value position is legal. Second, the §2 finding
> that `self.functions` in `ir::verify` does not cover built-ins no longer matters. No
> rule needs it. The boxes below are struck, not done.

- [x] ~~1.1 RED first: add `tests/syntax/functions/nothing-call-value-invalid/`. It
      contains a `FUNC f() AS Nothing` bound with `LET`, nested as an argument, and
      `LET v = io::print("x")`, plus a statement-position call and a statement-position
      inline-`TRAP` call that must stay clean. Its `golden/build.log` must carry three
      `2-203-0074` diagnostics. Confirm that before the fix the build panics (the
      repro in §2).~~ — moot: dropped by owner decision (bug-744)
- [x] ~~1.2 Read `ir::verify`'s signature table (`src/ir/verify/mod.rs`, the `functions`
      map and its `kind` doc at l.684) and establish whether built-ins are in it. If
      they are not, resolve the built-in return type through the same oracle
      `check_call_argument_types` uses. Record the finding under §2 Verified properties.~~ — moot: dropped by owner decision (bug-744)
- [x] ~~1.3 `src/ir/verify/values.rs`: replace `sig.kind == "sub"` with a
      return-type-is-`Nothing` test covering user and built-in callees. Rename the
      `allow_sub_call` cell (`mod.rs:785`, `:1023`, `ops.rs:114,117,433,435`,
      `values.rs:33-70`) to `allow_value_less_call`.~~ — moot: dropped by owner decision (bug-744)
- [x] ~~1.4 `src/rules/table.rs:977`: rename to `TYPE_NOTHING_CALL_HAS_NO_VALUE` with the
      new message, then update every reference:
      `rg -n 'TYPE_SUB_HAS_NO_VALUE' src tests` → each hit updated.~~ — moot: dropped by owner decision (bug-744)
- [x] ~~1.5 `tests/syntax/functions/sub-value-less-invalid/golden/build.log`: the rule
      name in this golden changes. The four questions: (1) it pins `SUB` value-less
      rules; (2) a `SUB` call in value position is an error; (3) nothing else depends
      on the name (grep in 1.4); (4) proof: this plan renames the rule and the SUB
      case still errors under the same code. Update only the renamed text.~~ — moot: dropped by owner decision (bug-744)
- [x] ~~1.6 Spec: `src/docs/spec/diagnostics/01_rule-codes.md` (the 0074 row) and
      `src/docs/spec/language/07_subs.md` / `06_functions.md`: state that a `Nothing`
      call has no value. Cite `[[src/ir/verify/values.rs:check_value_depth]]`.~~ — moot: dropped by owner decision (bug-744)

~~Acceptance: the RED fixture goes green with three `2-203-0074` diagnostics and no panic,
and no `.ncodesum` moves.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc nothing-call-value-invalid sub-value-less-invalid` → both PASS (est. 1 min);
  `bash scripts/artifact-gate.sh target/release/mfb strings` → 0 diffs (est. 1 min; one unaffected fixture proves the blast radius, per `.ai/testing-gates.md`).~~
Commit: — (moot: dropped by owner decision; bug-744 fixed the panic in codegen)

### Phase 2 — `FUNC main … AS Nothing` is a valid entry

- [ ] 2.1 RED: add `tests/rt-behavior/functions/entry-func-nothing-valid/` with
      `FUNC main() AS Nothing` printing one line, and golden `run.log`/exit 0. Add
      `tests/rt-behavior/functions/entry-func-nothing-fail-rt/`, where `main` does
      `FAIL`, whose golden must match the exit code/stderr `SUB main` gives for the same
      `FAIL`. Take that expectation from a sibling `SUB main` fixture, not from the new
      code.
- [ ] 2.2 `src/manifest/entry.rs`: accept `Func` whose `returns` is `Integer` **or**
      `Nothing`. Update the message to `must return Integer or Nothing`. Confirm that
      the runtime entry wrapper keys off the return type, not the kind, by reading the
      entry lowering the `SUB` case uses. Record it in Verified properties.
- [ ] 2.3 Unit test in `src/manifest/entry.rs` tests (next to the `SUB main` case at
      l.198): `FUNC main AS Nothing` accepted, `FUNC main AS String` rejected.
- [ ] 2.4 Spec: `src/docs/spec/tooling/01_project-manifest.md:346` and the entry
      contract in `language/08_error-model.md`.

Acceptance: both new fixtures pass with the expected exit codes.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/p159-acc 'entry-func-nothing*'` → 2 PASS (est. 1 min); `cargo test --bin mfb manifest::entry` → pass (est. 2 min).
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

- Tests: ~~`nothing-call-value-invalid` (negative, 3 sites + 2 clean)~~ (dropped, bug-744), the two
  `entry-func-nothing` fixtures, and the `manifest::entry` unit test.
- Coverage: the new fixtures own `golden/` dirs, so `test-accept.sh` compares them.
- Runtime proof: `mfb run` on `entry-func-nothing-valid` prints its line and exits 0.
- Doc sync: `01_rule-codes.md`, `07_subs.md`, `06_functions.md`,
  `01_project-manifest.md`, `08_error-model.md`, then `scripts/spec-census.sh --citations`.
- Final gate: plan-159 runs the full suite once, at the end of D. A runs the scoped
  checks above and `cargo test --bin mfb` (est. 6 min, because the rule rename touches
  inline verifier tests).

## Open Decisions

- ~~Rule name for 0074~~ — moot. The generalized rule is dropped (owner decision,
  bug-744), and `TYPE_SUB_HAS_NO_VALUE` stays accurately named while `SUB` exists.

## Corrections

- 2026-09-26 (bug-744, owner decision): Phase 1's diagnostic is dropped. A `Nothing`
  call in value position yields `NOTHING`, and bug-744 fixed the §2 panic in codegen.
  Its runtime tests cover `LET`, `LET AS Nothing`, `MUT`, argument, operand, `RETURN`,
  record field, lambda, `FUNC` value, `SUB`-as-value, a `Nothing` built-in, inline
  `TRAP`, and `LET b = same(a)`. For D: `2-203-0074` does not stay live. It is retired
  with `SUB` (plan-159-D §"`2-203-0074`" amended to match), and the D spec rewrite
  must not describe `Nothing` calls as value-less.

## Summary

The only risk in A is the verifier change (a false positive in statement position
would break real programs). Everything else is additive. After A, `FUNC … AS Nothing`
can do everything `SUB` can, which is the premise B–D rest on.
