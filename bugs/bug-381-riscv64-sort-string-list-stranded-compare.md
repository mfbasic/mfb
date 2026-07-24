# bug-381: riscv64 codegen panics compiling any string-list sort — a spilled compare operand is stranded across its flag-reading branch

Last updated: 2026-07-23
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (compile-time panic — no code is emitted; the target cannot build the program)

Status: Open
Regression Test: tests/rt-behavior/codegen-cover/cover-fs (build for -target linux-riscv64) — currently panics; add a linux-riscv64 `.ncodesum` golden once fixed.

Compiling **any** program that sorts a string list to `linux-riscv64` aborts the
build with a panic instead of emitting code. `_mfb_rt_sort_string_list` (the
selection-sort runtime helper emitted whenever a program orders string-keyed
collection output — e.g. `fs.listDirectory`, whose entries are sorted) contains
a `cmp v1,v7; b.lo take_j; b.hi keep_min` sequence. Under riscv64's register
pressure the allocator spills and **reloads one of the compare's operands
between the compare and its branch**. RISC-V has no condition flags, so
`select_riscv64` re-derives each branch from the compare's operands; once the
operand register is redefined by the reload, the pending compare is correctly
invalidated (bug-284 C3) and the trailing flag-reading branch then has nothing
to read — `select_riscv64` panics.

The single correct behavior a fix produces: `mfb build -target linux-riscv64`
of a string-sorting program **emits a correct `.exe`/object** (byte-stable
`.ncode`) that sorts identically to the linux-x86_64 / linux-aarch64 / macos
builds — no panic, and no silently-wrong branch.

This is riscv64-specific. On x86-64 and aarch64 the same sequence is correct:
the EFLAGS/NZCV result of the `cmp` is independent of the later reload, so the
branch reads valid flags. Only the flagless riscv64 backend, which must keep the
compared *values* live from compare to branch, is exposed. So it is **not** a
latent miscompile on the flag machines.

References:

- `src/arch/riscv64/select.rs:336` (`select_riscv64`) — the flagless expansion;
  the panic is at `:393` (`.expect("rv64: standalone flag branch without a
  preceding compare")`), the invalidation that strands it at `:470`–`:483`
  (Label / Call / rhs-redefinition clear `pending`).
- `src/target/shared/code/codegen_utils.rs:17` (`lower_sort_string_list_helper`)
  — the helper whose `cmp; b.lo; b.hi` at `:85`–`:87` is the trigger.
- `src/target/shared/code/mir.rs:469` — the cmp+branch fusion that only fires
  when the branch is *immediately* adjacent to the compare; a reload between
  them defeats it and drops to the bare-`pending` path.
- Lineage: bug-126 (flagless shared setter re-application) and bug-284 C3 (the
  invalidation of a stranded `pending` rhs) — this bug is the case those two
  comments call "latent, never fires today." It fires.
- Found during plan-47 (Windows x86-64) prerequisite work — seeding
  linux-riscv64 byte-identity goldens (§Prerequisites row 3). cover-fs is the
  one of 7 cover fixtures that cannot be captured because of this.

## Failing Reproduction

```
# At HEAD, with a release mfb built:
target/release/mfb build -ncode -target linux-riscv64 tests/rt-behavior/codegen-cover/cover-fs
```

