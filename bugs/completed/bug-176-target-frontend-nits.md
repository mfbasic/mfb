# bug-176 — target/regalloc frontend nits (bump-regalloc hands out pinned regs, over-permissive validation, incomplete import arm, assertion panics, stale attrs)

Last updated: 2026-07-12
Severity: LOW (batch).
Class: Correctness / Footgun / Dead-code.
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Findings

**A. `temporary_register` hands out the pinned `%thread` (x20) and `%closure_env`
(x28).** `src/target/shared/abi.rs:68-87`. `temporary_register(18) => x20`,
`temporary_register(26) => x28`; post-plan-34 x20 is `CURRENT_THREAD` and x28 is
`CLOSURE_ENV`. Under the non-default `-regalloc bump`
(`builder_codegen_primitives.rs:22` uses the result verbatim as the eager
physical), the bump oracle can color a body vreg onto a program-wide pinned
register and clobber it. The linear-scan allocatable set excludes x20 (plan-34-C)
but this bump list was not updated. Trigger: a worker/closure body needing ≥11
simultaneous int temporaries under `-regalloc bump`. Fix: drop x20/x28 from
`temporary_register`'s table (or reject those slots).

**B. Any uppercase-initial `Local` reference passes NIR validation.**
`src/target/shared/validate.rs:1218-1228, 1526-1530`. `is_imported_constructor_name`
returns true for any name whose first char is ASCII-uppercase, so the
`NirValue::Local` arm accepts a malformed/typo'd `Local("Foobar")`
unconditionally, removing the resolution backstop for every capitalized reference
(a genuinely-dangling one reaches codegen). Fix: track imported constructor names
explicitly rather than matching on first-letter case.

**C. Thread runtime-imports arm omits `transferResource`/`acceptResource`.**
`src/target/linux_aarch64/plan.rs:232-234` (and `linux_riscv64/plan.rs:231-233`,
`linux_x86_64/plan.rs:245-247`). `thread::transfer`/`accept` lower to
`thread.transferResource`/`acceptResource` (distinct `RuntimeHelperSpec`s), which
are not in the thread match arm, so `runtime_imports` returns `Vec::new()` — no
pthread imports declared for their symbols. Masked in practice (any transfer/accept
program also called `thread.start`, which declares the full pthread set,
deduplicated). Fix: add the two calls to the thread arm; drop the dead
`thread.read`/`emit`/`drop` entries that have no reachable lowering.

**D. Codegen assertion panics on programmer error (batched, footgun).**
`src/target/linux_gtk/mod.rs:231` (`lib_for` `panic!("... referenced unmapped
symbol")`) and `src/target/shared/code/regalloc/mod.rs:190-214`
(`find_physical_operand` matches all three ISAs' register spellings on every
target and scans all fields, so a symbol/label token literally named `ra`/`gp`/
`s0`/`w0`/`q3` is flagged as a physical-register regression → hard error at the
call sites). Both reachable only via programmer error / token collision (latent).
Fix: return a plan-level error instead of `panic!`; restrict the physical scan to
register-role fields and select the active ISA's parser.

**E. Stale `#[allow(dead_code)]` + misleading "Phase 5 / unused" comments on
live constants.** `src/target/macos_aarch64/app/mod.rs:232, 238-241, 368-377`
(`ARENA_ALLOC_SYMBOL`, `ERR_UNSUPPORTED_*`, the five `CELL_*_OFFSET`s are all
consumed by `emit_app_terminal_size`/`emit_term_view_draw_rect`). The attributes
suppress a lint that would catch a genuinely-orphaned constant later. Fix: drop
the attrs and correct the comments.

**F. Coverage dump aborts the whole SUB on the first file's write error.**
`src/testing/desugar.rs:663-678` (+`:621-632`). `dump_list_to_file` wraps
`fs.writeText` in an inline TRAP whose handler is `Exit { target: Sub }` →
`Return` from `__mfb_cov_dump`; the two dump blocks are flattened sequentially, so
a trap in the first (`covdata`) skips the second (`covfail`) even though the
comment says "best-effort". Coverage tooling only. Fix: use a continue-past
handler (RECOVER) so each dump is independently best-effort.
