# bug-402: dead code with forbidden "later phase" `#[allow(dead_code)]` justifications (goal-07 batch)

Last updated: 2026-08-01
Effort: small (<1h)
Severity: LOW
Class: Dead-code

Status: FIXED (see STATUS block below)
Regression Test: none (removal is validated by the compiler: the fields/items
below have no readers, so deleting them must still build). Codegen-shifting
removals (items 6, 8) are guarded by the byte-identity `.ncodesum` goldens
(`tests/byte-identity/math`, `tests/syntax/app/macos-app-mode-*`), regenerated
here with an inspected before/after `.ncode` diff proving the delta is only the
removed instruction/data object.

A batch of dead production items surfaced during the goal-07 full-source review.
Each is written but never read, and (where annotated) carries an
`#[allow(dead_code)]` whose justification is the "consumed by a later phase" form
that AGENTS.md explicitly forbids ("never 'consumed by a later phase'. Else
delete."). Batched per the goal's same-class rule; distinct-root-cause defects stay
in their own documents.

References: found during goal-07; sites listed below.

## Items

### (1) `src/resolver/mod.rs:179-183` — `LinkFnSig.return_type` / `return_resource` unused
Both fields are written in `collect_top_level_symbols` (`mod.rs:248-249`) but never
read anywhere in the resolver — the only `LinkFnSig` reads are `params`,
`param_resource`, and `line` (`resolution.rs:586-601`). They carry
`#[allow(dead_code)]` with the rationale "Consumed by later native-resource phases
(producer typing); recorded now." — the forbidden "later phase" justification.
- Evidence: `grep -rn "\.return_resource\|\.return_type" src/resolver/` shows only
  the write sites, no reads.
- Fix: delete both fields (and their writes), or — if a concrete near-term consumer
  exists — wire it and replace the justification with a load-bearing reason.

### (2) `src/os/windows/mod.rs:14` — stale module-wide `#![allow(dead_code)]`
The module doc (lines 6-14) claims the writer's "public surface is unreferenced by
non-test code — hence the module-scoped `dead_code` allow below" and that
"plan-47-D removes it". Both are now false: `write_native_object_plan`,
`validate_native_object_plan`, and `write_linked_executable` are called from the
production build path (`src/target/win_x86_64/mod.rs:271,281,331`; plan-47/66
landed). The `#![allow(dead_code)]` at line 14 is a **file-level allow** (the class
goal-07 flags) that now blanket-suppresses dead-code detection for the entire
`windows` module (mod.rs, object.rs, link/*), masking any genuinely dead helper
added later.
- Fix: remove the module-wide `#![allow(dead_code)]` and the stale doc paragraph;
  add targeted `#[allow]` only if a specific still-dead item remains (with a
  load-bearing reason), else let the compiler flag it.

### (3) `src/os/linux/appimage/squashfs/mod.rs:382` — vacuous, unreachable guard
`if ref_offset(inode_at[0]) as usize > METADATA_BLOCK { ... }` intends to reject a
root-inode whose in-block offset exceeds the 8192-byte metadata block, but
`ref_offset(x)` returns `(x % METADATA_BLOCK) as u16` (`mod.rs:234-235`), always
0..=8191 < `METADATA_BLOCK` (8192). The condition can never be true — dead
defensive code with a misleading error string that reads as a bounds check but
validates nothing.
- Fix: remove the vacuous `if` (the `% METADATA_BLOCK` already bounds the value),
  or, if a real invariant was intended, check the pre-modulo `stream_offset`.

### (4) `src/syntaxcheck/mod.rs:1332-1335,1353` — dead scaffolding in `check_function`
`let _is_resource = self.is_resource_type(&param_type);` (`mod.rs:1353`) computes a
side-effect-free value and immediately discards it, and the preceding
`if param.default.is_some() { seen_default = true; } else if seen_default { }`
(`mod.rs:1332-1335`) has an empty `else if` body. Both are leftover scaffolding from
when the non-default-after-default and resource-param rules lived in syntaxcheck
(relocated to `ir::verify` per plan-20). Neither has any effect.
- Fix: delete the discarded `_is_resource` binding and the empty `else if` branch
  (or restore the intended check if one is still wanted — but the rule now lives in
  ir::verify).

### (5) `src/target/shared/code/builder_values.rs:1354-1361` — unreachable else field-copy loop
In the `NirValue::UnionWrap` arm, every non-resource (data) variant returns early at
`builder_values.rs:1285` via `emit_wrap_record_in_union`. Past that point
`is_resource_variant` is always `true`, so the guard at `:1344`
(`if is_resource_variant || self.record_has_inline_data(member_type)`) is
unconditionally true and the `else` field-copy loop at `:1354-1361` (plus the
`record_has_inline_data` disjunct) is unreachable. Also inert (resource variants set
`fields = Vec::new()`). Leftover from a pre-early-return refactor.
- Fix: delete the dead `else` branch and simplify the guard.

### (6) `src/target/shared/code/builder_simd_float_math.rs:517` — dead exp-setup broadcast
In `emit_float_kernel_setup`, the `Exp` arm emits `broadcast_i64(&k.v23, -1022)`, but
`k.v23` is never read anywhere in `emit_exp_body` (:1499-1629; exp uses v16-v21,
v24-v29). `-1022` was the old subnormal-flush bound (bug-130), replaced by the
two-step `2^n1·2^n2` scaling + v25/v26 saturation. The setup line is residual and
emits a wasted broadcast on every `math::exp` call.
- Fix: delete the dead `broadcast_i64(&k.v23, -1022)` in the Exp setup arm.

### (7) `src/target/shared/code/os/paths.rs:88` — `unreachable!` landmine invited by a live WindowsApp arm
`emit_executable_path_into` has `PlatformFamily::Windows => unreachable!("47-D owns
the Windows executable path")` (:88-90). `lower_executable_path` early-returns to
the Windows wide-string path before calling it, but `lower_resource_path` (:204)
does **not** early-return for Windows — it calls `emit_executable_path_into`
unconditionally (:305). Meanwhile `resource_base_offset` (:190) has a live
`NativeBuildMode::WindowsApp` arm (plan-66-I/J) returning `(1, "")`, signaling
intent to support `os.resourcePath` on Windows. Today this is unreachable only
because `"os.resourcePath"` is absent from `win_x86_64`'s `RUNTIME_CALLS` (so
`validate_capabilities` rejects it first). The moment someone adds it to that list —
a one-line change the WindowsApp arm invites — `os::resourcePath` on Windows ICEs
the compiler with a raw `unreachable!` instead of a diagnostic; the Windows exe-path
arm was never wired.
- Fix: wire a real Windows arm in `emit_executable_path_into` (or make
  `lower_resource_path` early-return for Windows like `lower_executable_path` does),
  so opening the capability gate can't ICE.

### (8) `src/target/win_x86_64/app/mod.rs:56` — unused writable global `TUI_FONT_SYM`
`TUI_FONT_SYM` ("cached monospace HFONT") is declared and gets a `writable_qword`
in `app_mode_data_objects` (:1404) but is never written or read — `emit_term_on`
fetches the font via `GetStockObject(16)` + `SelectObject` without caching it. Dead
writable data (8 wasted bytes) and the "cached … HFONT" claim is inaccurate.
- Fix: delete `TUI_FONT_SYM` and its `writable_qword`, or actually cache the HFONT.

### (9) `src/syntaxcheck/checking.rs:571` — dead discarded `_element_is_resource` (found during fix)
The `FOR EACH` handler computed `let _element_is_resource =
self.is_resource_type(&element_type);` and immediately discarded it — the exact
same relocated-scaffolding pattern as item (4)'s `_is_resource`. The comment
above it described the "loop variable may not close/RETURN/transfer the resource
(§15.6)" rule, which now lives in `ir::verify`. Surfaced while confirming item
(4); `is_resource_type` has many other live callers, so only the discarded
binding (and its now-orphaned comment) was removed.
- Fix: delete the discarded binding and its comment.

### (10) `src/builtins/general.rs:103` — dead `P_ERROR` parameter-list constant (found during fix)
`const P_ERROR: &[Parameter]` had no readers (a bare-`git grep` on base `main`
found only the definition). It is a **pre-existing** `dead_code` warning — base
`main` (67bc4018f) already emits `warning: constant P_ERROR is never used` (its
sole standing warning); it is unrelated to item (2) (that allow was scoped to
`src/os/windows/`, which does not cover `general.rs`, and unmasking the windows
module surfaced no new dead code). Noticed while building the item (1)–(8) fix
and removed per "never leave a bug you found (not excused by pre-existing)".
- Fix: delete the `P_ERROR` constant; keep the shared `P_*` group header comment.

## Goal

- No production field/item is retained solely on a "consumed by a later phase"
  justification; each is either read by real code or deleted. No file-level
  `#![allow(dead_code)]` masks a whole module.

### Non-goals

- No behavioral change; pure removal of unread state.

## Blast Radius

- Each item is isolated (write sites + the `#[allow]`); removal is compiler-checked.
- Additional dead-code items found later in goal-07 are appended here as they
  surface.

## STATUS: FIXED (fix 7221e2ef3, merged to main e9e8ab1f7)

All 10 items removed on a single integration worktree (`worktree-B-402`). The
compiler validates the removals: a clean `cargo build` (0 warnings — the standing
`P_ERROR` warning is gone) and the full suite green.

- [x] (1) `LinkFnSig.return_type`/`return_resource` fields + their writes deleted;
      only `params`/`param_resource`/`line` are read (via `link_target_signature`).
- [x] (2) `windows` module-wide `#![allow(dead_code)]` + stale doc paragraph
      removed; the three writers are called from `win_x86_64/mod.rs`. Unmasking
      surfaced no new dead code in the module.
- [x] (3) vacuous `ref_offset(..) > METADATA_BLOCK` guard removed (`ref_offset`
      returns `% 8192`, always < 8192); `ref_offset` keeps its two other callers.
- [x] (4) `seen_default` machinery (decl + empty `else if`) and the discarded
      `_is_resource` binding deleted; the coverage-only `non_default_after_default_walk`
      test (which asserted nothing and existed solely to walk the deleted branch)
      removed.
- [x] (5) unreachable `else` field-copy loop deleted and guard collapsed; the
      third scratch vreg's *allocation* is kept (as `let _ = self.temporary_vreg();`
      with a load-bearing comment) so `next_vreg` — and byte-identical codegen —
      is unchanged. Byte-identity proven: the full artifact-gate shows 0 diffs.
- [x] (6) dead `broadcast_i64(&k.v23, -1022)` exp-setup removed. Codegen-shifting:
      before/after `.ncode` diff on all SIMD targets shows ONLY the `-1022`
      broadcast materialization removed (v23 is never read in `emit_exp_body`).
      `tests/byte-identity/math` `.ncodesum` goldens (5 targets) regenerated.
- [x] (7) latent-ICE `unreachable!("47-D owns the Windows executable path")` in
      `emit_executable_path_into` replaced with a returned `Err` diagnostic, so
      opening the `os.resourcePath` capability gate on Windows degrades to a
      compile error rather than an ICE (the arm is unreachable today — gated out
      of `win_x86_64`'s `RUNTIME_CALLS` — so zero fixture reaches it, 0 golden
      change). A full Windows resource-path implementation is out of scope (a
      plan-66 follow-up).
- [x] (8) `TUI_FONT_SYM` const + its `writable_qword` removed. Codegen-shifting:
      before/after `.ncode` diff on the 3 `windows-x86_64.app` app-mode fixtures
      shows ONLY the `_mfb_winapp_tui_font` data object removed (1 line each, no
      offset shift). Their `.ncodesum` goldens regenerated.
- [x] (9) discarded `_element_is_resource` binding + orphaned comment removed
      (`checking.rs`).
- [x] (10) pre-existing dead `P_ERROR` const removed (`general.rs`).

Deviation from the doc's suggested fixes: item (7) uses the "returned diagnostic"
option rather than wiring a full Windows arm (that is an untested feature, out of
scope for a dead-code removal); item (5) keeps the scratch vreg *allocation* (not
the binding) to preserve byte-identical output — removing it would renumber
`next_vreg` across every value lowering and churn essentially all goldens for a
cosmetic gain. Items (9) and (10) were found during the fix and appended.

Verification: full artifact-gate `0 diff(s)` (1121 tests / 1511 goldens); full
`cargo test` `4197 passed; 0 failed` across 39 binaries; `cargo build` 0 warnings.
