# bug-917: architecture spec topics 02/03/04/05/06/08/12 drift from the implementation (glob base, pin-enforcement site, `.ir` field claims, breadth-limit example, removed seam accessors)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec drift)

Status: Open
Regression Test: none (spec-only; `mfb spec architecture <topic>` render is the verification)

A batch of claims in `src/docs/spec/architecture/` that the code or a run
contradicts. Each item: the claim, the evidence, the class. All runs used
`/tmp/hardening/mfb` on macOS AArch64.

References:

- `./mfb spec architecture frontend|packages|ir|binary-representation|native|artifacts|monomorphization`
- Found during hardening batch B11.

## Items

### 1. SPEC — `02_frontend.md` "Source-File Selection": glob base

Claim: "keeps each file whose **project-relative** path satisfies the entry's `include`/`exclude` glob patterns."
Evidence: `sources: [{ "root": "src", "exclude": ["src/skip/**"] }]` still parsed
`src/skip/bad.mfb` (`MFB_PARSE_UNEXPECTED_STATEMENT` at `./src/skip/bad.mfb:1`);
`"exclude": ["skip/**"]` excluded it (/tmp/hardening/B11/mf). `./mfb spec tooling source-selection`
step 4 correctly says "root-relative path". Fix: "root-relative".

### 2. SPEC — `03_packages.md`: where `pin` is enforced, and the message

Claim: "The `pin` flag is enforced separately, during the build's binary-representation merge … (`package \`<name>\` is pinned to version <v>, but installed package is version <w>`) … [[src/manifest/package.rs:installed_package_files]]".
Evidence: installing a `0.2.0` `pA.mfp` under a `pin: true, version: "0.1.0"` entry (/tmp/hardening/B11/c1) gives
`error[6-605-0012 PACKAGE_PIN_MISMATCH]: … package `pA` is pinned to version 0.1.0, but the installed package is version 0.2.0; refusing to build`.
The check lives in `src/cli/build/packages.rs:verify_and_report_packages` (its own bug-491 comment: every caller of `installed_package_files` discards that error). Fix: cite `verify_and_report_packages`, name the rule `PACKAGE_PIN_MISMATCH`, quote the real text, and say it runs at dependency verification (flows step 2), not at merge.

### 3. SPEC — `03_packages.md` "Installing Packages": entry fields

Claim: the entry written to `project.json` "includes: name, version, pin, source".
Evidence: `mfb pkg add file:///tmp/hardening/B11/pA/pA.mfp` writes `name, ident, version, pin, source, direct, requiredBy`. List is incomplete (closure section mentions `direct`/`requiredBy` later; `ident` is never mentioned).

### 4. SPEC — `05_binary-representation.md` "Error Source Locations": `.ir` JSON contents

Claim: "The `loc`, result-type, and `explicit_type` fields are not serialized to the `.ir` JSON debug dump".
Evidence: `mfb build --ir` (/tmp/hardening/B11/irp) emits result types on value nodes —
`{"kind": "binary", "type": "Integer", "op": "+", …}`, `{"kind": "callResult", "type": "Integer", "target": "mayFail", …}`, `{"kind": "call", "type": "Nothing", "target": "io.print", …}` — and a `"line"` on every op. `src/ir/json.rs` writes `"type"` for `call`, `callResult`, `memberAccess`, `binary`, `unary`, … Fix: only `column` and `explicit_type` are omitted. (Cross-topic: `20_ir-json-artifact.md`'s Values table lists `call`/`callResult` payload as `target`, `args` and `binary`/`unary`/`memberAccess` without `type` — same drift, other batch's topic.)

### 5. SPEC — `12_monomorphization.md`: the breadth-budget example

