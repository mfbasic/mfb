# plan-161-B: Keep temporaries in registers at the default level

Last updated: 2026-09-26
Effort: large (3h–1d)
Depends on: plan-161-A

bug-740 mechanism (b). The lowering gives every local, parameter and operand
temporary its own 8-byte frame object and round-trips each value through it:
brogue's `gi` (`x * DROWS + y`) stores `x0`, `x1`, both multiply operands, the
product and both add operands, and reloads each before use. This plan makes
those slots registers at `-O1`, in three layered pieces: a Level-1 **stack slot
promotion** row that turns a provably private one-word slot into a virtual
register; a register-allocator refinement so a value that is merely *dead* at a
call inside its interval (the raise path of every checked operation) is not
forced out of the caller-saved registers; and **register coalescing** moved from
Level 2 to Level 1 so the copies promotion leaves behind collapse.

The single behavioral outcome: at `-O1`, brogue's `gi` touches the stack only to
save `lr`, and every program computes, raises and reports exactly what it did.

References:

- `bugs/bug-740-brogue-dig-leaf-call-and-spill-overhead.md` — mechanism (b).
- `src/codegen/engine/regalloc/linear_scan.rs`, `analysis.rs` — the allocator.
- `src/codegen/engine/regalloc/builder_registers.rs:run_register_allocation` —
  where the promotion runs.
- `.ai/codegen-invariants.md` (vreg-alloc order, `_mfb_*` clobbers),
  `.ai/compiler.md` ("Native Codegen Register Lifetimes"), `.ai/arch-abi.md`.
- `mfb spec memory native-calling-convention` — frame layout.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-161-A complete | `ls planning/plan-161-A-*` → no match (archived to `planning/completed/`) | MET in substance (2026-09-27): plan-161-A phase 1 landed (`2d170c339`); its phase 2 (goldens + gates) runs together with this plan's — see Correction 3 |

## 1. Goal

- `mfb build -ncode` of brogue at `-O1`: `gi`'s body has no `sp`-based
  `ldr_u64`/`str_u64` except the `lr` save/restore.
- `tools/brogue-perf/bench.sh --runs 7` (heavy lock held) at or below 10x.
- Every program's output, exit code, runtime error (code, file, line, column),
  `--debug` report and allocation counts are unchanged.

### Non-goals (explicit constraints)

- No reordering, duplication or removal of any computation, check or call:
  promotion changes *where a value lives*, never *what executes*.
- No change to frame layout visible to anything outside the function: no code
  outside a function reads its frame (no frame-pointer chain, no unwinder —
  `mfb spec memory native-calling-convention`), and every object this plan does
  not promote keeps its offset.
- Owned-value cleanup slots, string-capacity shadows and any slot whose address
  is taken keep living in memory (they are exactly the slots something other
  than a whole-word load/store touches).
- `-O0` codegen unchanged.
- The `.mir` dump stays the lowering's own output (promotion runs after the
  capture).

## 2. Current State

- `CodeBuilder::allocate_stack_object` hands out consecutive 8-aligned frame
  objects (`stack_slots`); lowerings store and reload through them
  (`lower_arithmetic_binary`'s `arith_left`/`arith_right`, comparison
  `cmp_left`/`cmp_right`, every local and parameter).
- The linear-scan allocator colors virtual registers by over-approximated
  intervals `[first, last]` and forbids a register clobbered by *any* call in
  that interval (`call_clobber_in`), and every `_mfb_*` call clobbers all
  integer registers (`.ai/codegen-invariants.md`). Every checked operation has
  an inline raise path calling `_mfb_make_error_result` and `_mfb_rt_park_error`,
  so almost every interval contains a call at which the value is dead.
- Register coalescing (copy hints + self-move sweep) exists at Level 2.
- Opt2's memory rows (store-to-load forwarding, DSE) are Level 3 and stop at
  every call.

### Measured populations

