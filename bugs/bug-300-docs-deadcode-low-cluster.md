# bug-300: cross-module docs / dead-code LOW cluster (stale comments, dump-serialization loss, grammar drift, table drift)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Docs / Dead-code / Correctness (dump-only)

Status: Open
Regression Test: per-item (mostly doc/comment; serialize golden for D3)

A cluster of LOW-severity documentation, dead-code, and dump-only-serialization
residuals found across several modules during goal-06. Distinct root causes, one
document per the repo's low-cluster convention. None affects compiled program
behavior.

References:

- Found during goal-06 review of `src/target.rs`, `src/arch/ops.rs`,
  `src/ast/serialize.rs`, `src/arch/x86_64/regmodel.rs`, `src/builtins/math.rs`,
  `src/cli/build.rs`, and the spec grammar.

## Items

### E1 — `target.rs` stale doc: `-app` "rejected for non-macOS targets"
- `src/target.rs:230-232` (`target_supports_app_mode` free-function comment).
- Linux app mode exists (`NativeBuildMode::LinuxApp`, GTK4); bug-93(3) fixed the same
  claim on the trait method but missed this free-function comment.
- Fix: reword to "for targets whose backend lacks app mode".

### E2 — `arch/ops.rs` error string says "aarch64" in the arch-neutral op layer
- `src/arch/ops.rs:548` (`CodeOp::from_mnemonic`): `Err(format!("aarch64 code op
  '{other}' is not encodable"))`. This file is the neutral MIR vocabulary for all
  four backends (bug-82 moved it out of `aarch64/`); a bad mnemonic on the x86/riscv
  path misreports "aarch64".
- Fix: drop "aarch64" from the message.