Claim: "a generic that recurses through ≥2 type-widening self-calls (e.g. `recurse<T>` → `recurse<List OF T>` + `recurse<Set OF T>`) … the breadth budget rejects it with `TYPE_INSTANTIATION_BUDGET_EXCEEDED`."
Evidence: exactly that program (`RETURN recurse(a, n - 1) + recurse(s, n - 1)` with `a AS List OF T`, `s AS Set OF T`) reports
`error[2-203-0102 TYPE_INSTANTIATION_TOO_DEEP]: … exceeds the 256 level limit.` Expansion is depth-first, so the first child chain reaches 256 frames and latches before the tree is wide. The budget is reachable only by a wide *non-recursive* program: 66 records × `pf OF A, B` called at all 66² pairs → `error[2-203-0135 TYPE_INSTANTIATION_BUDGET_EXCEEDED] … 4096-instantiation budget` (/tmp/hardening/B11/budget). The diagnostic's own hint ("This usually means a generic recurses through two or more type-widening self-calls") is misleading for the same reason. Fix the example (and the hint).

### 6. SPEC — `12_monomorphization.md` "Symbol scheme": undocumented `$N` suffix

Claim: "The pass uses one string form for every generated symbol — a base name followed by `$`-joined, sanitized type tokens."
Evidence: `src/monomorph/lower.rs:unique_concrete_symbol` appends `$2`, `$3`, … when two distinct keys sanitize to one symbol (bug-226/bug-400 guard; unit test `instantiate_type_disambiguates_mangle_colliding_arguments`). The spec should name this suffix as part of the scheme.

### 7. SPEC — `06_native.md` "The CodegenPlatform Seam": removed accessors

Claim: the seam carries "ABI struct layouts queried as scalar accessors", tabulating `termios_size`, `termios_lflag_offset`, … `termios_vtime_index`, and "`stat_mode_offset` gives the byte offset of `st_mode`".
Evidence: `grep -rn termios_size src/` matches only the spec. Raw mode is now the intent-level `CodegenPlatform::emit_apply_raw_mode` (`src/codegen/engine/types/types.rs`) with the constants inlined per OS (`src/target/macos_aarch64/code.rs`, `src/target/linux_common/code.rs`); `stat_mode_offset` is a `LinuxArch` method (`src/target/linux_common/code.rs`, test `stat_mode_offset_stays_per_arch` says "the CodegenPlatform accessor was folded into `emit_stat_is_kind` in 47-E") and macOS inlines `+ 4` in `emit_stat_is_kind`. The numeric values in the table are still correct (checked against both implementations). Fix: describe the intent-level methods; keep the values.

### 8. UNCLEAR — `04_ir.md` §6: `CASE Ok`/`CASE Error` lowering

Claim: "For a `Result` scrutinee a `CASE Ok`/`CASE Error` union pattern lowers to a `Value` of the boolean constant…".
Evidence: in user code `MATCH mayFail(2)` / `CASE Ok(v)` is rejected with `error[2-203-0071 TYPE_RESULT_NOT_MATCHABLE]: Ok and Error are not matchable as Result members in user code`. If the lowering path exists only for compiler-injected source, §6 should say so; otherwise it is dead and should be removed.

**Coordinator verification (2026-10-05, `/tmp/hardening/mfb-head`, HEAD 28a15a153).**
The owner's model holds: `Result` and `Ok` never reach user code, and `Error` is the only
user-visible member. `LET x AS Result OF Integer = …` is `TYPE_RESULT_NOT_USER_VISIBLE`,
`Ok(1)` is `SYMBOL_UNKNOWN_IDENTIFIER`, `MATCH mayFail(2)` / `CASE Ok(v)` is
`TYPE_RESULT_NOT_MATCHABLE`, and `typeName(mayFail(1))` is `Integer`. The `ResultOf`
branch in `src/ir/lower.rs` (`match_ok` flag, inside `HirStatement::Match` lowering) is
marked `coverage:off … unreachable from valid source; kept for plan-20 total lowering`.
**Resolution:** that branch is dead code. Delete it, and delete the §6 sentence that
describes it, instead of documenting it. Making `Result`/`Ok` ordinary user-declarable
names is **bug-945**. (Side note: `error(code, msg)` also yields a
plain `Error` value outside `TRAP`, as in `LET e = error(7, "x")` / `FAIL error(…)`,
with `typeName(e)` = `Error`. That is the intended way to raise.)

