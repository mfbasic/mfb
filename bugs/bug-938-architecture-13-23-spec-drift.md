# bug-938: architecture spec 13–23 (NIR, ISA catalogs, MIR, math kernels, internal naming, IR JSON, type inference, escape analysis) drift from the code

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Other (SPEC BUG — documentation stale/wrong vs. correct code; one UNCLEAR item marked)

Status: Open
Regression Test: none (doc-only); `mfb spec architecture <topic>` render is the verification

Each item below quotes a claim from `src/docs/spec/architecture/<file>`, states what the code does,
and the command that measured it. The code is correct in every item (no behavior change wanted)
unless marked UNCLEAR. Correct behavior: the spec text is updated to match.

References: found in hardening batch B12 (architecture topics 13–23). Related filed drafts:
B03-spec12-int-into-float-list-literal (collections page, same root as item 22a).

## 13_native-ir.md

**13a. Native-direct list is incomplete.** Claim: the set "includes every `native_builtin_target`,
plus a fixed list: `len`, the `fs.path*` family, the numeric coercions
(`toByte`/`toFixed`/`toFloat`/`toInt`/`toString`), the `is*` predicates, and the `math.abs`/… group".
Code (`src/target/shared/runtime/usage.rs:is_native_direct_call`) also returns true for every
`codegen::registry::abi_inline_lower` member (`abi::` AbiInline) and for `toMoney` and `toScalar`.
Measured: `sed -n 6,60p src/target/shared/runtime/usage.rs`.

**13b. `mfb-nir` function objects carry an undocumented `"file"` key.** `mfb build --nir` on a
one-file project emits `{"name": "main", "visibility": "public", "kind": "func", "isolated": false,
"file": "src/main.mfb", "params": [], "returns": "Nothing", "body": …}`; the spec example omits `file`
(also on the synthetic `__mfb_init_globals_<project>`). Measured: `python3 -c "…print(f.keys())"` over
`/tmp/hardening/B12/p1/p1.nir`.

**13c. `checked` row lists `observe` as always present.** Claim: "`Checked` | `checked` | `type`,
`value`, `observe`". `src/target/shared/nir/json.rs` emits `"observe": false` only when false and
omits the key when true (the common inline-TRAP case): a built fixture
(`trap/inline-trap-fixed-comparison-promotion-rt`) dumps `"kind": "checked", "type": "Boolean",
"value": …` with no `observe`. (The IR-JSON page already says this correctly.)

## 14_aarch64-instruction-set.md

**14a. Op catalog omits 18 ops the encoder implements.** Claim: the tables are "that repertoire"
("Every op has a dedicated encoder"). Missing from both tables: `rorv_w` (`0x1AC02C00`), `lslv`
(`0x9AC02000`), `lsrv` (`0x9AC02400`), `asrv` (`0x9AC02800`), `clz` (`0xDAC01000`), `rbit`
(`0xDAC00000`), `rev_w` (`0x5AC00800`), `rev_x` (`0xDAC00C00`), `sxtw` (`0x93407C00`), `b.vs`
(`0x54000006`), `stlr_u64` (`0xC89FFC00`), `ldar_u64` (`0xC8DFFC00`), `ldar_u32` (`0x88DFFC00`),
`ldr_d` (`0xFD400000`), `str_d` (`0xFD000000`), `fmov_d_from_d` (`0x1E604000`, mentioned only in
prose), `cnt8b_v` (`0x0E205800`), `addv8b_v` (`0x0E31B800`). All 18 base words were checked against
`clang -c -target arm64-apple-macos` + `otool -t` (0 mismatches; same for the 109 documented rows).
Measured: CodeOp enum vs. the spec's backticked names (`src/arch/ops.rs`), constants from
`src/arch/aarch64/encode/emitter.rs`.

**14b. `mov` has a second encoding.** The `Mov` row gives only `orr Xd,xzr,Xm` (`0xAA0003E0`); when
either operand is `sp`/`raw_sp` the encoder emits `add Xd, Xn, #0` (`0x91000000`) because register 31
is XZR in the ORR form (`emitter.rs:emit_mov`, bug-178 B). Likewise the shifted-register ops reject an
`sp`-spelled operand (`operand.rs:shifted_reg`, bug-284 C1) — the "Operands are decoded by small
helpers" list omits `shifted_reg`.

## 15_x86_64-instruction-set.md

