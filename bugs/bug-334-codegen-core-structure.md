# bug-334: structural problems in the root of the code-generation layer (glob-flattened namespace, `lower_runtime_helper` boilerplate, grab-bag modules, misfiled tests)

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup)

Status: Open
Regression Test: none new — the invariant is byte-identical output
(`scripts/artifact-gate.sh`), plus the existing `src/target/shared/code/tests.rs`
and the architecture lint in `mir.rs`.

The root of `src/target/shared/code/` — `mod.rs` plus the handful of helper
modules it glob-imports — has accumulated four independent structural problems
that make the 95,446-line codegen subtree harder to change than its size alone
would explain. None of them affects a compiled program: every item here is a
move, a rename, a deduplication, or a comment fix, and **the single correct
outcome is a subtree with the same emitted bytes and a namespace the compiler
can check.**

The framing finding is (B): `mod.rs` glob-imports 18 sibling modules while each
of those opens with `use super::*;`, so the entire subtree shares one flat
namespace. That is not a style complaint — it is the demonstrated root cause of a
verified duplicate. `type_utils::align` and `types::data_align` are the same
function over the same domain, and because both are reachable unqualified from
every file, neither author saw the other. Nothing in the toolchain can report
that today.

References:

- Found during the cleanup review (worktree `.claude/worktrees/cleanup-review`,
  base `25c38ba1`), agent 04 "codegen core"; corroborated independently by agents
  01, 05, and 06 from the collection, regalloc, and fs/io sides.
- `scripts/artifact-gate.sh` — the execution-free byte-identity gate for
  codegen/IR/lowering changes (memory note "Fast codegen gate").
- `scripts/test-accept.sh` — the full acceptance/golden harness.
- Cross-references:
  - **bug-331** (fs/io runtime-helper lowering) — reached the same conclusion
    about `lower_runtime_helper` from the fs/io arms. Item A1 here is the
    whole-function form of that finding; land them together, not twice.
  - **bug-327** (file splits in the codegen subtree) — the concrete split
    proposals for `mod.rs`, `builder_codegen_primitives.rs`,
    `entry_and_arena.rs`, `error_constants.rs`, and the `builder_collection_*`
    pair live there. This document names *what is misfiled* and does not restate
    the split geometry.
  - **bug-322** (arena-alloc boilerplate) — the sibling "same literal written N
    times" cleanup, same class as A1.

## Current State

Measured against `25c38ba1` in the cleanup-review worktree. Every number below
was counted, not estimated.

| Measurement | Value |
| --- | --- |
| `src/target/shared/code/` subtree | 95,446 lines |
| `mod.rs` | 3,548 lines; **no module doc**; `mod` block at 3061–3156 |
| `lower_runtime_helper` (`mod.rs:1333`–`2558`) | 1,226 lines |
| — `CodeFunction { … }` literals inside it | **44** |
| — of those, `params:` built from `spec.abi.params` | **38** (19 lines each) |
| — of those, `params: Vec::new()` | **6** (10 lines each) |
| — total lines that are the repeated literal | **782 of 1,226 (64%)** |
| — identical `does not emit runtime call` error arms | 3 (`:2460`, `:2530`, `:2555`) |
| `mod.rs` glob imports of sibling modules (`use x::*;`) | **18** |
| — of those 18 modules that open `use super::*;` | **18** |
| — of those 18 modules with a `//!` module doc | 3 |
| Files in the subtree containing `use super::*;` | 73 |
| `builder_*.rs` files / lines / with any `#[cfg(test)]` | 29 / 37,513 / **0** |
| `error_constants.rs` | 841 lines, **17** banner sections |
| — collection ABI constants in it (`:752`–`:818`) | **41** |
| `validation.rs` | 554 lines, of which ~157 are validation |
| `builder_emit_helpers.rs` thread special case (`:360`–`:508`) | 149 of 525 lines (28%) |
| `#[cfg(test)]` code at the bottom of `mod.rs` (`:3409`–`:3548`) | 140 lines |
| `mir.rs` | 1,797 lines: 794 code + 1,003 test (`mod tests` at `:795`) |

The two claims with real correctness stakes were both resolved before writing
this up:

