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
