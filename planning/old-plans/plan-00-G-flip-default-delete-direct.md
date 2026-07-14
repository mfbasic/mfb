# plan-00-G — Flip Default to MIR, Delete the `direct` Path

Last updated: 2026-06-29

The commitment plan. Once plans A–F all hold AArch64-byte-identical under `-codegen mir`,
make MIR the **default** and **remove the `direct` (no-MIR) AArch64-emitting path** — so the
AArch64 backend *is* `MIR → machine`, with one code path (`mir.md §13` step 4).

Depends on plan-00-A–F all green (byte-identical, full suite).

## 1. Goal

- `-codegen mir` becomes the default; `-codegen direct` is removed.
- Delete the legacy direct AArch64-emitting builder paths and the `select_aarch64`-only-as-
  oracle scaffolding. The `lower_*`/builders now produce **MIR**; the AArch64 backend is the
  sole consumer (selection + encode).
- `-mir` and `-ncode` remain (neutral dump + final dump); the byte-identical *differential*
  harness is retired (there is no second path to diff against).

### Non-goals

- No new ISA (that is H/I).

> **plan-00-F follow-up — the helper vreg migration (load-bearing for H/I).**
> F left the *bodies* of the hand-written runtime helpers (`lower_*` in
> `entry_and_arena.rs` etc.) emitting AArch64 via `abi::*` with **fixed physical
> registers** (`x9`/`x10`/`v22`…) — neutral *ops*, but pinned *registers*. That
> is fine for byte-identical F (the registers ride through `select_aarch64`
> unchanged) but **blocks H/I**: both are "additive — must not touch shared
> code," and H runs *the shared register allocator with the x86_64 RegisterModel*
> — which can only assign registers to **vregs**, not to a literal `x9`. So the
> helpers must be rewritten to **vreg-based MIR** (let the allocator place
> registers per-ISA). That re-pins the AArch64 registers too, so it is **NOT
> byte-identical** — it cannot live in F, and it is the one part of G that is not
> a pure no-op. It is scoped as G's Phase 2 below and validated by the **runtime
> suite**, not the self-diff (which is being retired here anyway).

## 2. Current State

After A–F: two paths exist behind `-codegen <direct|mir>`, proven byte-identical across the
suite. `direct` is dead weight — the same logic now flows through MIR. Keeping both
indefinitely doubles maintenance and risks drift.

## 3. Design

- Switch the default; update `mfb build` help/usage; update any docs/goldens that named
  `direct`.
- Delete the direct-emit code (the old `abi::`/`CodeOp`-direct builder shortcuts) and the
  differential-oracle plumbing. The `RegisterModel`/encoder/peephole stay (they are the
  AArch64 backend).