| What | Count | Command |
|---|---|---|
| frame-address sites in the lowering | 568 | `grep -A3 'add_immediate(' src/codegen \| grep -B1 -A2 'stack_pointer()' \| grep -c add_immediate` |
| brogue frame addresses by consumer | 1738 drop_owned_collection, 692 drop_owned_string, 32 drop_owned_string_cap, 16 other (all in builtin raw-frame helpers) | `mfb build -ncode examples/brogue` + a scan of each `add_imm _, sp, _` to its next `bl` (session probe) |
| brogue `sp`-based memory ops | 210,397 `ldr_u64`, 166,022 `str_u64`, 1 `str_u8` | same dump, count of ops with `base == sp` |
| brogue slots promoted (prototype) | 29,453 | `mfb build -v examples/brogue` → `Stack slot promotion: 29453` |
| ratio, prototype (lock NOT held, median of 5; base 17.6x) | promotion + live-across + coalescing 7.5x; without coalescing 8.5x; without live-across 8.8x | `/tmp/b740-measure.py 5 oracle base ab ab_nocoal ab_nolive` |
| compile time, brogue (debug mfb, user s) | 16.9 → 18.6 | `/usr/bin/time -p mfb build -q examples/brogue` base vs prototype |
| compile time, `byte-identity/regex` (debug mfb, user s) | 7.99 → 8.09 | same |

### Verified properties

- The drop helpers read and zero exactly the one word their `x0` points at
  (`DROP_OWNED_*_SYMBOL` docs in `src/codegen/error/constants/error_constants.rs`;
  `emit_owned_value_drop` emits `add_imm c_arg(0), sp, slot` immediately before
  the `bl`).
- Owned-value slots and string-capacity shadows are zeroed by a store spliced
  in *after* register allocation (`function_lowering.rs`, the
  `owned_value_slots` / `string_capacity_slots` splices), so promotion must not
  touch them — they are passed in as pinned.
- Incoming/outgoing stack arguments use the `incoming_args`/`outgoing_args` base
  tokens, not `sp`, until `finalize_frame` (`src/target/shared/abi.rs`), so they
  never look like frame objects.
- A vreg that is dead after a call cannot be harmed by that call: the only
  other occupants of its register in `[first, last]` are excluded by the
  interval test, and "dead after" means every path from the call redefines it
  before any use.

## 3. Design Overview

1. **Allocator refinement** (`analysis::analyze`, `linear_scan::run`): compute,
   during the existing backward liveness walk, the union of clobber masks of the
   calls each vreg is *live-out* of (`vreg_live_across_clobber`). At Level 1,
   coloring checks only that mask; `-O0` keeps the whole-interval rule. Splitting
   (Level 2) keeps its own conservative range checks.
2. **Stack slot promotion** (`optimizer::opt2::promote`, Level 1): on the
   pre-selection stream, after the `-mir` capture, classify every instruction
   naming `sp` (full-word access / pin / whitelisted address / unknown); an
   unknown use or an access outside every registered object disables the whole
   function. Promote every one-word, unpinned object whose every access is a
   full-word access at its start: `str src` → `mov %vO, src` (`xzr` →
   `mov_imm %vO, 0`), `ldr dst` → `mov dst, %vO`.
3. **Coalescing at Level 1**: the catalog row "Register coalescing" moves to
   Level 1 (copy hints + self-move deletion; every interference test still has
   to pass, so it can lose a choice, never make an unsafe one).

Correctness risk concentrates in (2)'s reachability proof — a missed way to
reach a frame word would turn a store into a lost write. The design makes every
unrecognized `sp` mention fail closed (whole function off), and admits frame
addresses only for the three documented single-word helpers.

**Gate class: not byte-identical.** Every function's `.ncode` changes on every
target; `.mir` does not. Expected diffs: `sp` loads/stores replaced by register
moves or removed, frame sizes shrinking, register choices shifting. A diff that
adds or removes a call, a compare or a branch is a bug.

Rejected alternatives:

- *Stop the builders spilling operands* (keep `left` in a vreg across
  `right`'s lowering). Correct but touches every lowering site (arithmetic,
  comparison, boolean, calls, locals) one by one; promotion covers all of them
  at one seam, including locals and parameters the builder cannot keep.
- *Move the Level-3 memory rows down.* They stop at every call and never delete
  the stores; measured at `-O3` brogue's `gi` still stores every slot.