1. **`params: Vec::new()` is NOT latent drift — it is safe to collapse.** The 6
   `Vec::new()` arms cover 9 runtime calls: `io.flush`, `io.isBuffered`,
   `io.readChar`, `io.readByte`, `io.isInputTerminal`, `io.isOutputTerminal`,
   `io.isErrorTerminal`, `fs.currentDirectory`, `fs.tempDirectory`. Every one of
   those declares `params: &[]` in its spec —
   `runtime/io_specs.rs:72,94,138,149,171,182,193` and
   `runtime/fs_specs.rs:146,157`. So `spec.abi.params.iter().map(…).collect()`
   yields exactly `Vec::new()` for all 9. The distinction is cosmetic, and
   collapsing it cannot change output. (The "9 arms" in the review notes was a
   count of *call names*; there are 6 match arms.)
2. **`align` and `data_align` are behaviorally identical over the whole
   domain.** `type_utils.rs:221` guards `alignment == 0`; `types.rs:172` guards
   `alignment <= 1`. For `alignment >= 1` both compute
   `value.div_ceil(alignment) * alignment`; for `alignment == 0` and
   `alignment == 1` both return `value` unchanged — `types.rs:742-743` already
   asserts `data_align(1, 0) == 1` and `data_align(0, 0) == 0`. There is no input
   on which they differ.

Belt-and-braces on (1): `CodeFunction.params` is read in exactly two places —
`code_impl.rs:215` (the `-code` JSON dump) and `mir.rs:761` (the `-mir` dump). It
does not reach any encoder. So even a genuine mismatch would have been
dump-only; there is none.

## Root Cause

`src/target/shared/code/mod.rs` grew as the single entry point of the codegen
layer and never got a namespace discipline. Three mechanisms compound:

- **The glob pair.** `mod.rs:3065`–`3151` re-exports 18 modules with
  `use <m>::*;`, and all 18 open with `use super::*;`. Every item in the subtree
  is therefore visible unqualified from every other file, with no import line
  recording the dependency. `rustc` cannot flag a duplicate definition in two
  different modules, an unused cross-module item, or a helper that "belongs"
  somewhere else. `align`/`data_align` is the proof it already failed.
- **Declaration position.** Because the `mod`/`use` block sits at
  `mod.rs:3061`, 3,060 lines after the first line of code, the file reads as if
  it has no dependencies at all. The same inversion recurs at
  `regalloc/mod.rs:384` (of 405), `net/mod.rs:865` (of 869),
  `macos_aarch64/app/mod.rs:558` (of 792), and `linux_gtk/mod.rs:786` (of 996).
- **Residual modules.** Several helper modules were carved out by "what was in
  the way" rather than by a concern, and the glob namespace hid the cost: a
  reader never has to know which file a symbol came from, so no one is pushed to
  make the boundary make sense. 15 of the 18 glob-imported modules have no
  module doc at all.

`lower_runtime_helper` (A) is an independent accretion: each new runtime helper
was added by copying the previous arm, including its 19-line `CodeFunction`
construction, rather than by hoisting it — even though the `net.*`, `tls.*`, and
`audio.*` arms (`mod.rs:2400`, `:2500`, `:2478`) already demonstrate the correct
shape, an inner `match` that produces only the tuple.

## Items

### (A) The `lower_runtime_helper` boilerplate

**A1 — a 19-line `CodeFunction` literal is written 44 times, 64% of the
function.** `mod.rs:1333`–`2558`. The literal appears at `:1370, 1393, 1417,
1440, 1469, 1521, 1555, 1569, 1583, 1606, 1652, 1675, 1689, 1723, 1737, 1765,
1792, 1816, 1839, 1862, 1886, 1909, 1932, 1955, 1978, 2001, 2023, 2046, 2069,
2092, 2115, 2139, 2163, 2191, 2214, 2237, 2260, 2283, 2306, 2329, 2382, 2464,
2487, 2534`. Every copy has the identical `name`, `symbol`, `returns`, `frame`,
`stack_slots`, `instructions`, `relocations` fields; only `params` varies (38 vs
6, see A2). The `net.*` arm (`:2400`–`:2413`) already shows the fix: its inner
`match` yields `(frame, instructions, relocations, stack_slots)` and one
`CodeFunction` is built after it. Applying that shape to the whole function
removes ~740 lines.

*Cross-reference bug-331: that document proposes the same collapse from the
fs/io side. One implementation, not two.*

**A2 — 6 arms hardcode `params: Vec::new()`, an undocumented inconsistency the
boilerplate hides.** `mod.rs:1558, 1572, 1678, 1692, 1726, 1795`. Verified
equivalent (see Current State ¶1): all 9 covered calls declare `params: &[]`.
Record that verdict in the commit message when A1 lands, so the next reader does
not have to re-derive it.