- Regenerate `.ncode`/`.nobj`/`.hex` goldens from the MIR path (must equal the pre-flip
  goldens — byte-identical — so really a no-op regeneration; verify, don't assume).

## 4. Phases

1. **(byte-identical) — DONE (commit 1cbcb7e8).** Flipped the default to `mir`;
   removed the `-codegen direct` option + the `CodegenKind` machinery; made the
   MIR round trip unconditional in `run_register_allocation` (builders) and
   `route_function_through_mir` at plan assembly (helpers); deleted the
   differential self-diff harness (`scripts/codegen-selfdiff.sh`); updated usage
   + the CLI-reference / MIR-instruction-set spec topics. Verified: native-code
   goldens byte-identical to pre-flip (default-mir == old direct); only
   `audit_usage.audit` (embeds USAGE) changed. Acceptance 975/975.
2. **(NOT byte-identical — runtime-validated) The helper vreg migration —
   COMPLETE for all shared-compute + macOS helpers (arena family, RNG, error
   builders, simd_alloc, shutdown/signal/link_init, ALL 24 fs, ALL 14 net,
   link_thunk, all 4 macOS TLS); program-entry/trampoline/app are per-backend
   hooks. Only the Linux-OpenSSL TLS bodies (untestable here) + the objc block
   trampolines remain, both deferred as per-backend.** Rewrite every
   `lower_*` helper body to build **vreg MIR** (neutral ops, `arena_base`,
   `%vN`/`%fN` instead of `x9`/`v22`…) and run it through the shared allocator +
   AArch64 RegisterModel — the same path the builder functions already use.
   Delete the `abi::`-hand-asm helper bodies.

   > **Status (current session):**
   > - 2a (carry liveness fields in regalloc/analysis.rs), 2b (no new op needed on
   >   AArch64 — `mov;bl;mov` + clobber model), 2c (`finalize_vreg_helper` in
   >   codegen_utils.rs + `Vregs` counter) — **DONE.**
   > - 2d migrated + committed, acceptance 975/975 zero `.run` regressions:
   >   RNG/fill ×5 (`c1adf1b0`), error builders ×2 (`17e8553a`), arena leaves ×3
   >   (`14b672cf`: arena_insert_free, arena_free, simd_alloc_list).
   > - **arena_alloc: DONE (commit a45742f3).** Root cause of the prior SIGBUS
   >   found+fixed: arena_alloc's hand-written callers (builder collection/string
   >   lowerings, fixed physicals in `_mfb_fn_*`) rely on the registers the original
   >   PRESERVED on its first-fit fast path (x8/x11/x12/x13/x16/x17 — e.g.
   >   `lower_list_remove_at` holds the new-count in x13 across the call). A
   >   vreg-migrated arena_alloc colored scratch into x13 → corrupt header → SIGBUS
   >   (regex's grow path + a caught TRAP allocating an ErrorLoc is the trigger). FIX:
   >   (a) added `reserved: &[&str]` to regalloc (`allocate`/`linear_scan::run` +
   >   `finalize_vreg_helper_reserved`), reserve those six so the fast path preserves
   >   them; (b) the grow path's nested `arena_fill_random` clobbers x8/x11/x12/x13 →
   >   save/restore those around the bl via spilled vregs (as the original did). Also
   >   decoupled `emit_arena_map(size_reg)`.
   > - **Machine-y tier — more DONE this session (all committed, suite 975/975):**
   >   arena_destroy + lower_shutdown + lower_signal_handler (d1134061);
   >   lower_link_initializer + `Vregs` pub(super) (ad184470); fs.exists +
   >   `finalize_vreg_body` + fs-helper 4-tuple (ac71de0f).
   > - **fs (24/24) — DONE.** `finalize_vreg_body_with_locals` adds the explicit
   >   on-stack buffer (sp+0..local_size, spills above); fs_helpers_paths (incl.
   >   list_directory), fs_helpers_io (8), fs_helpers_atomic (6) all migrated.
   > - **net (14/14) — DONE (commit 34f4e7ed).** All net/io + net/poll + net/mod
   >   endpoint/wrappers. Rename ephemeral x9-x17 scratch → vregs, keep the OS-struct
   >   frame as an explicit local region. Bug class fixed: `emit_errno` writes
   >   physical x9 (untouched by the rename) → `mov %v9,x9` bridge after each.
   > - **link_thunk — DONE (commit a2f6ecd2).** C-ABI marshaling thunk; blr through a
   >   vreg target; `emit_link_expr` writes physical x9 → same bridge.
   > - **macOS TLS (4/4) — DONE (commit 6aae66b2).** connect/read/write/close;
   >   validated by a real TLS round-trip to example.com:443. The objc/dispatch block
   >   trampolines stay hand-written (fixed-ABI, like the thread trampoline).
   > - **OpenSSL/Linux TLS (4/4) — DONE.** tls/openssl.rs Linux bodies vreg-migrated;
   >   dlopen'd OpenSSL fn pointers called via `blr` through a vreg target; errno
   >   bridge. Validated on the Alpine/musl aarch64 remote (`ssh -p 2224`): cross-build
   >   `mfb build -target linux-aarch64 tests/func_tls_connect_valid` → scp the
   >   `-musl.out` → run; byte-identical to the pre-migration baseline (real
   >   SSL_connect/write/read/close round-trip to example.com:443).
   > - **Only remaining (genuinely per-backend, NOT vreg-able):** the objc/dispatch
   >   block trampolines in tls/macos.rs (fixed-ABI entry points like the thread
   >   trampoline). An x86 macOS backend supplies its own.
   > - **NOT migratable (machine floor — leave hand-written):** lower_program_entry
   >   (`add x19, sp, 0` establishes the arena_base/sp invariants the allocator
   >   presumes; no caller, no `ret`), thread trampoline, app bootstrap/term_view.
   >   The plan over-scoped by listing these as Phase-2 migration targets.
   > **Infrastructure prerequisites verified missing (each is real backend work,
   > not a rewrite):**
   > - **(2a) Explicit-carry `addc`/`subc` — DONE** (commit bb8ec574). BUT the
   >   register allocator does **not** yet track the new operand fields: `DEF_FIELDS`
   >   is `["dst"]` and `USE_FIELDS` omits `carry_out`/`carry_in`/`borrow_out`/
   >   `borrow_in` (regalloc/analysis.rs). Vreg-ifying any add_carry user needs
   >   those added, or the allocator mis-models the carry value's liveness.
   > - **(2b) ABI-abstract `call`/`syscall`.** Calling helpers (build_error_loc,
   >   make_error_result, arena_alloc→mmap, shutdown ×2, signal, thread trampoline,
   >   the entry shim ×6 calls) hand-place args in physical `x0`–`x7` and read the
   >   result from `x0`. For the allocator to manage registers across a call, the
   >   call must model `args=[vregs]`/`rets=[vregs]` and place them in the backend.
   >   The `call_clobber_mask` special-case (`_mfb_arena_alloc` tramples callee-saved
   >   `x20`–`x28`) also has to be revisited once arena_alloc is allocator-managed.
   > - **(2c) Helper → allocator + frame routing.** `finalize_frame` is reusable,
   >   but helpers carry *bespoke manual frames* (manual callee-saved save/restore,
   >   `sp` setup, the entry/thread stack setup) that must be stripped and rebuilt.
   >
   > Tractability tiers (do in this order, runtime-validate each): pure-compute
   > leaves first (rng_next, rng_seed_at, arena_fill_seed — no calls, add_carry is
   > now vreg-friendly once 2a's field tracking lands); then the arena family +
   > error builders (one call each); then the math kernels (already builder/vreg —
   > confirm); LAST the machine-y ones (entry shim, shutdown/signal, syscall stubs,
   > thread trampoline — non-PCS register setup, hardest). Each helper that breaks
   > can corrupt the runtime foundation (arena, RNG, threads, syscalls), and there
   > is no byte-identical oracle — only the runtime suite. So this is genuinely
   > incremental, multi-session work, and is best tracked as its own plan.
   > **Prerequisites discovered (the migration is blocked on these — they are
   > new MIR primitives, not rewrites).** The helpers rely on hardware state the
   > vreg allocator cannot preserve, so these must land *first*:
   > - **Explicit-carry `addc`/`subc` (mir.md §4).** The PCG64 RNG, `rng_seed_at`,
   >   and `arena_alloc` (6 sites in `entry_and_arena.rs`, plus `builder_numeric`)
   >   use `adds; adc` 128-bit chains where the carry rides the *flags register*
   >   between two instructions. A vreg allocator may insert a spill/reload/move
   >   between them, clobbering the carry. Carry must become an explicit value
   >   (`addc dst, carry_out = a + b + carry_in`) so it is a vreg dependency, not
   >   an implicit flag, before any carry-using helper can be vreg-ified.
   > - **ABI-abstract `call`/`syscall` (mir.md §7).** Helpers hand-place args in
   >   physical `x0`–`x7` (and the syscall nr in `x8`/`x16`) around `bl`/`svc`.
   >   For the allocator to manage registers across a call, the call must model
   >   `args=[vregs]`/`rets=[vregs]` and place them per-ISA in the backend.
   > - **Helper → allocator + frame routing.** Helpers bypass `regalloc::allocate`
   >   and `finalize_frame` and carry *bespoke manual frames* (manual callee-saved
   >   save/restore, `sp` setup). Routing them through the shared pipeline means
   >   stripping the manual frame and letting `finalize_frame` rebuild it.
   >
   > So Phase 2 is really a small sub-series: (2a) explicit-carry op + selection,
   > (2b) ABI-abstract call/syscall + selection, (2c) helper allocator/frame
   > routing, (2d) migrate helpers in risk order (pure-compute leaves → arena →
   > error/RNG → kernels → entry/shutdown/signal → thread trampoline), runtime-
   > validating each. Not a single edit.
   The AArch64 helper output *changes* (allocator re-pins), so:
   - validate by the **runtime suite** — RNG reproducibility, thread/transfer
     tests, signal/shutdown, the `_invalid` trap codes+locations, and the **ULP
     harness** (the kernels are the accuracy gate);
   - regenerate the helper-affected `.ncode`/binary goldens from the MIR path
     (they will differ from pre-flip — that is expected here, unlike Phase 1);
   - go helper-by-helper (arena family → error builders → RNG → kernels →
     entry/shutdown/signal → thread trampoline), keeping the runtime suite green
     after each, since there is no longer a self-diff oracle.
   After this there are **no hand-written AArch64 helper bodies** — every helper
   is vreg MIR the shared allocator places, which is what makes H/I additive
   (`src/arch/<isa>/` only, no shared-code edits).
3. Full suite green.

## 5. Validation

- Full unfiltered `scripts/test-accept.sh` green with the **same** goldens as before the flip
  (the equivalence was already proven; this confirms removal didn't perturb anything).
- No reference to `direct`/the no-MIR path remains in code or docs.

## Summary

The point of no return — and the payoff of doing A–F byte-identically. After this, there is
one AArch64 code path (MIR→machine), the MIR is the real intermediate, and the project is a
genuine multi-ISA compiler with one ISA implemented. Everything from here (x86_64, rv64) is
*adding a backend*, never *forking the frontend*.