- *Move raise paths out of line so intervals skip them.* A larger codegen change
  with the same effect the liveness refinement gets for free.

## Phases

### Phase 1 — allocator: only calls a value is live across constrain it

- [x] `analysis.rs`: `Liveness::vreg_live_across_clobber`, filled in the
      interval-expansion walk.
- [x] `linear_scan.rs`: at Level 1, `clobbered` is that mask; the `usable` test
      uses it.
- [x] Unit test (`regalloc/tests.rs`): a vreg defined before an inline raise
      path (a `bl` then `ret`) and used after it gets a caller-saved register at
      `-O1` and is kept out of them (or spilled) at `-O0`; a vreg live across a
      real call still avoids the clobbered set at `-O1` (the existing
      `linear_scan_spills_integer_across_arena_alloc` /
      `..._keeps_value_across_call_in_callee_saved`, which run at the default
      level). `cargo test --bin mfb regalloc` → 21 passed.

Acceptance: `cargo test --bin mfb regalloc` passes.
  Check: that command (est. 10 min; bin test build).
Commit: 8b36c06e3

### Phase 2 — stack slot promotion

- [x] `src/optimizer/opt2/promote.rs` (the design above) + `mod.rs` wiring.
- [x] `builder_registers.rs:run_register_allocation`: run it after the `-mir`
      capture with the frame objects, frame size, and the post-allocation zeroed
      slots.
- [x] `stats.rs`/`catalog.rs`: the "Stack slot promotion" row (Level 1, stage
      regalloc) and its counter.
- [x] Unit tests in `promote.rs`: promotes a private slot; zero store becomes
      `mov_imm`; pins an address-taken slot fed to a drop helper; disables the
      function for an address fed to anything else, for an unknown `sp` use, and
      for an access outside every object; pins a narrow access and a
      zeroed-after-allocation slot; leaves a two-word object alone; off at `-O0`;
      keeps a slot some path reads before writing (Correction 1).
      `cargo test --bin mfb promote::tests` → 8 passed.
- [x] Codegen test `tests/codegen/codegen_stack_slot_promotion.rs`: a
      `gi`-shaped function's `.ncode` has no `sp`-based `ldr_u64`/`str_u64`
      except the `lr` save/restore at `-O1`, and still has them at `-O0`; the
      program (including an overflow inside the helper) prints the same at both.
      `MFB_TEST_EXE=target/debug/mfb cargo test --test
      codegen_stack_slot_promotion` → 2 passed; with the main-tip compiler
      `a_leaf_helper_keeps_its_values_in_registers` FAILS (RED at base).
      brogue `dig` seeds 1–8 × depths 1–40 against the oracle (debug-built
      compiler at this phase): 320 pairs, 0 mismatches.

Acceptance: the unit tests and the codegen test pass;
`examples/brogue/check/check-terrain.sh 8 1` → 320 pairs match.
  Check: those commands (est. 15 min).
Commit: d34fdae20

### Phase 3 — register coalescing at Level 1

- [x] `linear_scan.rs`: `coalescing = level_enabled(1)`; `catalog.rs`: the row's
      level 2 → 1.
- [x] Unit test: the coalescing hint fires at `-O1` and not at `-O0`
      (`coalescing_removes_a_dying_copy_at_level_one`). `cargo test --bin mfb
      regalloc` → 22 passed.

Acceptance: `cargo test --bin mfb regalloc` passes.
Commit: 71c76232a

### Phase 4 — goldens, docs, budget (largest blast radius)

- [x] (`scripts/regen-native-goldens.sh target/release/mfb` after `scripts/artifact-gate.sh target/release/mfb all` gave 0 diffs at main tip b1a563acd and 168 on the branch (all five targets; .ncode/.ncodesum, 2 .mir, app-mouse-surface .nir/.nplan; no .ir/.ast); gate after regen: 1537 tests, 2170 goldens, 0 diffs (`2a009e7e5`). One fixture inspected: app-mouse-surface .nir — `#CANVAS_*` LET reads became literals, `#canvas_geoAt` calls inlined.) Classify then regenerate the native goldens (`scripts/regen-native-goldens.sh`)
      against a detached main-tip baseline gate; inspect one fixture's `.ncode`.