**A3 — the "unknown call" error is written three times.** `mod.rs:2460`
(inside the `net.*` inner match), `:2530` (inside the `tls.*` inner match), and
`:2555` (the outer fallthrough) all format
`"native code plan does not emit runtime call '{other}'"`. The two inner copies
are unreachable in practice — an unrecognized `net.*`/`tls.*` symbol would not
have produced a spec at `:1341` — so they are defensive duplicates of the outer
arm. Keep one; if the inner ones stay as assertions, say so in a comment.

### (B) Namespace and module structure

**B1 — the glob pair flattens the whole subtree.** `mod.rs:3065, 3067, 3073,
3076, 3080, 3082, 3084, 3086, 3088, 3090, 3092, 3094, 3096, 3098, 3100, 3147,
3149, 3151` glob-import `error_constants, types, entry_and_arena, codegen_utils,
fs_helpers, fs_helpers_paths, fs_helpers_io, fs_helpers_atomic, float_format,
io_helpers, stdin_broadcast, runtime_helpers, runtime_helpers_thread,
data_objects, module_analysis, type_utils, serialization_utils,
function_lowering`. All 18 open with `use super::*;`. **This is the enabling
item** — see Fix Design.

**B2 — the verified duplicate B1 caused.** `type_utils.rs:221` (`align`) and
`types.rs:172` (`data_align`). Identical over the whole domain (Current State
¶2). `data_align` has 4 call sites, all inside `types.rs`
(`:149, 152, 161, 166`) plus its own tests at `:742`–`:746`; `align` is called
from `codegen_utils.rs` (5×), `mod.rs` (4×), `data_objects.rs:736`, and
`link_thunk.rs` (3×). Delete `data_align`, point `types.rs` at `align`, and move
the `data_align` tests onto `align` (they cover the `alignment == 0` case the
existing callers never hit).

**B3 — `mod` declarations sit at the bottom of five files.** `mod.rs:3061`–`3156`
of 3,548 lines, in two partially-alphabetized runs (`:3061`–`:3100` interleaves
`mod`/`use` pairs; `:3101`–`:3145` is a sorted run; `:3146`–`:3156` is a third
unsorted tail) — bug-327 tracks the `mod.rs` split; the declaration block should
move to the top regardless of whether the split lands. Same inversion:
`regalloc/mod.rs:384-385,405`; `net/mod.rs:865-866` (of 869);
`macos_aarch64/app/mod.rs:558-560` (of 792); `linux_gtk/mod.rs:786-788` (of
996). `regalloc/mod.rs` is otherwise the model to copy — it uses explicit
`use super::types::CodeInstruction` (`:21`), not a glob.

**B4 — four grab-bag helper modules with unprincipled boundaries.**
  - `codegen_utils.rs` (765 lines, no module doc): two disjoint halves —
    `:8`–`:350` lowers two standalone runtime helpers (`sort_string_list`,
    `validate_utf8`); `:352`–`:765` is frame finalization and
    prologue/epilogue machinery (`finalize_frame`, `finalize_vreg_helper`,
    callee-save/restore, stack-arg sentinel resolution). Nothing connects them.
  - `types.rs` (751 lines, no module doc): the plan data model
    (`:3`–`:113`), a data-blob layout algorithm (`:115`–`:204`,
    `layout_data_objects` + hex decode + `data_align`), a **65-method**
    `CodegenPlatform` god-trait (`:206`–`:624`), and the entry specs
    (`:626`–`:693`). The trait alone is 419 lines, 56% of the file.
  - `code_impl.rs` (333 lines): named for nothing. It is `impl CodeInstruction`
    (`:3`–`:183`) plus the entire `ToCodeJson` serializer for 7 types
    (`:185`–`:333`).
  - `type_utils.rs` (369 lines): coherent — type-string parsing and static
    constant folding — but one character from `types.rs`, which holds *no*
    type-string logic. The two names are mutually misleading.
  - `serialization_utils.rs`: **17 lines** (two functions, `join_json` and
    `json_string_list`) costing two lines in the declaration block
    (`mod.rs:3148-3149`). It belongs in `code_impl.rs` next to `ToCodeJson`,
    which is its only consumer trait.

**B5 — `builder_value_semantics.rs` (890 lines) names none of its concerns.**
Resource `STATE` initialization (`:10`), default-value lowering (`:38`), field
access (`:160`), `WITH` update (`:286`), string concat (`:366`), global
access (`:502`, `:509`), static constant folding and typing (`:520, 563, 624,
650`), thread runtime return type (`:750`), match compare (`:790`), result
payload classification (`:860`). "Value semantics" describes at most the
resource/default pair. (Agent 01 reached the same verdict independently.)