### 9. UNCLEAR — `08_artifacts.md`: `.mir` "Target-neutral MIR dump"

Evidence: `mfb build --mir` for macos-aarch64 vs `--target linux-x86_64` differs from line 16 on: `"dst": "arena_base", "src": "sp"` vs `"dst": "r15", "src": "rsp"`. The dump is post-register-assignment and names target registers (and role names on one target, physical names on the other). Either the description or the dump should change.

**Coordinator verification (2026-10-05).** Inside the compiler, operands are typed
(`src/codegen/engine/operand/operand.rs:Operand`: `VReg`, `Phys{class,index,name}`,
`Abi{convention,role,index}`, `Imm`, plus a `Raw(Box<str>)` escape hatch still used in
10 files). The `-mir` capture (`src/codegen/engine/regalloc/builder_registers.rs`,
`mir::capture_function`) runs before register allocation. Code lowered from MFBASIC
source is neutral there: the user `main` in a hello-world dump uses only virtual
registers (`%vN`), the neutral ABI tokens (`%argMFB0`, `%retMFB0`, `%argC0`, …), `sp`
and `arena_base`. The target-specific names all come from the **runtime helper
functions** in the same dump (`runtime.arena_alloc`, `runtime.io.print`, …). Those are
hand-written per backend with physical registers: `x8/x9/…` on macOS AArch64, `r10/r11/…`
on x86-64, `t3/s1/…` on riscv64. `program.entry` is 174 physical + 18 `arena_base`
operands, `main` is 6 vreg + 13 ABI + `sp`. So the dump is target-neutral for program
code, not for the runtime helpers it also contains. **Split out (owner, 2026-10-05):** the runtime-helper half is now **bug-930** (decision
pending), and removing `Operand::Raw` is **bug-946**. Nothing for item 9 remains in this doc.

### 10. UNCLEAR — `TYPE_OVERLOAD_AMBIGUOUS` headline for a parameter ambiguity

`12_monomorphization.md` specifies `f([])` against `f(List OF Integer)`/`f(List OF String)` as `TYPE_OVERLOAD_AMBIGUOUS`; observed (`pA::show([])`) the headline reads "return-type overload cannot be resolved without an expected type" although no return-type set is involved (detail text is right: "matches 2 imported overloads; annotate the argument types").

**Coordinator verification (2026-10-05).** The owner's model is already the behavior
when there is one candidate: `FUNC f(xs AS List OF Integer)` + `f([])` builds and prints
`0`. `[]` fits the parameter like `LET x AS List OF String = []`. The diagnostic
only appears when `[]` fits **two** overloads (`List OF Integer` and `List OF String`),
which is a genuine ambiguity. Imported overloads report it under the misleading
`TYPE_OVERLOAD_AMBIGUOUS` headline ("return-type overload…"). Local overloads report it
as bug-836's "Callable `f` is not a top-level function". **Owner decision (2026-10-05):** when `[]` fits more than one overload it is
ambiguous, and the compiler reports the ambiguity error. There is no tie-break. The fix
gives that error an accurate headline: not "return-type overload cannot be resolved",
either by rewording `TYPE_OVERLOAD_AMBIGUOUS`'s headline to cover argument ambiguity or by
giving argument ambiguity its own rule (registry updated). The detail tells the user the
correct form: bind the empty list to a typed variable and pass that
(`LET e AS List OF String = []` then `f(e)`). The local-overload variant of the same call
must report this ambiguity too, not bug-836's "not a top-level function".

### 11. Minor — `04_ir.md` citation `[[src/ir/mod.rs:IrDocDecl]]`

`IrDocDecl` is defined in `src/ir/docs.rs` (`pub(crate) struct IrDocDecl`); `src/ir/mod.rs` only re-exports it. Cite `src/ir/docs.rs:IrDocDecl`.