**15a. `RETS = [rax, rdx, rcx, rsi]` does not exist and contradicts the page's own ABI paragraph.**
Claim: "The error-`Result` convention returns its four fields in `RETS = [rax, rdx, rcx, rsi]`.
[[src/arch/x86_64/select.rs:RETS]]" and "Arguments in `rdi … r9`; return in `rax`". `grep -n RETS
src/arch/x86_64/select.rs` finds only `C_RETS = ["rax", "rdx"]`; `realize_abi_operand` maps
`(AbiConvention::Mfb, _)` — MFB arguments **and results** — onto `CALL_ARGS` (`rdi, rsi, rdx, rcx, …`
on SysV, `rcx, rdx, r8, r9, …` on Win64), exactly as the earlier "MFB's convention is aligned"
paragraph says. `rax` is the C/syscall return only. The citation `[[…select.rs:RETS]]` is dangling.

**15b. Reserved-register list is incomplete.** The register-model bullet lists the non-allocatable
GPRs and why, but omits `rbx` (realizes the worker current-thread token, `regmodel.rs:current_thread`)
and `r13` (closure-environment register, `regmodel.rs` closure env). Both are why `INT_ALLOCATABLE`
is only `r10, r11, r12, r14`.

**15c. Line-number citations.** `[[src/arch/x86_64/mod.rs:1]]`, `[[src/arch/x86_64/mod.rs:14]]`,
`[[src/arch/x86_64/select.rs:1]]`, `[[src/arch/x86_64/encode/tests.rs:1]]` cite lines, not symbols
(the citation checker resolves no symbol `14`).

(See the separate draft B12-x86-float-min-max-signed-zero-diverges for the page's "neither reachable
through `math::min`/`max`" claim, which is false.)

## 16_riscv64-instruction-set.md

**16a. Store row omits `str_u16`.** Claim (op catalog): "`str_u64` `str_u32` `str_u8` | `sd` `sw`
`sb`". The encoder also implements `str_u16` → `sh` (`src/arch/riscv64/encode/emitter.rs`,
`"str_u16" => self.emit_store(0b001, …) // sh`).

## 17_mir-instruction-set.md

**17a. `--mir` example envelope is not what the compiler writes.** Claim (example): `"entrySymbol":
"_mfb_fn_main"` and a relocation `"binding": "global"`. `mfb build --mir` writes `"entrySymbol":
"_main"` on macos-aarch64, linux-x86_64 and linux-riscv64 alike, and relocation bindings are only
`internal`, `data`, `external` (no `global` anywhere in `src/codegen/engine/mir/` or `types/`).
`mov_imm` instructions also carry a `type` field not shown in the field table. Measured:
`python3 -c "import json;d=json.load(open('p1.mir'));print(d['entrySymbol'], {r['binding'] for f in
d['functions'] for r in f['relocations']})"`.

**17b. Field-signature table omits mirror/simd ops that appear in the vocabulary.** Not listed:
`sxtw`, `adds`/`subs`, `stlr_u64`/`ldar_u64`/`ldar_u32`, `v128.cnt8b`, `v128.addv8b` (all in the
`mir_ops!` macro, `src/codegen/engine/mir/mir.rs`).

## 18_math-kernels.md

**18a. Validation section is stale about the large-argument bucket.** Claim: "The tooling buckets
trig vectors above `2^20*pi/2` as "extended … out of scope" (`tools/math-kernels/README.md` §verify
scope, `runtime_ulp.py:_is_primary`)". Both sources now say the opposite: README §verify scope — "In
`runtime_ulp.py` … they are **gated like any other vector** since bug-618"; `runtime_ulp.py` —
"bug-618: the extended bucket counts too … a miss there is a defect" (`ok = … and e[4] == e[0]`).

(Accuracy claims themselves held in this batch: 362 Float cases incl. huge/subnormal/near-pole trig,
exp/log near 0/1/overflow/underflow, pow extremes — all ≤1 ULP of macOS libm except `tan`, whose
2-ULP-vs-libm points were within 0.4 ULP of mpmath truth; list ≡ scalar bit-for-bit; riscv64 (hardware
and qemu RVV on/off) bit-identical to aarch64; 54 `fmod` cases bit-exact; 459 Fixed trig/inverse-trig
cases within 1 unit of truth.)

## 19_internal-naming.md