- Observed:
  ```
  thread 'main' panicked at src/arch/riscv64/select.rs:393:22:
  rv64: standalone flag branch without a preceding compare
  ```
  No `.ncode` (or any artifact) is written. Instrumented, the stranded branch is
  `b.lo -> _mfb_rt_sort_string_list_take_j`, preceded (newest-first) by a
  `mov gp, t5` (the bare compare's lhs snapshot) and a `ldr_u64 dst=t3, [sp+80]`
  reload that redefines the compare's rhs register `t3`.
- Expected: a byte-stable `.ncode` dump is written and the build exits 0, as it
  does for the other three targets.

Contrast cases that build correctly today (bound the bug):

| Environment | target | Result |
| --- | --- | --- |
| cover-fs | linux-x86_64 | works ✓ |
| cover-fs | linux-aarch64 | works ✓ |
| cover-fs | macos-aarch64 | works ✓ |
| cover-fs | **linux-riscv64** | **panics ✗** |
| cover-audio / cover-tls / cover-os / cover-crypto / cover-net / crypto-ec-valid | linux-riscv64 | works ✓ (none emit `sort_string_list`) |

The riscv64 build of the six sibling cover fixtures succeeds, confirming the
fault is specific to the `sort_string_list` code shape, not riscv64 codegen
generally.

## Root Cause

`select_riscv64` (`src/arch/riscv64/select.rs`) emulates the absent condition
flags. A `cmp`/`cmp_imm` that is **immediately** followed by its flag-reading
branch is fused upstream (`mir.rs:469`) into one op and expanded to a native
`b<cond> rs1, rs2, label`. When the branch is *not* adjacent, the bare compare
takes the `pending` path (`:360`): it snapshots the lhs into `gp` and remembers
the rhs **by register name**. A later standalone branch re-derives the native
compare from `gp` + that rhs register.

That saved rhs is only valid until the register is redefined. bug-284 C3 added
the invalidation at `:470`–`:483`: on a Label, a Call, or any instruction that
writes the rhs register, `pending` is cleared. That invalidation is correct —
after the reload, the rhs register no longer holds the compared value.

The unhandled consequence: once `pending` is cleared, the trailing branch has
**no way to reconstruct the comparison at all**, and `:393` panics rather than
emit a wrong branch. The comment at `:459` asserts this is latent ("current
streams place the consuming branch immediately after the compare"). The register
allocator falsifies that: in `sort_string_list`, which keeps `v1..v16`
simultaneously live, it spills and reloads a compare operand into the
compare→branch span, so the compare and its branch are no longer adjacent and
the operand is redefined between them. Both the fusion miss and the invalidation
then fire, and the branch is stranded.

Why the flag machines are immune: on x86-64/aarch64 the `cmp` writes the flags
register, and the subsequent reload of a *general* register does not disturb the
flags, so the branch reads a valid condition regardless of adjacency. riscv64
has no such independent flag state — it must keep both compared values live —
so the same schedule that is fine there is impossible here.

## Goal

- `mfb build -target linux-riscv64` of a string-sorting program emits a
  byte-stable `.ncode` and a correct object; no panic.
- The emitted riscv64 sort produces the identical ordering the other three
  targets produce (behavioral parity, provable on a riscv64 host / qemu).
- The fix is robust to *any* schedule the allocator produces, not just this
  helper's — the compare→branch flag emulation must survive a spill/reload or a
  redefinition of either operand in the span.

### Non-goals (must NOT change)

- No change to x86-64, aarch64, or macos emitted bytes. This is a riscv64-only
  backend fix; the byte-identity guard for the other targets must stay green.
- No change to the shared `sort_string_list` helper's neutral instruction stream
  to "route around" riscv64 — the helper is target-neutral and correct; the
  defect is in riscv64 selection. (Tempting wrong fix: reorder/duplicate the
  compare in `codegen_utils.rs` so it lands adjacent to its branch. That masks
  this one helper while leaving every other pressure-induced schedule exposed,
  and perturbs the other three targets' bytes. Forbidden.)
- No weakening of the bug-284 C3 invalidation — it prevents a silently-wrong
  branch. The fix must make the stranded case *recoverable*, not un-detected.

## Blast Radius

Found by searching `select.rs` for the `pending`/flag-emulation path and every
runtime helper that emits a non-adjacent or multi-branch compare.

- `_mfb_rt_sort_string_list` (`codegen_utils.rs:85`) — the observed failure.
  Fixed by this bug.
- The bare-`pending` path in `select_riscv64:360-435` — the actual defect site.
  Any helper whose compare and branch are separated by a spill, a label, or a
  call is latent-exposed by the same mechanism. The `:49` comment names "a few
  hand-written net/link helpers" as deliberate users of the non-adjacent path;
  they do not currently panic only because their operands happen not to be
  reloaded in the span. Fixing the selector (below) covers them too.
- Fusion at `mir.rs:469` — unaffected; it is correct that it only fuses adjacent
  pairs. The fix belongs in the riscv64 fallback, not in fusion.
- x86-64 / aarch64 selectors — unaffected (independent flag state).

## Fix Design

The correct-and-robust fix keeps the *compared values* live across the span
instead of the operand *registers*. Two shapes, to be decided in Phase 2:

- **(a) Snapshot both operands at the compare.** Today the bare compare saves
  only the lhs (into `gp`) and keeps the rhs by name. Also snapshot the rhs into
  a second reserved, call-preserved scratch register at the compare point, and
  have the branch read the two snapshots. Immune to any later redefinition or
  reload of the original operands. Cost: one extra `mov` per bare
  register-compare, and it needs a second reserved register with `gp`'s
  properties (never codegen-allocated, preserved across calls) — audit the
  riscv64 `regmodel` reserved set for a free one (`tp`/`x4` is the candidate;
  confirm it is not otherwise used). Changes riscv64 bytes for every function on
  the bare path (the net/link helpers) — acceptable, and newly-captured goldens
  will pin them.
- **(b) Re-materialize the compare from the spill slot.** Have the allocator/
  selector reload the compared value into a scratch at the branch. More
  invasive; interacts with slot lifetimes. Likely rejected in favor of (a).

Correctness risk concentrates in the reserved-register choice for (a): a wrong
pick (a register the ABI or a helper actually uses) is a silent riscv64
miscompile. Validate against a riscv64 run, not just a rebuild.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a linux-riscv64 build assertion for a string-sorting fixture
      (cover-fs is sufficient; a minimal `List<String>`-sort program is
      cleaner). Confirm it panics at `select.rs:393` today.
- [ ] Audit the riscv64 reserved-register set (`src/arch/riscv64/regmodel.rs`)
      for a second `gp`-like register, and enumerate every runtime helper that
      reaches the bare-`pending` path. Write each verdict into §Blast Radius.

Acceptance: the new test fails for the documented reason; the reserved-register
audit names a concrete second scratch (or rules (a) out).
Commit: —

### Phase 2 — the fix

- [ ] Implement (a) (or (b) if the audit kills (a)) in `select_riscv64`: keep
      both compared values live from compare to branch.
- [ ] Confirm the bug-284 C3 invalidation still fires for genuinely-dead cases
      and no longer strands recoverable ones.

Acceptance: Phase 1 test passes; riscv64 `.ncode` is byte-stable across two
builds; x86-64/aarch64/macos bytes unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Capture the linux-riscv64 `.ncodesum` golden for cover-fs (and any other
      newly-buildable fixture), completing plan-47 §Prerequisites row 3.
- [ ] Run the full acceptance suite + `scripts/artifact-gate.sh` (all four
      targets byte-identical except the intended new riscv64 goldens).
- [ ] Run the sorted-output program on a riscv64 host / qemu and confirm the
      ordering matches the other targets.

Acceptance: full suite green; only riscv64 bytes shift, as intended; sort output
matches cross-target.
Commit: —

## Validation Plan

- Regression test: linux-riscv64 build of a string-sort fixture (cover-fs), plus
  its new `.ncodesum` golden once buildable.
- Runtime proof: the sorted collection output on riscv64 equals the x86-64
  output for the same input (behavioral parity, not just "it compiles").
- Doc sync: none expected — no spec/ABI contract changes; riscv64 already
  advertises the full surface.
- Full suite: the project acceptance suite + `scripts/artifact-gate.sh`.

## Open Decisions

- Fix shape (a) snapshot-both-operands vs (b) re-materialize-from-slot — §Fix
  Design recommends (a), pending the reserved-register audit in Phase 1.

## Summary

The engineering risk is in the reserved-register choice for the flag-emulation
snapshot: it is a riscv64-only selector change, but a wrong register is a silent
miscompile, so it must be proven on a riscv64 run, not merely a rebuild. The
shared helper and the other three backends stay untouched. Until this lands,
linux-riscv64 cannot compile string-sorting programs, and cover-fs cannot carry
a riscv64 byte-identity golden.
