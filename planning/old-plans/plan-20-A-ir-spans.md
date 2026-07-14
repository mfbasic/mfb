# plan-20-A — Spans on every IR statement/declaration node

Last updated: 2026-07-03
Effort: medium
Overall Effort (plan-20 whole): huge (>3d)
Depends on: none (first sub-plan of plan-20)
Parent: planning/plan-20-typed-ir-single-checker.md

## Goal

Give the IR the complete source-span vocabulary the relocated diagnostics (§6
census of the master plan) will need, serialized into the `.mfp`, with native
output byte-identical.

## Refinement vs. the master plan (logged fork)

The master plan said "add `loc` to every `IrValue`/`IrOp` variant". Verified
against `src/typecheck/`: **typecheck never reports an expression-specific
line.** `infer_expression` threads the enclosing statement line into every
rule (`Expression::Binary { .. }` at inference.rs:177 discards the operator's
own line and reports at the threaded `line`); `show_diagnostic` always passes
column 1. The complete line vocabulary typecheck can emit is:

- statement lines (every `Statement` variant has `line`)
- match-case lines (`MatchCase.line`)
- inline-trap lines (`Expression::Trapped.line`) — lowered to ops, so covered
  by op spans stamped from the trap line
- declaration lines (function/param/type/field/binding decls)

Therefore 20-A adds `loc` to: **every `IrOp` variant** (statement line, col 1),
**`IrMatchCase`**, and the declaration structs **`IrFunction`, `IrParam`,
`IrType`, `IrField`, `IrVariant`, `IrBinding`**. `IrValue` keeps its existing
four locs (`Call`/`CallResult`/`Binary`/`Unary` — those serve runtime
`ErrorLoc`, not diagnostics). If a later porting sub-plan finds a rule that
genuinely needs an expression-precise span, the fix is to add `loc` to that
variant then — not to skip the rule.

## Tasks

1. `src/ir/op.rs`: add `loc: IrSourceLoc` to all `IrOp` variants except `For`
   (has it). `src/ir/value.rs`: add `loc` to `IrMatchCase`.
2. `src/ir/types.rs`: add `loc` to `IrType`, `IrField`, `IrVariant`,
   `IrBinding`, `IrParam`. `src/ir/mod.rs`: add `loc` to `IrFunction`.
3. `src/ir/lower.rs`: stamp real lines — `LowerContext.current_loc` set at
   `lower_statement`/`lower_match_case`/`lower_inline_trap`/`lower_binding`
   entry (statement/case/trap/binding line, col 1); arms capture a local `loc`
   so nested blocks can't leak a stale loc into a parent op (re-set before the
   trailing condition in `DoUntil`). Declarations stamp their decl lines.
4. `src/ir/binary.rs`: encode/decode `loc` (loc-last, matching the existing
   `For`/`Call` style) for every changed struct/variant; bump
   `BINARY_REPR_VERSION` 2 → 3.
5. `src/ir/json.rs`: `-ir` dump gains `loc` on ops/cases/declarations.
6. Fix all pattern/construction sites that break (compiler-driven; the error
   list IS the site audit): `src/ir/{package,tests}.rs`, `src/ir/verify/`,
   `src/target/shared/nir/lower.rs`, `src/target/shared/runtime/usage.rs`,
   `src/binary_repr/{writer,sections}.rs`.
7. Regenerate `.ir`/`.hex`/`.mfp`/`.info` goldens via `scripts/sync-goldens.sh`.

## Acceptance

- Baseline acceptance green before starting (recorded).
- After change + sync: full acceptance green; `git diff --stat tests/` shows
  ONLY `.ir`/`.hex`/`.mfp`/`.info`/`build.log-for-those` golden classes — no
  `.nir/.nplan/.nobj/.ncode/.mir` or run-output diffs (G5 byte-identity).
- Decoded `.mfp` round-trips: existing `cargo test` IR tests pass with locs.
- Spec sync: `mfb spec package binary-representation`/`08_ir-section.md`
  updated for v3 + loc fields.

## Fixture regeneration record (executed 2026-07-03)

The v3 format invalidates every pre-built `.mfp` under `tests/*/packages/`:

- **Source-backed workers** rebuilt from `tools/thread-package-sources/` and
  copied to consumers. Three sources were stale relics that no longer
  typecheck: `fs_thread_workers` was modernized (`LET f AS File` → `RES`,
  same behavior); `thread_file_workers`/`thread_file_sink` could NOT be
  modernized (see below) and were **transcoded** instead.
- **Transcode → superseded by source reconstruction.** The initial fix
  transcoded `state_xfer_workers`/`thread_file_workers`/`thread_file_sink`
  from v2 bytes (temporary `legacy_v2` reader shim + one-shot `#[ignore]`
  test, both deleted). Because 20-B changed the byte format again — and a
  second transcode would have stamped fake `Unknown` types that 20-C's
  complete package checker would reject — all three now have REAL sources
  under `tools/thread-package-sources/` (state_xfer_workers newly written
  from its consumer's contract; the file workers modernized to the
  accept/resource-plane API). Behavior verified against the `.run` goldens:
  `thread-transfer-state-rt` prints 99, `func_thread_start_valid` matches,
  `func_thread_transfer_valid` prints 20, `thread-send-file-ownership-rt`
  prints sent/20. The one semantic concession: `acceptFile` no longer closes
  its seed-arg File (unwritable today — borrow rule); observable behavior
  unchanged.
- **native-resource-link-valid** linked the nonexistent library `"demo"`; the
  old fixture predated the LINK trailer so its consumer never dlopen'd. The
  rebuilt fixture made the consumer dlopen at startup and fail. Fixed by
  pointing the LINK at the real `sqlite3` library (`sqlite3_open`/`close`/
  `prepare_v2`/`finalize` symbols), preserving the test's purpose (resource
  type/table round-trip + a running importer) on every test box.
- **Project-built fixtures** rebuilt from their test/package projects:
  `package_comparable_types`, `package_record_comparable`, `package_import_as`,
  `package_fs_create_temp_file`, `trap_builtin_pkg`,
  `native_resource_link_valid`, `sqlite3` (bindings/sqlite3).
- **Security fixtures**: `mfp_craft.py` hand-encoded MFBR bodies updated to v3
  (version u16 + trailing `loc` on ops and function declarations); all 7
  `pkg-0N` generators re-run against the new compiler.

**Discovered pre-existing language inconsistency (not fixed here):** a worker
receiving a resource as a `thread::start` seed argument cannot be written in
today's language — every resource parameter is a borrow
(`typecheck/mod.rs:1762`), and a borrow cannot close, so
`thread_file_workers::acceptFile` (which closes its seed `File`) is
unwritable from source while `func_thread_start_valid` still exercises that
call shape against the fixture. Needs its own plan: either owned resource
params for ISOLATED entries or migrating seed-resource passing to the
transfer/accept plane.

## Commit

Single commit at the end: `feat(ir): source spans on IR ops/declarations,
Binary Representation v3 (plan-20-A)`.