**19a. Line-number citations point at unrelated code.** `[[src/lexer.rs:262]]` ("`#` falls through to
the catch-all arm") is the `:`/`::`/`:=` arm; `[[src/lexer.rs:752]]` ("the rewrite happens after
keyword classification") is the numeric-exponent scanner. The `internalize` call is at
`src/lexer.rs` ~982 (`value = crate::internal_name::internalize(&value)`). Measured: `sed -n
255,270p` / `745,760p src/lexer.rs`, `grep -n internalize src/lexer.rs`.

**19b. `display_name` user cited at the wrong function.** Claim: "The monomorphizer uses it when
reporting errors … [[src/monomorph/helpers.rs:mangle_name]]". `mangle_name` never calls
`display_name`; the calls are in `src/monomorph/lower.rs` (lines ~830, ~1119). Measured:
`grep -rn display_name src/monomorph/`.

**19c. "`#` … is reserved exclusively for this purpose".** The sigil also prefixes file-scoped
PRIVATE mangles (`#<hash>$name`, `internal_name::mangle_private`, demangled by `display_name`), and the
`$` domain list ("lambda names … and the monomorphizer's mangling") omits the hidden default-argument
functions `$default$<function>$<index>` (`internal_name::hidden_default_function_name`) and the
lowering temporaries (`$for_iter0`, `$res_tmpN`, `$trap_valN`, visible in `--nir`). (A real sigil leak
in diagnostics is filed separately: B12-internal-sigil-name-leaks-in-collections-arity-diagnostic.)

## 20_ir-json-artifact.md

**20a. Eight value kinds carry an undocumented `type` key.** The Values table lists `call` = `target,
args`, `callResult` = `target, args`, `resultIsOk`/`resultValue`/`resultError` = `value`,
`memberAccess` = `target, member`, `binary` = `op, left, right`, `unary` = `op, operand`.
`src/ir/json.rs` emits a `"type"` immediately after `"kind"` on all eight (`resultIsOk` always
`"Boolean"`, `resultError` always `"Error"`). Measured with a schema checker written from the page
(`/tmp/hardening/B12/irv/validate.py`) over all 1036 committed `tests/**/golden/*.ir` plus fresh
`mfb build --ir` output: the only deviations are these eight `extra-keys:<kind>:['type']` classes
(130 673 `binary`, 127 475 `call`, 38 252 `memberAccess`, …). Header, `bindings`-omitted-when-empty,
`entry: null` for a package, op `"op"`→`"line"` ordering, match-case/pattern shapes, `capture.byRef`
and `checked.observe` all conformed.

(Unrelated hygiene seen during that sweep: 16 zero-byte golden files literally named `.ir`/`.ast`, e.g.
`tests/rt-behavior/types/bug105_grouped_type_names/golden/.ir` — stray outputs of an earlier harness run.)

## 22_type-inference.md

**22a. The recommended annotation is rejected.** Claim (Bare List-Literal Synthesis): "To force an
element type, annotate the binding (`LET xs AS List OF Float = [1, 2]`), which takes the expected-type
path." `mfb build` rejects it: two `TYPE_LIST_ELEMENT_MISMATCH` "List element has type Integer, expected
Float." — consistent with the same page's coercion table, which has no Integer-literal→Float arm. Use
e.g. `LET xs AS List OF Fixed = [1, 2]` (accepted, measured `len` = 3 for `[1, 2.5, -3]`) or
`[1.0, 2.0]`.

**22b. "`=` / `<>` accept any two numerics with no compatibility requirement".** `Money` is in
`is_numeric`, yet `m = f`, `m = x`, `m <> i`, `m < f`, `m >= x` (Money vs Float/Fixed/Integer) are all
`TYPE_MONEY_OPERATION_INVALID` ("Compare a Money only with a Money"). The non-Money mixes (Fixed=Float,
Byte=Integer, Integer<>Float, Fixed>Byte) are accepted as stated. The page should carve out Money.

**22c (UNCLEAR). "`expression_compatible` … is the assignability check used at every typed slot (…
call arguments …)".** For a user function it is: `f(3)` into `b AS Byte` compiles and prints `4`. For
an overloaded builtin it is not: `toString(1.5, 3)`, `toString(f, 3)`, `toString(x, 3)` are all
`TYPE_CALL_ARGUMENT_MISMATCH` (`(Float, Integer)`), and `mfb man general toString` documents
"precision is a Byte, so an Integer literal needs toByte around it". Either the inference page should
say the small-int→Byte coercion does not apply to registry overload selection, or overload selection
should apply it (owner decision; `toString(x, 2)` is the natural spelling every user tries first).

**Owner decision (2026-10-05): apply the rule to built-ins (CODE change).** Registry overload
selection uses the same small-Integer-literal → `Byte` assignability as user calls
(`expression_compatible`), so `toString(1.5, 3)`, `toString(f, 3)` and `toString(x, 3)` compile.
The `mfb man general toString` advice ("an Integer literal needs toByte around it") is removed,
along with every man example that wraps a literal precision in `toByte`. A non-literal
`Integer` argument still needs `toByte`.

## 23_escape-analysis.md

**23a. Names a function that does not exist.** Claim: "after mapping qualified `collections.*` names
back to the bare op via `native_member_bare`" with citation `[[src/codegen/registry/mod.rs:
native_bare_target]]`. No `native_member_bare` exists in the tree (`grep -rn "fn native_member_bare"
src` → nothing); `is_insertion_builtin` (`src/ir/resource_escape.rs`) maps through
`crate::codegen::builtins::native_builtin_target` (`src/codegen/builtins/mod.rs`), not
`registry::native_bare_target`.

(Behavioral escape-analysis gap filed separately: B12-resource-floated-through-take-drop-flatten-closed-early.)

## Goal

- Every item above is reconciled in its spec file (22c's owner decision is recorded above).

### Non-goals (must NOT change)

- No code change for items 13a–23a; the measured behavior is correct.