### E3 — `-ast` dump silently drops LinkFunction `return_state_type`, `bind_in`, `bind_state`
- `src/ast/serialize.rs:382-451` (`impl ToAstJson for LinkFunction`).
- No keys for `return_state_type` (plan-53-A), `bind_in` (plan-50-E), or `bind_state`
  (plan-53-B), so a native `FUNC … AS RES SoundFile STATE FileInfo` with `BIND
  IN`/`BIND STATE` dumps identically to one without them; ordinary `Function::to_json`
  does emit `returnState`, so the LINK side is asymmetric. Dump-only (no compile
  impact — `.mfp` goes through `binary_repr`), but the `-ast` dump misrepresents the
  AST (bug-171-B fixed the identical class for `Function::to_json`'s `isolated`).
- Fix: emit `"returnState"`, `"bindIn"`, `"bindState"`; guard goldens (only
  LINK-with-STATE fixtures change).

### E4 — spec grammar documents the removed `"return"` ABI slot name and omits IN/OUT/INOUT directions
- `src/docs/spec/language/19_grammar.md:44,49,53` vs `src/ast/items.rs:1200-1206`
  (`parse_abi_slot_name`).
- The `nativeFree`/`abiSlot`/`abiReturn` productions still admit the literal
  `return`, but plan-50-H deleted that special case (`return` lexes as
  `Keyword::Return` and is rejected in all three positions); `abiSlot` also omits the
  `IN`/`OUT`/`INOUT` directions the parser accepts (plan-50-E). The grammar also
  requires `"(" [params] ")"` on funcDecl/subDecl though the parser accepts
  paren-less `FUNC f AS Integer` / `SUB main`.
- Fix: update the three productions to plain `ident`, add `[ "IN" | "OUT" | "INOUT"
  ]` to `abiSlot`, and note or remove the paren-less FUNC/SUB leniency.

### E5 — x86 `regmodel` header still documents r14 as the pinned zero register; `ZERO_REGISTER` is dead
- `src/arch/x86_64/regmodel.rs:24-31` (doc), `:43` (`INT_ALLOCATABLE` includes
  "r14"), `:45-49` (`ZERO_REGISTER`).
- The `INT_ALLOCATABLE` "Excludes:" comment still says r14 is the pinned zero
  register "never allocated", contradicting the pool one line below (r14 is
  allocatable, plan-34-C). `ZERO_REGISTER`'s doc says it is what the `x31` spelling
  realizes as, but select.rs maps `x31` to `abi::ZERO` (immediate-zero), never r14 —
  the const is referenced only by its own tests and a stale `abi.rs:111` comment.
- Fix: rewrite the Excludes list (r14 is allocatable; the zero token is an
  immediate), delete/re-doc `ZERO_REGISTER`, fix `abi.rs:111`.

### E6 — `math::rand` nominal return type is `Integer`, disagreeing with its `Money` overload
- `src/builtins/math.rs:88` (`call_return_type_name`) vs `:182-184` (`resolve_call`).
- `resolve_call` resolves `rand(Money, Money) → Money`, but the arg-type-independent
  `call_return_type_name(RAND)` returns `Some("Integer")` unconditionally. Shadowed
  everywhere by the arg-typed resolver (no reachable miscompile; both are i64
  scalars), so it is a table inconsistency — the one place the two return-type
  tables can disagree.
- Fix: drop `RAND` from `call_return_type_name` (arg-type-dependent, like abs/min/max
  which correctly return `None`), or document the split.

### E7 — `math::round`/`floor`/`ceil` man pages omit the accepted `Money` overload
- `src/builtins/math.rs:174,194` (accepts `Money`) vs the math man pages.
- `resolve_call` accepts `floor|ceil|round(Money) → Integer` (plan-29-G §4.7) and
  `expected_arguments` lists Money, but the `math::round` man page lists only
  Float/Fixed overloads — even though the `money` man page cross-references
  `math::round(Money)` as the dimension-exit path. The two man pages disagree.
- Fix: add the `round(Money) AS Integer` (and floor/ceil) rows to the math man pages.

### E9 — plan validation does not cross-check branch `target` fields against defined labels
- `src/target/shared/code/validation.rs:96` (`CodeFunction::validate`) / `:25`
  (`CodeInstruction::validate`).
- `CodeInstruction::validate` only checks that required *fields are present* (a branch
  has a `target`); `CodeFunction::validate` checks relocation targets resolve but never
  verifies a `Branch`/`BranchEq`/… `target` names a `Label` present in the function.
  A codegen bug emitting a branch to an undefined label passes `plan.validate()` —
  but is caught loudly downstream (the AArch64 encoder errors "branch target label
  does not resolve", `emitter.rs:1187`), so this is a redundant defense-in-depth gap,
  not a shippable-wrong-binary path.
- Fix (low priority): optionally collect `Label` names and assert every branch
  `target` resolves at plan level, to fail earlier than encode level.

### E10 — `net.write`/`net.writeText` declare a dead libc `write` import on x86_64
- `src/target/linux_x86_64/plan.rs:336-343` (`runtime_imports`, net arm) via
  `plan::net_libc_symbols` (`shared/plan/mod.rs:58`).
- On x86_64 `emit_write` is a raw `SYS_WRITE` syscall (the net write helper derives
  errno from the negated raw return, bug-109), so the libc `write` PLT symbol is never
  referenced — leaving an unreferenced `write` in the dynamic symbol table. On aarch64
  `emit_write` is a libc call, so the same shared `["write"]` list is correct there.
  Acknowledged in bug-109 ("makes the libc `write` import … dead on x86") but left; the
  file's `write_is_never_imported` test also omits `net.write`/`net.writeText`.
- Fix: filter `"write"` out of the x86 net import list; add `net.write`/`net.writeText`
  to the `write_is_never_imported` test.

### E11 — x86_64 `emit_variadic_call` does not zero AL for variadic SysV calls (latent)
- `src/target/linux_x86_64/code.rs:776-787` (`emit_variadic_call`).
- The x86_64 SysV ABI requires AL = number of vector registers used when calling a
  variadic function; `emit_variadic_call` reuses `emit_libc_call` (plain `call`) with
  no `xor eax,eax`, and its comment is copied from aarch64 (which has no AL
  convention). Latent — the only caller is `open` (no float variadic args), so glibc/
  musl never reads the XMM save area; it would bite only if a float-carrying variadic
  libc call were routed here.
- Fix: emit `xor eax,eax` before the variadic `call` on x86_64, or document that only
  non-float variadics are supported.

### E12 — `validate_capabilities` under-approximates runtime calls folded inside loop bodies
- `src/target/shared/validate.rs:344-373`
  (`collect_runtime_calls_from_ops_with_constants`, loop arms).
- For `While`/`For`/`ForEach`/`DoUntil` bodies this clones the constants map but never
  invalidates locals reassigned inside the body, so a loop-entry constant still folds a
  call (e.g. `strings.upper(s)`/`toString(s)`) used before an in-loop reassignment —
  eliding it from `runtime_calls`. Codegen does the opposite (`builder_control.rs`
  calls `clear_local_constants()` before every loop body), so codegen emits the real
  call validate believes folded. Worst case: a bypassed capability gate surfaces as a
  codegen-time error rather than a validate-time rejection — no broken binary today
  (the foldable targets are supported on all backends); latent for a future
  backend-restricted foldable call.
- Fix: mirror codegen — remove from `constants` every local assigned anywhere in the
  loop body (reuse `scan_loop_locals`) before recursing.

### E13 — `NirOp::Match` leading union-extract binds added to scope but their values never validated
- `src/target/shared/validate.rs:1008-1026` (`validate_ops`, `Match` arm).
- The loop consuming leading `Bind { value: Some(UnionExtract) }` ops inserts each
  name/type into `guard_locals` and advances `body_start` but never calls
  `validate_value` on the extract value, so those expressions escape the backstop
  (only `case.body[body_start..]` is validated). Latent — front-end match desugar
  generates these with a resolved subject local; only hand-crafted/corrupted NIR (what
  the backstop exists to catch) would slip through.
- Fix: call `validate_value` on each consumed bind's `UnionExtract` value against the
  accumulating `guard_locals` before inserting the name.

### E14 — `FunctionPlanBuilder::lower_ops` never traverses match-case guards (bug-118 residual)
- `src/target/shared/plan/function_builder.rs:136` (`NirOp::Match` arm).
- The `Match` handler lowers `value` and each `case.body` but not `case.guard`, while
  every sibling pass does (`plan/symbols.rs:569`/`:340`, `code/data_objects.rs:837`).
  A `WHEN v WHERE <expr>` guard with a call or string literal is omitted from
  `PlannedFunction.calls`/`string_literals`, under-populating the descriptive
  `.nplan`/`.nobj` model. Not a real-binary defect (the authoritative code layer walks
  guards and lays out the string pool/relocations independently); impact limited to
  descriptive-golden fidelity. bug-118 fixed this exact omission in `plan/symbols.rs`
  but overlooked `function_builder.rs`.
- Fix: add `if let Some(guard) = &case.guard { self.lower_value(guard)?; }` in the
  `Match` arm, mirroring `plan/symbols.rs:569`.

### E8 — dead final `else` arm in `build_project`
- `src/cli/build.rs:633-638`.
- The `outputs.is_empty()` block's `else` ("Validated MFBASIC project…") is
  unreachable because `validate_project_manifest` restricts `kind` to exactly
  `executable | package`. Harmless but misleading (implies a third project kind and
  a signable non-exec/non-pkg build).
- Fix: replace with `unreachable!("validate_project_manifest restricts kind to
  executable|package")`, or drop it.

## Goal

- Each stale comment / dead item / dump-serialization gap / grammar drift is
  corrected so the docs, dumps, and code agree.

### Non-goals (must NOT change)

- Any compiled-program behavior (all items are docs/dead/dump-only).
- The `.mfp` wire format (E3 is `-ast` dump only).

## Blast Radius

Each item is a single cited site across independent modules; land per item.

## Fix Design / Phases

- [ ] Phase 1: serialize golden for E3; the rest are comment/table/spec edits.
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: regenerate the E3 golden; full suite green; spec/man render checks.

## Validation Plan

- Regression: E3 `-ast` golden for a LINK-with-STATE fixture.
- Doc sync: E1/E2/E4/E5/E7 (comments, grammar, man pages).
- Full suite: acceptance + doc/spec render.

## Summary

Eight cross-module docs/dead/dump residuals; all cosmetic or dump-only, each a
localized edit. Value is keeping the docs/dumps honest before MVP.