- [x] `planning/optimizations.md`: the new row and the coalescing level change;
      spec: `mfb spec tooling cli-reference`'s `-O` line and
      `mfb spec memory native-calling-convention`'s frame section (which values
      live in frame objects at `-O1`).
- [x] `tools/brogue-perf/bench.sh`: tighten `BUDGET_RATIO` to the measured floor — A+B 7.6x (lock held), budget 25 → 11 (`ce0964dc9`).

Acceptance: `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`.
Commit: —

## Validation Plan

- Tests: the unit tests above; the whole existing runtime suite is the real
  proof — every `rt_*` test and `rt-behavior`/`rt-error` fixture now runs with
  promoted slots.
- Coverage check: `mfb build -v` on any fixture shows "Stack slot promotion: N"
  with N > 0.
- Runtime proof: `examples/brogue/check/check-terrain.sh 8 1`;
  `tools/brogue-perf/bench.sh --runs 7` under the heavy lock.
- Arena proof: `rt_inplace_self_update --ignored` narrowed with
  `MFB_SELF_UPDATE_FILTER` — the self-update arms hold collection pointers in
  exactly the slots this moves (`.ai/testing-gates.md`).
- Cross-target (every native backend): the artifact gate for bytes; for runtime,
  `scripts/linux-runtime-proof.sh` on 2223 (`linux-aarch64`) and 2229
  (`linux-riscv64`), a filtered run on 2227 (`linux-x86_64`, emulated), and
  brogue `dig` against the oracle on every target including the real Windows box
  (`windows-x86_64`). What could not run is stated in Corrections.
- Final gate (once, heavy lock): `cargo test --release --no-fail-fast` and
  `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Promote FP-class slots too? — **No** for now (recommended): brogue has none
  and the float path already keeps values in `d` registers (plan-01).
- Shrink the frame by dropping promoted objects? — **No** (recommended): it
  changes nothing measurable and would renumber every other object.

## Corrections

1. The design promoted a slot even when some path reads it before any store
   (the register would then hold whatever it held, where the frame word held
   whatever *it* held). Both are garbage a well-formed stream never uses, but
   "never uses" is a claim about every lowering, not something this row proves.
   Phase 2 adds `read_before_written`: backward liveness over the allocator's
   CFG with loads as uses and stores as definitions; a candidate live into the
   entry block stays in memory, so even such a read is exactly what it was.
   brogue's count is unchanged by it (29,453 both ways).
2. The code for this plan was developed in a scratch detached worktree
   (`/tmp/b740-dev`) while plan-161-A's lock-bound gate waited in the heavy-lock
   queue, and was cherry-picked onto `worktree-B-740` only after plan-161-A
   completed — the Prerequisites row was checked at landing, not at drafting.
   Cherry-picked onto `worktree-B-740` on 2026-09-27 (`8b36c06e3`, `d34fdae20`,
   `71c76232a`, renumbered from plan-158 in `c328fe608`).
3. **Gates for A, B and C run once, together, at the branch tip.** The three
   letters' code landed as separate itemized commits, but only the tip is ever
   merged to main, so an intermediate commit's golden state is unobservable and
   regenerating goldens three times (each a release build + ~20 min gate under
   the shared heavy lock) would prove nothing more. What stays per mechanism:
   each letter's own tests, brogue vs oracle at each step (stepA, stepAB,
   stepABC: 320/320 each), the timing of each step (measured together under the
   lock), and a separate budget-tightening commit per step.
4. The riscv64 cross-target run first failed 0/80 with B+C: a pre-existing
   allocator bug (eviction around the fused `rv.br` restored the victim only on
   the fall-through edge), filed as bug-746 and fixed on main by the riscv64
   lane (`efd343f3a`). After merging main: riscv64 80/80.

## Summary

The risk is entirely in proving a frame word unreachable except through the
instructions being rewritten; the design fails closed on anything it does not
recognize. The payoff is the one measured: 17.6x → ~7.5x on brogue with no
change to what executes.