**B6 — `validation.rs` is 28% validation.** 554 lines: `NativeCodePlan::validate`
(`:3`–`:63`) and `CodeFunction::validate` (`:96`–`:188`) are the ~157 validation
lines; `:65`–`:94` is a stray `to_json` (belongs with the other serializers in
`code_impl.rs`); `:189`–`:461` is a 273-line `TypeModel` builder carrying the
canonical union-tag doc; `:462`–`:478` is `CollectionTypeLayout::from_type`;
`:480`–`:554` is a test module scoped to the union tags only.

**B7 — 41 collection ABI constants live in `error_constants.rs`.**
`error_constants.rs:752`–`:818`: the List/Map record layout, entry layout, map
hash index (`MAP_*`, `FNV1A_*`), geometric growth shape, and the 11
`COLLECTION_TYPE_*` element codes. The file is a 17-section grab-bag whose name
covers exactly one section (`:10`, "Result / Error calling protocol") — the
others are the error catalog, entry frame, closures, term state, arena layout,
stdin log, PCG64, SIMD, filesystem modes, resource records, collections,
Unicode tables, and threads. bug-327 carries the split; the collections section
is the cleanest single extraction. (Agent 01 filed the same item.)

**B8 — a 149-line thread-transfer special case is 28% of the generic
`builder_emit_helpers.rs`.** `emit_thread_send_runtime_helper_call` at
`builder_emit_helpers.rs:360`–`:508`, called once from `:283`. The rest of the
file is the generic runtime-call emission path plus string-address loading. The
thread case belongs with the other thread-transfer lowering. The file has no
module doc.

**B9 — `Vregs` is buried 1,996 lines into `entry_and_arena.rs`.**
`entry_and_arena.rs:1996`–`:2012` defines the `%vN` name generator used by
`codegen_utils.rs`, `float_format.rs`, `fs_helpers_io.rs`,
`fs_helpers_atomic.rs`, `fs_helpers_paths.rs`, `link_thunk.rs`, and `os.rs` —
seven sibling modules, none of which imports it explicitly. They reach it only
through `use entry_and_arena::*;` at `mod.rs:3073`. It is a 17-line utility
living inside a 2,379-line file about program entry and the arena.

**B10 — `runtime_helpers.rs` contains only thread code, so `_thread` on its
sibling distinguishes nothing.** `runtime_helpers.rs` (1,054 lines) is the
thread block layout constants (`:3`–`:60`), `thread_symbol` (`:62`),
`emit_thread_external_call` (`:70`), `emit_thread_queue_alloc` (`:84`),
`lower_thread_helper` (`:218`), `lower_thread_start_helper` (`:375`), and
`lower_thread_trampoline` (`:715`). Nothing else. Its sibling
`runtime_helpers_thread.rs` (1,457 lines) is also entirely thread code. The two
leak into each other in both directions: `lower_thread_helper`
(`runtime_helpers.rs:218`) dispatches to `simple_thread_handle_helper`,
`thread_queue_write_helper`, and `thread_queue_read_helper` in the `_thread`
file, while that file calls back into `emit_thread_external_call`
(`runtime_helpers_thread.rs:29, 99, 113, 155, 175`). Neither file imports the
other — both resolve through `use super::*;` and the mod.rs globs. Under B1 this
becomes visible immediately; the natural shape is `thread/mod.rs` +
`thread/ops.rs`.

### (C) Smaller misplacements and stale comments

**C1 — the `mfb.string.v1` data object is hand-built 4 times.** `mod.rs:501-509,
511-518, 594-604, 1234-1245`. Each repeats the same six fields, including the
verbatim layout string
`"mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"` and the same
`align: 8` / `size: align(8 + len + 1, 8)` computation (the `EMPTY_STRING_SYMBOL`
copy at `:511` hardcodes `size: 16`). `data_objects.rs:724` already has the
sibling `raw_data_object` helper; add `string_data_object` next to it.

**C2 — `regalloc/mod.rs`'s module doc claims only one strategy ships, and
misnames the flag.** `regalloc/mod.rs:11-15`: "The strategy is selected by the
`-regalloc <name>` build flag (§4.2). Stage A ships exactly one strategy,
[`BumpAndReset`]". Both halves are wrong: `:56`–`:70` defines two variants and
`:109`–`:113` defaults to `LinearScan` (`BumpAndReset`'s own doc at `:60`–`:66`
says it "has no spilling… miscompiles" and "Never default to it"). The flag is
`--regalloc` post plan-42, with `-regalloc` kept as an alias
(`cli/build.rs:198, 204-205`); the doc-comment at `cli/build.rs:98` has the same
stale single-dash spelling.

**C3 — a comment says the `term::` helpers await "Phase 5"; Phase 5 landed on
every app platform.** `mod.rs:1352-1354`: "the remaining `term::` helpers keep
the shared console backend until Phase 5 wires their app bodies." All app
backends implement `emit_app_term_helper` today —
`macos_aarch64/app/app_io.rs:599` (whose own doc says "plan-01-term.md §6.3,
Phase 5") and `linux_gtk/app_io.rs:9`, reached from
`macos_aarch64/code.rs:183`, `linux_aarch64/code.rs:237`, and
`linux_x86_64/code.rs:261`. Both dispatchers cover `on/off/clear/sync/moveTo/
setForeground/setBackground/setBold/setUnderline/terminalSize/showCursor/
hideCursor`. Drop the clause.

**C4 — an orphaned tombstone comment.** `mod.rs:1296-1299`: a four-line comment
about the rv64 `v128` slot region, followed by a blank line and attached to no
item ("no process-global data object is emitted"). It documents an absence at
the site of the absence. Move it to where the per-thread `v128` slot region *is*
defined, or delete it.

### (D) Test organization

**D1 — a 140-line `#[cfg(test)]` arena simulator sits at the bottom of `mod.rs`
in production position.** `mod.rs:3409`–`:3444` (`checked_arena_used_after_alloc`)
and `:3446`–`:3548` (`FreeListSim`, an executable reference model of the
coalescing free list). Both are `#[cfg(test)]` and both are used **only** by
`tests.rs` (`:7, 26, 48, 63, 76, 92, 96, 104, 112, 116, 124, 128`). They are
4% of `mod.rs` and belong in `tests.rs` — which is 131 lines and contains
nothing but the assertions that drive them.

**D2 — a whole-subtree architecture lint hides inside `mir.rs`'s round-trip test
module.** `mir.rs:1656`–`:1772`, `shared_lowering_names_no_physical_register`,
inside `mod tests` (`mir.rs:795`). It walks every `.rs` file under
`src/target/shared/`, scans for ~180 forbidden physical-register spellings across
three ISAs, and enforces the plan-34-D invariant that makes the bug-56 class
unrepresentable. That is the most load-bearing test in the subtree and it is
findable only by reading a MIR serialization test module. `mir.rs` is 794 lines
of code and 1,003 lines of test.

Its exemption heuristic is also loose: `mir.rs:1738`,
`if name.contains("test") || name == "abi.rs"`. Today that exempts `tests.rs`
and `test_support.rs` as intended, but it is a substring match on the whole
filename — any future file whose name happens to contain `test` (`latest.rs`,
`fastest.rs`, `builder_test_helpers.rs`, a `tests/` subdirectory file) is
silently and permanently exempted from the invariant, with no diagnostic. Fix
the heuristic to an explicit allowlist (`tests.rs`, `test_support.rs`) at the
same time as the move.

**D3 — zero unit tests across the 37,513-line `builder_*.rs` surface.** 29
`builder_*.rs` files; not one contains `#[cfg(test)]`. A purpose-built
`TestPlatform` exists at `test_support.rs:21` (a 65-method `CodegenPlatform`
stub) and is already used by `net/io.rs`, `tls/openssl.rs`, `tls/macos.rs`,
`crypto_ec/openssl.rs`, and `crypto_ec/macos.rs` — but by no builder. Every
builder change is therefore validated only end-to-end through
`scripts/test-accept.sh`. This is the item to fix opportunistically: establish
the pattern on the first builder file that gets split under bug-327, rather than
as a standalone campaign.

## Goal

- `cargo build` and `scripts/artifact-gate.sh` produce **byte-identical**
  binaries and byte-identical `-code`/`-mir` dumps before and after every phase.
- `mod.rs` glob-imports fewer sibling modules; each converted module's
  dependencies are recorded in explicit `use` lists the compiler checks.
- `type_utils::align` is the only alignment helper in the subtree.
- `lower_runtime_helper` constructs `CodeFunction` in one place.
- The architecture lint in `mir.rs` lives in a file whose name says what it is,
  with an exact-match exemption list.
- No `#[cfg(test)]` production-position code remains in `mod.rs`.

### Non-goals (must NOT change)

- **Any emitted byte.** Every item is a move/rename/dedup/comment. Any diff in
  `scripts/artifact-gate.sh` output means the change is wrong, not that the
  golden needs regenerating.
- The `-code` and `-mir` JSON dump shapes, including the `params` array on every
  runtime helper (A2 is verified output-neutral; if a dump diff appears, stop —
  the equivalence assumption was violated).
- The `.mfp` wire format, the `CodeFunction`/`CodeInstruction` data model, and
  the `CodegenPlatform` trait surface (B4 proposes moving the trait, not
  changing it).
- The plan-34-D invariant itself. D2 moves and *tightens* the lint; it must not
  weaken any exemption. Explicitly forbidden: broadening the exemption list to
  make a moved file pass.
- The file-split geometry owned by bug-327. Do not re-derive it here.

## Blast Radius

- `src/target/shared/code/mod.rs` — A1, A2, A3, B1, B3, C1, C3, C4, D1. Fixed by
  this bug.
- `src/target/shared/code/{types,type_utils}.rs` — B2, B4. Fixed by this bug.
- `src/target/shared/code/{codegen_utils,code_impl,serialization_utils,
  builder_value_semantics,validation,error_constants,builder_emit_helpers,
  entry_and_arena,runtime_helpers,runtime_helpers_thread}.rs` — B4–B10. Moves
  only; each is behavior-preserving in isolation.
- `src/target/shared/code/mir.rs`, `tests.rs` — D1, D2.
- `src/target/shared/code/regalloc/mod.rs`, `src/cli/build.rs:98` — C2 (comments
  only).
- `src/target/shared/code/net/mod.rs`, `src/target/macos_aarch64/app/mod.rs`,
  `src/target/linux_gtk/mod.rs` — B3, same declaration-position inversion.
  In scope for the one-line move; their contents are not.
- The other 73 files containing `use super::*;` — **latent, same hazard, out of
  scope.** They are the far side of B1. Converting them all is a much larger diff
  than converting `mod.rs`'s 18 exports, and the `mod.rs` side is where the
  duplicate-definition risk concentrates (that is the namespace everything joins).
  Do them opportunistically as bug-327 splits land.
- `src/target/shared/runtime/{io,fs}_specs.rs` — **unaffected.** Read during
  verification of A2; no change needed (all 9 declare `params: &[]`, which is
  correct).
- The three arch backends (`src/arch/*`) — **unaffected.** Nothing here crosses
  the `CodeFunction`/`MirPlan` boundary.

## Fix Design

**The glob-to-explicit conversion (B1) is the enabling change and should go
first — but incrementally.** Once a module's consumers are recorded in explicit
`use` lists, every subsequent move in (B) and (C) becomes mechanically checkable
by the compiler: moving a function out of `codegen_utils.rs` produces a build
error at each consumer instead of silently resolving through the glob, and a
second `align` can no longer be written without a name collision. Doing the
moves first, on a flat namespace, means the compiler validates nothing and the
reviewer carries the whole burden.

It is also the largest diff in this document, so **convert module-by-module,
smallest first**, one commit each, `scripts/artifact-gate.sh` after each:

1. `type_utils` (369 lines) — do this one first *because* B2 lands with it: with
   `use type_utils::align;` explicit, deleting `types::data_align` is a
   one-symbol change the compiler verifies.
2. `serialization_utils` (17 lines) — or skip the conversion and merge it into
   `code_impl.rs` outright (B4), which removes the glob by removing the module.
3. `module_analysis`, `data_objects` — small, few consumers.
4. `code_impl` (already partially explicit: `mod.rs:3078` imports only
   `ToCodeJson`).
5. Then the larger ones (`codegen_utils`, `entry_and_arena`, `fs_helpers*`,
   `runtime_helpers*`, `types`) as their owning items in (B)/(C) come up.

**Keep `error_constants` as a glob.** It is 841 lines of nothing but `pub(crate)
const`, consumed by essentially every file in the subtree; an explicit list would
be hundreds of lines of import noise per consumer and would obscure rather than
reveal the dependency structure. A constants module is the one legitimate use of
`use x::*;` here. Say so in a comment at `mod.rs:3064` so the exception is
deliberate rather than residual. The same reasoning may apply to `types` (the
plan data model) — decide when its turn comes; see Open Decisions.

**A1 is independent of B and can land in parallel.** Rewrite
`lower_runtime_helper` so every arm produces
`(frame, instructions, relocations, stack_slots)` and one `CodeFunction` is
constructed after the `match`, exactly as the `net.*` arm does today. The two
early `return Ok(CodeFunction { … })` arms at `:1370` (term) and `:1393`
(crypto_ec) fold into the same shape. Coordinate with **bug-331**: whichever
lands first should carry the whole collapse, and the other should be reduced to
a cross-reference.

**Rejected alternatives:**

- *A macro for the `CodeFunction` literal.* Hides the same duplication behind a
  new abstraction, keeps 44 expansion sites, and makes the A2 inconsistency
  harder to see rather than resolving it. The `net.*` arm already proves a plain
  restructure works.
- *Converting all 73 `use super::*;` files at once.* An unreviewable diff across
  95K lines with no incremental verification point. The value is in the `mod.rs`
  boundary; the leaf files can follow.
- *Preserving `params: Vec::new()` "just in case".* Verified equivalent for all
  9 calls; preserving it would preserve a distinction with no meaning and
  guarantee the next reader repeats this investigation.
- *Deleting `type_utils::align` instead of `types::data_align`.* `align` has 13
  call sites across 4 files vs `data_align`'s 4, all inside one file. Keep the
  one with reach.
- *Weakening or dropping the `mir.rs` lint during the D2 move.* Explicitly
  forbidden — it is the guard for bug-56.

## Phases

Each phase ends with `scripts/artifact-gate.sh` green (byte-identical) before
the next begins. Any byte diff halts the phase.

### Phase 1 — baseline + the enabling conversion (no behavior change)

- [ ] Capture the `scripts/artifact-gate.sh` baseline artifacts on `25c38ba1`.
- [ ] Convert `type_utils` from glob to an explicit `use` list at `mod.rs:3147`;
      land B2 in the same commit (delete `types::data_align`, repoint
      `types.rs:149,152,161,166` at `align`, move the `data_align` tests from
      `types.rs:742-746` onto `align`).
- [ ] Convert `serialization_utils` by merging it into `code_impl.rs` (B4) and
      deleting `mod.rs:3148-3149`.
- [ ] Convert `module_analysis` and `data_objects` to explicit lists.
- [ ] Add the deliberate-exception comment at `mod.rs:3064` for
      `error_constants`.

Acceptance: artifact gate byte-identical; exactly one `align` in the subtree;
`cargo build` clean with no new `unused_imports`.
Commit: —

### Phase 2 — `lower_runtime_helper` (A1–A3)

- [ ] Restructure `mod.rs:1333-2558` to build one `CodeFunction` after the
      `match`, per the `net.*` arm's shape.
- [ ] Collapse the 6 `params: Vec::new()` arms into the shared construction;
      record the A2 equivalence verdict in the commit message.
- [ ] Reduce the three duplicate error arms (`:2460, 2530, 2555`) to one, or
      comment the inner two as assertions.
- [ ] Reconcile with bug-331 — one implementation, the other cross-referenced.

Acceptance: artifact gate byte-identical; `-code` dumps for a program using
`io.flush`, `fs.tempDirectory`, `net.read`, and `thread.send` are unchanged
including their `params` arrays; `lower_runtime_helper` is under ~500 lines.
Commit: —

### Phase 3 — declaration position, module moves, and comments (B3–B10, C1–C4)

- [ ] Move the `mod`/`use` block to the top of `mod.rs`, `regalloc/mod.rs`,
      `net/mod.rs`, `macos_aarch64/app/mod.rs`, `linux_gtk/mod.rs`; merge the
      three unsorted runs in `mod.rs` into one alphabetized block.
- [ ] Move `Vregs` (B9) out of `entry_and_arena.rs:1996-2012` to
      `codegen_utils.rs`, converting its 7 consumers to explicit imports.
- [ ] Move `emit_thread_send_runtime_helper_call` (B8) out of
      `builder_emit_helpers.rs:360-508`.
- [ ] Move `validation.rs:65-94` (`to_json`) to `code_impl.rs`; extract the
      `TypeModel` builder (`:189-461`) per bug-327 (B6).
- [ ] Rename `runtime_helpers.rs`/`runtime_helpers_thread.rs` to a `thread/`
      module (B10), with explicit cross-imports replacing the glob leakage.
- [ ] Add `string_data_object` next to `data_objects.rs:724` and route the 4
      `mfb.string.v1` sites through it (C1).
- [ ] Comment fixes: `regalloc/mod.rs:11-15` + `cli/build.rs:98` (C2),
      `mod.rs:1352-1354` (C3), `mod.rs:1296-1299` (C4).
- [ ] Add module docs to the 15 glob-imported modules that lack one.

Acceptance: artifact gate byte-identical after each move; every moved item is
reached by an explicit import.
Commit: —

### Phase 4 — test organization (D1, D2) + full validation

- [ ] Move `mod.rs:3409-3548` (`checked_arena_used_after_alloc`, `FreeListSim`)
      into `tests.rs` (D1).
- [ ] Move `mir.rs:1656-1772` into a new `architecture_guards.rs`; replace the
      `name.contains("test")` heuristic (`mir.rs:1738`) with an exact-match list
      of `tests.rs` and `test_support.rs`, and confirm the lint still passes
      with the tighter exemption (D2).
- [ ] Run `scripts/test-accept.sh` in full.
- [ ] Confirm the D3 pattern (a `TestPlatform`-based unit test) is agreed for the
      first builder file bug-327 splits; do not add one here.

Acceptance: `scripts/artifact-gate.sh` byte-identical against the Phase 1
baseline; `scripts/test-accept.sh` fully green; the architecture lint passes with
the tightened exemption and is findable by filename.
Commit: —

## Validation Plan

- Regression test(s): none new. The invariant is byte-identity, enforced by
  `scripts/artifact-gate.sh` after every commit. D2's tightened exemption is
  itself the regression guard for the lint's own scope.
- Runtime proof: `scripts/test-accept.sh` in full at the end of Phase 4, plus
  spot `-code` and `-mir` dumps for programs exercising the `io.*`, `fs.*`,
  `net.*`, `tls.*`, `audio.*`, `term.*`, and `thread.*` runtime-helper families —
  these cover all 44 A1 construction sites and both A2 spellings.
- Doc sync: `regalloc/mod.rs:11-15` and `cli/build.rs:98` (C2); the
  `mod.rs:1352-1354` term comment (C3); module docs for the 15 modules that
  lack one. No spec file states any of these structural facts, so no
  `src/docs/spec/**` change is expected — confirm with a grep for
  `codegen_utils`/`code_impl`/`type_utils` before closing.
- Full suite: `scripts/artifact-gate.sh` then `scripts/test-accept.sh`.

## Open Decisions

- **Does `types` stay a glob?** It exports the plan data model (`CodeFunction`,
  `CodeInstruction`, `CodeRelocation`, …) used almost everywhere, like
  `error_constants`. Recommended: convert it anyway — unlike `error_constants`
  its items are types that appear in signatures, so the import lines carry real
  information — but defer the decision until Phase 3, after the small conversions
  show what the import noise actually costs. (§Fix Design)
- **Where does `CodegenPlatform` live?** `types.rs:206-624` is 56% of the file
  and is a trait, not a type. Recommended: its own `platform.rs`. Alternative:
  leave it and rename `types.rs` to say it holds the trait. Coordinate with
  bug-327. (§B4)
- **Do the inner `net.*`/`tls.*` error arms survive A3?** Recommended: delete
  both (unreachable given the `:1341` spec lookup). Alternative: keep as
  `debug_assert`-style comments. (§A3)
- **Who lands the `lower_runtime_helper` collapse — this bug or bug-331?**
  Recommended: this bug (whole-function scope), with bug-331 reduced to a
  cross-reference on its fs/io arms. (§Fix Design)

## Summary

Four independent structural problems in the root of the codegen layer, none of
which changes a compiled byte. The engineering risk is concentrated almost
entirely in **not** changing bytes: `lower_runtime_helper` (A) touches 44
construction sites in the hottest lowering function in the tree, and the module
moves (B/C) relocate code that currently resolves through a flat glob namespace,
where a mistaken move fails silently rather than at compile time. That is exactly
why the glob-to-explicit conversion (B1) is sequenced first and per-module —
after it, the compiler checks the rest, which is also the durable fix for the
class of defect that produced `align`/`data_align` in the first place.

Both claims with correctness stakes were resolved before writing: the
`params: Vec::new()` arms are verified output-neutral (all 9 covered calls
declare `params: &[]`, and `CodeFunction.params` reaches only the two JSON
dumps), and `align`/`data_align` are identical over their whole domain. Neither
blocks the dedup.

Left untouched: the file-split geometry (bug-327), the arena-alloc boilerplate
(bug-322), the fs/io helper arms (bug-331), the 73 leaf files using
`use super::*;`, and the plan-34-D physical-register invariant, which this work
moves and tightens but must never weaken.
